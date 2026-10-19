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
//!   --a-no-incoming / --b-no-incoming  --a-weights / --b-weights の側に予告を教えない(比較実験用)
//!   --record <file>     1局ごとの結果を JSON Lines で書き出す

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
use tetrisai_engine::Weights;

enum Engine {
    /// TBP の bot プロセス
    Tbp { exe: PathBuf, args: Vec<String> },
    /// このAIの探索を直接呼ぶ
    InProcess { weights: Weights, label: String, width: usize, use_incoming: bool },
}

impl Engine {
    fn create(&self, rules: &Rules) -> Box<dyn Agent + Send> {
        match self {
            Engine::Tbp { exe, args } => Box::new(TbpBot::spawn(exe, args).expect("bot を起動できない")),
            Engine::InProcess { weights, label, width, use_incoming } => {
                let mut agent = EngineAgent::new(weights.clone(), rules.clone(), *width, label);
                agent.use_incoming = *use_incoming;
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
            let label = if use_incoming { stem } else { format!("{stem}(予告なし)") };
            Engine::InProcess { weights: load_weights(&path), label, width, use_incoming }
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
    });
    let mut record = get("--record").map(|path| File::create(path).expect("記録ファイルを作れない"));

    // 対局を並列に進める。各スレッドは自分の bot 2つを起動し、対局ごとに使い回す
    let next = Arc::new(AtomicU64::new(0));
    let (tx, rx) = mpsc::channel::<GameResult>();
    let mut names = [String::new(), String::new()];
    let mut handles = Vec::new();
    for t in 0..threads {
        let (next, tx, engines, config) = (next.clone(), tx.clone(), engines.clone(), config.clone());
        let mut a = engines[0].create(&config.rules);
        let mut b = engines[1].create(&config.rules);
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
    let header = ["", "手数", "攻撃", "攻撃/手", "相殺", "送った", "受けた", "テトリス", "TSS", "TSD", "TST", "ミニ", "パフェ", "B2B", "最大REN", "SD手%"];
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
        ];
        let label: String = format!("{}:{}", ["A", "B"][side], name).chars().take(8).collect();
        println!("{label:>8}{}", cols.iter().map(|v| format!("{v:>8.2}")).collect::<String>());
    }

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
