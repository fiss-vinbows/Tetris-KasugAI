//! 対戦シミュレーター: 2つの TBP の bot を、ぷよテト2のルールで何局も対戦させて勝率を出す。
//!
//! 使い方(TetrisAI フォルダで):
//!     cargo run --release -p tetrisai-arena -- [オプション]
//!
//! オプション:
//!   --games <数>        対局数(既定 100)
//!   --threads <数>      同時に進める対局数(既定: 論理コア数の半分。1局で bot 2つが1コアずつ使う)
//!   --think-ms <ミリ秒>  1手あたりの思考時間(既定 100)
//!   --max-pieces <数>   1人あたりの手数の上限。超えたら引き分け(既定 500)
//!   --seed <数>         最初の対局のミノ列の種(対局ごとに1ずつ増やす。既定 1)
//!   --a <exe> / --b <exe>              対戦させる bot(既定: A=このAI、B=Cold Clear 2)
//!   --a-config <file> / --b-config <file>  bot に `--config` で渡す設定
//!   --a-weights <file> / --b-weights <file>  プロセスを使わず、この重み(TOML。`default` なら既定値)で
//!                       このAIの探索を直接呼ぶ。思考時間ではなく幅(--width)を固定する
//!   --width <数>        --a-weights / --b-weights の探索の幅(既定 128)
//!   --a-width / --b-width <数>  片方だけ探索の幅を変える(既定は --width)
//!   --a-no-incoming / --b-no-incoming  --a-weights / --b-weights の側に予告を教えない(比較実験用)
//!   --a-no-opponent / --b-no-opponent  --a-weights / --b-weights の側に相手の情報を教えない(比較実験用)
//!   --a-speculate / --b-speculate      --a-weights / --b-weights の側で、見えているミノの先を予測して読む
//!   --a-no-opening / --b-no-opening    --a-weights / --b-weights の側で序盤の作戦(TD系テンプレか素の状態を
//!                       ランダムで選ぶ。設計書 8.10)を使わない
//!   --record <file>     1局ごとの結果を JSON Lines で書き出す
//!   --adjudicate        手数の上限に達したら危なさ(高さ + 予告)の小さいほうを勝ちにする(同じAI同士の比較用)

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};

use serde_json::json;
use tetrisai_arena::agent::{Agent, EngineAgent};
use tetrisai_arena::battle::{Loss, Stats};
use tetrisai_arena::game::{play_game, GameConfig, GameResult};
use tetrisai_arena::tbp::TbpBot;
use tetrisai_core::Rules;
use tetrisai_engine::{OpeningMode, OpeningTracker, Weights};

enum Engine {
    /// TBP の bot プロセス
    Tbp { exe: PathBuf, args: Vec<String> },
    /// このAIの探索を直接呼ぶ
    InProcess {
        weights: Weights,
        label: String,
        width: usize,
        use_incoming: bool,
        use_opponent: bool,
        speculate: bool,
        opening: bool,
        opening_mode: OpeningMode,
        incoming_lag: usize,
    },
}

impl Engine {
    /// `seed` は序盤の作戦を選ぶ乱数の種。
    fn create(&self, rules: &Rules, seed: u64) -> Box<dyn Agent + Send> {
        match self {
            Engine::Tbp { exe, args } => Box::new(TbpBot::spawn(exe, args).expect("bot を起動できない")),
            Engine::InProcess { weights, label, width, use_incoming, use_opponent, speculate, opening, opening_mode, incoming_lag } => {
                let mut agent = EngineAgent::new(weights.clone(), rules.clone(), *width, label);
                agent.incoming_lag = *incoming_lag;
                agent.use_incoming = *use_incoming;
                agent.use_opponent = *use_opponent;
                agent.speculate = *speculate;
                agent.opening = opening.then(|| {
                    OpeningTracker::with_mode(seed, *opening_mode)
                        .with_after_pc(weights.after_pc_seed, weights.after_pc_no_rival)
                        .with_attacked_well(weights.td_attacked_well, weights.td_attacked_clears)
                });
                Box::new(agent)
            }
        }
    }
}

/// 重みのファイル(TOML)を読む。`default` なら既定値。
fn load_weights(path: &str) -> Weights {
    if path == "default" {
        return Weights::default();
    }
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("重みのファイルを読めない: {path}: {e}"));
    toml::from_str(&text).unwrap_or_else(|e| panic!("重みのファイルの形式が違う: {path}: {e}"))
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let get = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let num = |name: &str, default: u64| get(name).map(|s| s.parse().expect("数値を指定する")).unwrap_or(default);

    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let default_a = root.join("target/release/tetrisai-bot.exe");
    let default_b = root.join("../Tetris/external/cold-clear-2/target/release/cold-clear-2.exe");
    let width = num("--width", 128) as usize;
    let engine = |side: &str, default: PathBuf| match get(&format!("--{side}-weights")) {
        Some(path) => {
            let use_incoming = !args.iter().any(|a| *a == format!("--{side}-no-incoming"));
            let stem = Path::new(&path).file_stem().map_or(path.clone(), |s| s.to_string_lossy().into_owned());
            let speculate = args.iter().any(|a| *a == format!("--{side}-speculate"));
            let use_opponent = !args.iter().any(|a| *a == format!("--{side}-no-opponent"));
            let opening = !args.iter().any(|a| *a == format!("--{side}-no-opening"));
            // 序盤の作戦(bot の --opening と同じ値。seed3 なら最初から C4W を目指す)
            let opening_mode = get(&format!("--{side}-opening"))
                .map_or(OpeningMode::default(), |s| OpeningMode::parse(&s).expect("--opening の値が分からない"));
            let mut label = stem;
            if !use_incoming {
                label += "(予告なし)";
            }
            if !use_opponent {
                label += "(相手なし)";
            }
            if speculate {
                label += "(予測)";
            }
            if !opening {
                label += "(序盤なし)";
            }
            // 片方だけ幅を変える(実機の思考時間で届く幅の差が強さにどう響くかを見る。2026-11-08)
            let width = num(&format!("--{side}-width"), width as u64) as usize;
            // 予告に気づくまでの遅れ(自分の手の数。実機の C4W 側の反応の遅れを再現する。2026-11-10)
            let incoming_lag = num(&format!("--{side}-incoming-lag"), 0) as usize;
            if incoming_lag > 0 {
                label += &format!("(予告の遅れ{incoming_lag})");
            }
            Engine::InProcess { weights: load_weights(&path), label, width, use_incoming, use_opponent, speculate, opening, opening_mode, incoming_lag }
        }
        None => {
            let exe = get(&format!("--{side}")).map(PathBuf::from).unwrap_or(default);
            assert!(exe.exists(), "bot の実行ファイルが無い: {}", exe.display());
            let args = get(&format!("--{side}-config")).map(|c| vec!["--config".to_string(), c]).unwrap_or_default();
            Engine::Tbp { exe, args }
        }
    };
    let engines = Arc::new([engine("a", default_a), engine("b", default_b)]);

    let games = num("--games", 100);
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2) as u64;
    let threads = num("--threads", (cores / 2).max(1)).min(games).max(1);
    let first_seed = num("--seed", 1);
    let config = Arc::new(GameConfig {
        rules: Rules::ppt2(),
        think: Duration::from_millis(num("--think-ms", 100)),
        max_pieces: num("--max-pieces", 500) as u32,
        adjudicate: args.iter().any(|a| a == "--adjudicate"),
        block_margin: num("--block-margin", 0) as u32,
        timed: args.iter().any(|a| a == "--timed"),
        time_pct: [num("--a-time-pct", 100) as u32, num("--b-time-pct", 100) as u32],
    });
    let mut record = get("--record").map(|path| File::create(path).expect("記録ファイルを作れない"));

    // 対局を並列に進める。各スレッドは自分の bot 2つを起動し、対局ごとに使い回す
    let next = Arc::new(AtomicU64::new(0));
    let (tx, rx) = mpsc::channel::<GameResult>();
    let mut names = [String::new(), String::new()];
    let mut handles = Vec::new();
    for t in 0..threads {
        let (next, tx, engines, config) = (next.clone(), tx.clone(), engines.clone(), config.clone());
        let mut a = engines[0].create(&config.rules, first_seed * 1000 + t * 2 + 1);
        let mut b = engines[1].create(&config.rules, first_seed * 1000 + t * 2 + 2);
        if t == 0 {
            names = [a.name(), b.name()];
        }
        handles.push(std::thread::spawn(move || loop {
            let i = next.fetch_add(1, Ordering::SeqCst);
            if i >= games {
                break;
            }
            match play_game([a.as_mut(), b.as_mut()], first_seed + i, &config) {
                Ok(r) => tx.send(r).unwrap(),
                Err(e) => {
                    eprintln!("対局 {} で bot とのやり取りに失敗: {e}", first_seed + i);
                    break;
                }
            }
        }));
    }
    drop(tx);

    println!(
        "A = {} / B = {}、{} 局、同時 {} 局、1手 {} ms、手数の上限 {}",
        names[0],
        names[1],
        games,
        threads,
        config.think.as_millis(),
        config.max_pieces
    );
    let started = Instant::now();
    let mut results = Vec::new();
    for r in rx {
        if let Some(f) = record.as_mut() {
            let line = json!({
                "seed": r.seed,
                "winner": r.winner.map(|w| ["A", "B"][w]),
                "losses": r.losses.map(|l| l.map(|l| format!("{l:?}"))),
                "pieces": [r.stats[0].pieces, r.stats[1].pieces],
                "attack": [r.stats[0].attack, r.stats[1].attack],
            });
            writeln!(f, "{line}").ok();
        }
        results.push(r);
        if results.len() % 10 == 0 {
            let (a, b, d) = tally(&results);
            println!("  {} 局: A {a} 勝 / B {b} 勝 / 引き分け {d}({:.0} 秒)", results.len(), started.elapsed().as_secs_f64());
        }
    }
    for h in handles {
        h.join().ok();
    }
    summary(&names, &results);
}

fn tally(results: &[GameResult]) -> (usize, usize, usize) {
    let a = results.iter().filter(|r| r.winner == Some(0)).count();
    let b = results.iter().filter(|r| r.winner == Some(1)).count();
    (a, b, results.len() - a - b)
}

/// 勝率の95%信頼区間(ウィルソンの方法)。
fn wilson(wins: usize, n: usize) -> (f64, f64) {
    if n == 0 {
        return (0.0, 1.0);
    }
    let (w, n, z) = (wins as f64, n as f64, 1.96f64);
    let p = w / n;
    let center = (p + z * z / (2.0 * n)) / (1.0 + z * z / n);
    let half = z * (p * (1.0 - p) / n + z * z / (4.0 * n * n)).sqrt() / (1.0 + z * z / n);
    (center - half, center + half)
}

/// A がテンプレを選んだ局で、最初の2回の消去が TST→TSD だったか(2026-11-08・ユーザー「テンプレは TST→TSD まで打ち切る」)。
fn template_report(results: &[GameResult]) {
    use tetrisai_core::Spin;
    let games: Vec<&GameResult> = results.iter().filter(|r| r.stats[0].template.is_some()).collect();
    if games.is_empty() {
        return;
    }
    let n = games.len() as f64;
    let kind = |r: &GameResult, i: usize| r.stats[0].first_clears.get(i).map(|c| (c.0, c.1));
    let first_tst = games.iter().filter(|r| kind(r, 0) == Some((Spin::Full, 3))).count();
    let tst_tsd = |r: &GameResult| kind(r, 0) == Some((Spin::Full, 3)) && kind(r, 1) == Some((Spin::Full, 2));
    let both = games.iter().filter(|r| tst_tsd(r)).count();
    let pc = games.iter().filter(|r| tst_tsd(r) && r.stats[0].first_clears.iter().skip(1).any(|c| c.2)).count();
    let first_spin = games.iter().filter(|r| r.stats[0].first_clears.first().is_some_and(|c| c.0 == Spin::Full)).count();
    let wins = games.iter().filter(|r| r.winner == Some(0)).count();
    // 消した直後にテンプレが続いていたか
    let alive_after = |r: &GameResult, i: usize| {
        let s = &r.stats[0];
        s.first_clear_at.get(i).and_then(|&m| s.template_alive.get(m as usize)).copied().unwrap_or(false)
    };
    let tst: Vec<&&GameResult> = games.iter().filter(|r| kind(r, 0) == Some((Spin::Full, 3))).collect();
    let tsd: Vec<&&GameResult> = games.iter().filter(|r| tst_tsd(r)).collect();
    let next_kind = |rs: &[&&GameResult], i: usize| {
        let mut m = std::collections::BTreeMap::new();
        for r in rs {
            let k = match r.stats[0].first_clears.get(i) {
                Some(c) => format!("{:?}{}{}", c.0, c.1, if c.2 { "PC" } else { "" }),
                None => "なし".to_string(),
            };
            *m.entry(k).or_insert(0) += 1;
        }
        m
    };
    println!();
    let rate = |f: &dyn Fn(&GameResult) -> bool| {
        let g: Vec<&&GameResult> = games.iter().filter(|r| f(r)).collect();
        let w = g.iter().filter(|r| r.winner == Some(0)).count();
        format!("{} 局・勝率 {:.1}%", g.len(), 100.0 * w as f64 / g.len().max(1) as f64)
    };
    let has_pc = |r: &GameResult| tst_tsd(r) && r.stats[0].first_clears.iter().skip(1).any(|c| c.2);
    println!(
        "届いた所ごとの A の勝率: TST→TSD→パフェ {} / TST→TSD でパフェなし {} / TST だけ {} / TST なし {}",
        rate(&has_pc),
        rate(&|r| tst_tsd(r) && !has_pc(r)),
        rate(&|r| kind(r, 0) == Some((Spin::Full, 3)) && !tst_tsd(r)),
        rate(&|r| kind(r, 0) != Some((Spin::Full, 3)))
    );
    {
        // パフェまで届いた局で、相手がどう耐えたか
        let g: Vec<&&GameResult> = games.iter().filter(|r| has_pc(r)).collect();
        let avg = |f: &dyn Fn(&GameResult) -> f64| g.iter().map(|r| f(r)).sum::<f64>() / g.len().max(1) as f64;
        let pc_at = |r: &GameResult| {
            let s = &r.stats[0];
            s.first_clears.iter().zip(&s.first_clear_at).skip(1).find(|(c, _)| c.2).map_or(0, |(_, &m)| m)
        };
        let fired_before = g.iter().filter(|r| r.stats[1].first_fire_piece > 0 && r.stats[1].first_fire_piece <= pc_at(r)).count();
        println!(
            "パフェまで届いた局 {}: パフェは平均 {:.1} 手目 / A が送った {:.1} 行 / B が相殺した {:.1} 行・せり上がった {:.1} 行 / B が撃ち始めた {:.1} 手目(パフェより前に撃っていた {:.1}%) / B の勝ち {} 局",
            g.len(),
            avg(&|r| pc_at(r) as f64),
            avg(&|r| r.stats[0].sent as f64),
            avg(&|r| r.stats[1].cancelled as f64),
            avg(&|r| r.stats[1].received as f64),
            avg(&|r| r.stats[1].first_fire_piece as f64),
            100.0 * fired_before as f64 / g.len().max(1) as f64,
            g.iter().filter(|r| r.winner == Some(1)).count()
        );
    }
    println!(
        "TST の直後にテンプレが続いていた {:.1}%({} 局中)/ 次の消去 {:?}",
        100.0 * tst.iter().filter(|r| alive_after(r, 0)).count() as f64 / tst.len().max(1) as f64,
        tst.len(),
        next_kind(&tst, 1)
    );
    println!(
        "TSD の直後にテンプレが続いていた {:.1}%({} 局中)/ 次の消去 {:?}",
        100.0 * tsd.iter().filter(|r| alive_after(r, 1)).count() as f64 / tsd.len().max(1) as f64,
        tsd.len(),
        next_kind(&tsd, 2)
    );
    println!(
        "A がテンプレを選んだ局 {}: 最初の消去が Tスピン {:.1}% / TST {:.1}% / TST→TSD {:.1}% / TST→TSD→パフェ {:.1}% / A の勝率 {:.1}%",
        games.len(),
        100.0 * first_spin as f64 / n,
        100.0 * first_tst as f64 / n,
        100.0 * both as f64 / n,
        100.0 * pc as f64 / n,
        100.0 * wins as f64 / n
    );
}

/// C4W の積み込み量・撃った順と勝敗の関係(両者とも撃った局だけ)。
fn well_report(results: &[GameResult]) {
    let both: Vec<&GameResult> =
        results.iter().filter(|r| r.winner.is_some() && r.stats[0].first_fire_piece > 0 && r.stats[1].first_fire_piece > 0).collect();
    let line = |name: &str, games: Vec<&&GameResult>| {
        let wins = games.iter().filter(|r| r.winner == Some(0)).count();
        println!("  {name}: {} 局 / A の勝率 {:.1}%", games.len(), 100.0 * wins as f64 / games.len().max(1) as f64);
    };
    println!();
    println!("両者が C4W を撃った局 {}(決着した {} 局のうち)。撃った時の行数の差(A − B)ごと:", both.len(), results.iter().filter(|r| r.winner.is_some()).count());
    let diff = |r: &GameResult| r.stats[0].first_fire_rows as i32 - r.stats[1].first_fire_rows as i32;
    line("A が4行以上少ない", both.iter().filter(|r| diff(r) <= -4).collect());
    line("A が1〜3行少ない", both.iter().filter(|r| (-3..=-1).contains(&diff(r))).collect());
    line("同じ", both.iter().filter(|r| diff(r) == 0).collect());
    line("A が1〜3行多い", both.iter().filter(|r| (1..=3).contains(&diff(r))).collect());
    line("A が4行以上多い", both.iter().filter(|r| diff(r) >= 4).collect());
    println!("撃った順ごと:");
    line("A が先", both.iter().filter(|r| r.stats[0].first_fire_piece < r.stats[1].first_fire_piece).collect());
    line("同時", both.iter().filter(|r| r.stats[0].first_fire_piece == r.stats[1].first_fire_piece).collect());
    line("B が先", both.iter().filter(|r| r.stats[0].first_fire_piece > r.stats[1].first_fire_piece).collect());
    // 【2026-11-08・ユーザー「互いに C4W を目指した場合、5REN 差があると後打ちが負けることが多い」】後に撃った側から見た行数の差ごと
    println!("後打ちから見た行数の差(後打ち − 先打ち)ごとの後打ちの勝率(同時は除く):");
    let order: Vec<(usize, i32, bool)> = both
        .iter()
        .filter(|r| r.stats[0].first_fire_piece != r.stats[1].first_fire_piece)
        .map(|r| {
            let later = if r.stats[0].first_fire_piece > r.stats[1].first_fire_piece { 0 } else { 1 };
            let diff = r.stats[later].first_fire_rows as i32 - r.stats[1 - later].first_fire_rows as i32;
            (later, diff, r.winner == Some(later))
        })
        .collect();
    for (name, lo, hi) in [("5行以上少ない", i32::MIN, -5), ("1〜4行少ない", -4, -1), ("同じ", 0, 0), ("1〜4行多い", 1, 4), ("5行以上多い", 5, i32::MAX)] {
        let g: Vec<_> = order.iter().filter(|o| o.1 >= lo && o.1 <= hi).collect();
        let w = g.iter().filter(|o| o.2).count();
        println!("  {name}: {} 局 / 後打ちの勝率 {:.1}%", g.len(), 100.0 * w as f64 / g.len().max(1) as f64);
    }
    let avg = |side: usize, f: &dyn Fn(&Stats) -> u32| both.iter().map(|r| f(&r.stats[side]) as f64).sum::<f64>() / both.len().max(1) as f64;
    println!(
        "撃った時の平均: A {:.1} 行({:.1} 手目) / B {:.1} 行({:.1} 手目)",
        avg(0, &|s| s.first_fire_rows),
        avg(0, &|s| s.first_fire_piece),
        avg(1, &|s| s.first_fire_rows),
        avg(1, &|s| s.first_fire_piece)
    );
    let only = |side: usize| results.iter().filter(move |r| r.winner.is_some() && r.stats[side].first_fire_piece > 0 && r.stats[1 - side].first_fire_piece == 0);
    println!(
        "片方だけ撃った局: A だけ {} 局(A の勝率 {:.1}%) / B だけ {} 局(A の勝率 {:.1}%)",
        only(0).count(),
        100.0 * only(0).filter(|r| r.winner == Some(0)).count() as f64 / only(0).count().max(1) as f64,
        only(1).count(),
        100.0 * only(1).filter(|r| r.winner == Some(0)).count() as f64 / only(1).count().max(1) as f64
    );
}

fn summary(names: &[String; 2], results: &[GameResult]) {
    let n = results.len();
    if n == 0 {
        println!("対局の結果が無い");
        return;
    }
    let (a, b, d) = tally(results);
    let decisive = a + b;
    let (lo, hi) = wilson(a, decisive);
    println!();
    println!("結果: A {a} 勝 / B {b} 勝 / 引き分け {d}");
    if decisive > 0 {
        println!(
            "A の勝率(引き分けを除く): {:.1}%(95%信頼区間 {:.1}〜{:.1}%)",
            100.0 * a as f64 / decisive as f64,
            100.0 * lo,
            100.0 * hi
        );
    }

    println!();
    println!("1局あたりの平均:");
    let header = ["", "手数", "攻撃", "攻撃/手", "相殺", "送った", "受けた", "テトリス", "TSS", "TSD", "TST", "ミニ", "パフェ", "B2B", "最大REN", "SD手%", "SD非T%", "消去手%", "B2B切", "REN中切", "単発切"];
    println!("{}", header.iter().map(|h| format!("{h:>8}")).collect::<String>());
    for (side, name) in names.iter().enumerate() {
        let stats: Vec<&Stats> = results.iter().map(|r| &r.stats[side]).collect();
        let avg = |f: &dyn Fn(&Stats) -> u32| stats.iter().map(|s| f(s) as f64).sum::<f64>() / n as f64;
        let pieces = avg(&|s| s.pieces);
        let attack = avg(&|s| s.attack);
        let cols = [
            pieces,
            attack,
            attack / pieces.max(1.0),
            avg(&|s| s.cancelled),
            avg(&|s| s.sent),
            avg(&|s| s.received),
            avg(&|s| s.tetrises),
            avg(&|s| s.tspin_singles),
            avg(&|s| s.tspin_doubles),
            avg(&|s| s.tspin_triples),
            avg(&|s| s.tspin_minis),
            avg(&|s| s.perfect_clears),
            avg(&|s| s.back_to_backs),
            avg(&|s| s.max_ren),
            100.0 * avg(&|s| s.soft_drop_moves) / pieces.max(1.0),
            100.0 * avg(&|s| s.soft_drop_non_spin) / pieces.max(1.0),
            100.0 * avg(&|s| s.clear_moves) / pieces.max(1.0),
            avg(&|s| s.b2b_breaks),
            avg(&|s| s.b2b_breaks_in_ren),
            avg(&|s| s.b2b_breaks_single),
        ];
        let label: String = format!("{}:{}", ["A", "B"][side], name).chars().take(8).collect();
        println!("{label:>8}{}", cols.iter().map(|v| format!("{v:>8.2}")).collect::<String>());
        println!(
            "  TSD の内訳 {label}: 予告あり {:.2} / 予告なしで低い {:.2}",
            avg(&|s| s.tsd_with_incoming),
            avg(&|s| s.tsd_low_quiet)
        );
    }

    println!();
    for side in 0..2 {
        let wins: Vec<&GameResult> = results.iter().filter(|r| r.winner == Some(side)).collect();
        let big = wins.iter().filter(|r| r.stats[side].max_ren >= 8).count();
        println!(
            "{}の勝ち {} 局のうち、REN 8 以上を撃った局 {}({:.1}%)",
            ["A", "B"][side],
            wins.len(),
            big,
            100.0 * big as f64 / wins.len().max(1) as f64
        );
    }
    well_report(results);
    template_report(results);
    println!();
    println!("負け方:");
    for (side, name) in names.iter().enumerate() {
        let count = |l: Loss| results.iter().filter(|r| r.losses[side] == Some(l)).count();
        println!(
            "  {} {name}: 置けない {} / 置けるのに手を返さない {} / 反則(届かない手) {} / 上で固定 {} / おじゃまであふれた {}",
            ["A", "B"][side],
            count(Loss::NoMove),
            count(Loss::BotGaveUp),
            count(Loss::IllegalMove),
            count(Loss::LockOut),
            count(Loss::GarbageOverflow)
        );
    }
}
