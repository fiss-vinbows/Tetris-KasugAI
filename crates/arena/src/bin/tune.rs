//! 評価の重みの自動調整(SPSA)。
//!
//! 1回の反復:
//! 1. 調整する重みすべてを、ランダムな向き δ(各 ±1)へ幅 c だけずらした「+側」と「−側」を作る
//! 2. +側と−側を、同じミノ列で手番を入れ替えて何局も対戦させる(同じプロセス内の探索、幅は固定)
//! 3. 勝ち越した側へ重みを動かす: θ += 学習率 × c × (+側の勝ち − −側の勝ち)/局数 × δ
//!
//! 初心者向けの減点(ソフトドロップの区間・途中で止める)は調整しない。中あけ積みを始める高さ(ユーザーの指定)も調整しない。
//! 【2026-11-08・ユーザー「移動速度を早くする学習」】時間ありの対局(--timed)は1手の実機の時間(キーの数・ソフトドロップの
//! 段数・消去の演出)を数えるので、ソフトドロップの段数(`soft_drop_rows`)・Tスピン以外のソフトドロップ(`soft_drop_non_spin`)・
//! キー1回(`input_keys`)も調整する(時間の損が勝敗に表れる)。
//!
//! 使い方(TetrisAI フォルダで):
//!     cargo run --release -p tetrisai-arena --bin tune -- [オプション]
//!   --out <フォルダ>       途中経過の保存先(既定 records/tune)。state.json があれば続きから再開する
//!   --hours <時間>         この時間が過ぎたら止める(既定 1)
//!   --iterations <数>      反復回数の上限(既定 なし)
//!   --games <数>           1回の反復の対局数(偶数。既定 24)
//!   --width <数>           探索の幅(既定 64)
//!   --threads <数>         同時に進める対局数(既定: 論理コア数)
//!   --max-pieces <数>      1人あたりの手数の上限。達したら危なさ(高さ + 予告)で判定する(既定 300)
//!   --init <file>          最初の重み(TOML。既定は既定値)
//!   --no-opponent          相手の情報を使わずに対戦させる(TBA は相手の盤面をまだ読まないので、実際に使う条件に合わせる)
//!   --lr <数>              最初の学習率(既定 0.5)
//!   --c <数>               最初のずらし幅(重みの大きさに対する割合。既定 0.25)
//!   --seed-offset <数>     ミノ列とずらす向きの乱数をずらす(既定 0)。同じ出発点から別の調整をする時に変える
//!                          (2026-11-08: 変えずに2回走らせたら、2回目が1回目とほぼ同じ重みになった)

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use serde_json::json;
use tetrisai_arena::agent::EngineAgent;
use tetrisai_arena::game::{play_game, GameConfig};
use tetrisai_core::Rules;
use tetrisai_engine::{OpeningMode, OpeningTracker, Weights};

/// 調整する重みの名前(Weights のフィールド。配列は [添字])。
const NAMES: [&str; 59] = [
    "cell_coveredness",
    "holes",
    "row_transitions",
    "height",
    "height_upper_half",
    "height_upper_quarter",
    "tetris_well_depth",
    "tslot[0]",
    "tslot[1]",
    "tslot[2]",
    "tslot[3]",
    "has_back_to_back",
    "wasted_t",
    "normal_clears[1]",
    "normal_clears[2]",
    "normal_clears[3]",
    "normal_clears[4]",
    "mini_spin_clears[1]",
    "mini_spin_clears[2]",
    "spin_clears[1]",
    "spin_clears[2]",
    "spin_clears[3]",
    "back_to_back_clear",
    "combo_attack",
    "perfect_clear",
    "pending_garbage",
    "overflow",
    "attack_sent",
    "attack_sent_danger",
    "lethal",
    "threat_overflow",
    "center_well_rows",
    "center_well_side_diff",
    "air_tsd_slot",
    "tsd_slot_any",
    "pc_chance",
    "tst_slot",
    "c_spin",
    "center_well_seed",
    "ren_potential",
    "rush_attack",
    "rival_ren",
    "ren_threat_overflow",
    "tsd_counter",
    "rival_deficit",
    "rival_early_fire",
    "soft_drop_rows",
    "soft_drop_non_spin",
    "input_keys",
    "rush_pressure",
    "rush_lethal",
    "rush_height",
    "rush_ren_overflow",
    "rush_finish",
    // 【2026-11-14】迎え撃ち(`--counter-profile` で迎え撃ちの重み一式に入れる)
    "counter_cancel",
    "counter_sent",
    // 【2026-11-14・ユーザー「あと1手積み込むとテトリス消しになるのにREN重視で3列消しすることがあります」】REN を切る手の減点
    // (2026-11-07 に手で -20 にした)と、TSD の溝をためる・予告が無いのに撃つ手の点(迎え撃ちで使う)も動かせるようにする
    "ren_cut",
    "tsd_stock",
    "tsd_low_fire",
];

fn get_params(w: &Weights) -> Vec<f64> {
    let v = [
        w.cell_coveredness,
        w.holes,
        w.row_transitions,
        w.height,
        w.height_upper_half,
        w.height_upper_quarter,
        w.tetris_well_depth,
        w.tslot[0],
        w.tslot[1],
        w.tslot[2],
        w.tslot[3],
        w.has_back_to_back,
        w.wasted_t,
        w.normal_clears[1],
        w.normal_clears[2],
        w.normal_clears[3],
        w.normal_clears[4],
        w.mini_spin_clears[1],
        w.mini_spin_clears[2],
        w.spin_clears[1],
        w.spin_clears[2],
        w.spin_clears[3],
        w.back_to_back_clear,
        w.combo_attack,
        w.perfect_clear,
        w.pending_garbage,
        w.overflow,
        w.attack_sent,
        w.attack_sent_danger,
        w.lethal,
        w.threat_overflow,
        w.center_well_rows,
        w.center_well_side_diff,
        w.air_tsd_slot,
        w.tsd_slot_any,
        w.pc_chance,
        w.tst_slot,
        w.c_spin,
        w.center_well_seed,
        w.ren_potential,
        w.rush_attack,
        w.rival_ren,
        w.ren_threat_overflow,
        w.tsd_counter,
        w.rival_deficit,
        w.rival_early_fire,
        w.soft_drop_rows,
        w.soft_drop_non_spin,
        w.input_keys,
        w.rush_pressure,
        w.rush_lethal,
        w.rush_height,
        w.rush_ren_overflow,
        w.rush_finish,
        w.counter_cancel,
        w.counter_sent,
        w.ren_cut,
        w.tsd_stock,
        w.tsd_low_fire,
    ];
    v.iter().map(|&x| x as f64).collect()
}

fn set_params(w: &mut Weights, v: &[f64]) {
    let f = |i: usize| v[i] as f32;
    w.cell_coveredness = f(0);
    w.holes = f(1);
    w.row_transitions = f(2);
    w.height = f(3);
    w.height_upper_half = f(4);
    w.height_upper_quarter = f(5);
    w.tetris_well_depth = f(6);
    w.tslot = [f(7), f(8), f(9), f(10)];
    w.has_back_to_back = f(11);
    w.wasted_t = f(12);
    w.normal_clears[1..5].copy_from_slice(&[f(13), f(14), f(15), f(16)]);
    w.mini_spin_clears[1..3].copy_from_slice(&[f(17), f(18)]);
    w.spin_clears[1..4].copy_from_slice(&[f(19), f(20), f(21)]);
    w.back_to_back_clear = f(22);
    w.combo_attack = f(23);
    w.perfect_clear = f(24);
    w.pending_garbage = f(25);
    w.overflow = f(26);
    w.attack_sent = f(27);
    w.attack_sent_danger = f(28);
    w.lethal = f(29);
    w.threat_overflow = f(30);
    w.center_well_rows = f(31);
    w.center_well_side_diff = f(32);
    w.air_tsd_slot = f(33);
    w.tsd_slot_any = f(34);
    w.pc_chance = f(35);
    w.tst_slot = f(36);
    w.c_spin = f(37);
    w.center_well_seed = f(38);
    w.ren_potential = f(39);
    w.rush_attack = f(40);
    w.rival_ren = f(41);
    w.ren_threat_overflow = f(42);
    w.tsd_counter = f(43);
    w.rival_deficit = f(44);
    w.rival_early_fire = f(45);
    w.soft_drop_rows = f(46);
    w.soft_drop_non_spin = f(47);
    w.input_keys = f(48);
    w.rush_pressure = f(49);
    w.rush_lethal = f(50);
    w.rush_height = f(51);
    w.rush_ren_overflow = f(52);
    w.rush_finish = f(53);
    w.counter_cancel = f(54);
    w.counter_sent = f(55);
    w.ren_cut = f(56);
    w.tsd_stock = f(57);
    w.tsd_low_fire = f(58);
}

/// 調整した値 `theta` から重みを作る。`rush_profile` なら、早撃ちの間だけ使う重み一式(`Weights::rush`)に入れる(2026-11-09)。
/// `counter_profile` なら、迎え撃ちの間だけ使う重み一式(`Weights::counter`)に入れる(2026-11-14)
fn build(base: &Weights, theta: &[f64], rush_profile: bool, counter_profile: bool) -> Weights {
    let mut w = base.clone();
    if counter_profile {
        let mut r = base.counter.as_deref().cloned().unwrap_or_else(|| {
            let mut r = base.clone();
            r.rush = None;
            r.counter = None;
            r
        });
        set_params(&mut r, theta);
        w.counter = Some(Box::new(r));
    } else if rush_profile {
        let mut r = base.rush.as_deref().cloned().unwrap_or_else(|| {
            let mut r = base.clone();
            r.rush = None;
            r
        });
        set_params(&mut r, theta);
        w.rush = Some(Box::new(r));
    } else {
        set_params(&mut w, theta);
    }
    w
}

/// 学習率とずらし幅の減り方(反復回数 k に対して)
fn learning_rate(lr0: f64, k: usize) -> f64 {
    lr0 / (1.0 + k as f64 / 500.0).powf(0.602)
}
fn perturbation(c0: f64, k: usize) -> f64 {
    c0 / (1.0 + k as f64 / 500.0).powf(0.101)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let get = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let num = |name: &str, default: f64| get(name).map(|s| s.parse().expect("数値を指定する")).unwrap_or(default);

    let out = PathBuf::from(get("--out").unwrap_or_else(|| "records/tune".to_string()));
    fs::create_dir_all(&out).expect("保存先を作れない");
    let hours = num("--hours", 1.0);
    let max_iterations = get("--iterations").map(|s| s.parse::<usize>().unwrap()).unwrap_or(usize::MAX);
    let games = (num("--games", 24.0) as usize / 2 * 2).max(2);
    let width = num("--width", 64.0) as usize;
    let lr0 = num("--lr", 0.5);
    let seed_offset = num("--seed-offset", 0.0) as u64;
    let c0 = num("--c", 0.25);
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
    let threads = num("--threads", cores as f64) as usize;
    let rules = Rules::ppt2();
    let use_opponent = !args.iter().any(|a| a == "--no-opponent");
    // 【2026-11-07・ユーザー「相手の C4W になすすべなく負ける」】--vs <重み> で、+側と−側を同じ相手・同じミノ列で戦わせて比べる。
    // 相手は3組のうち2組が開幕 C4W(seed3)、1組が既定の開幕。こちらの開幕は --own-opening(既定 no_c4w)
    let vs: Option<Weights> =
        get("--vs").map(|path| toml::from_str(&fs::read_to_string(&path).expect("--vs の重みを読めない")).expect("--vs の重みが壊れている"));
    // 【2026-11-09】--rush-profile: 早撃ちの間だけ使う重み一式を調整する(ふだんの重みは変えない)。
    // --only a,b,c: 名前を挙げた重みだけ動かす。--vs-opening: 相手の開幕を固定する(既定は3組のうち2組が seed3)
    let rush_profile = args.iter().any(|a| a == "--rush-profile");
    // 【2026-11-14・ユーザー「間に合わない場合は火力をため込んで相殺して迎え撃つ方針」】--counter-profile: 迎え撃ちの間だけ使う重み一式を調整する
    let counter_profile = args.iter().any(|a| a == "--counter-profile");
    let only: Option<Vec<String>> = get("--only").map(|s| s.split(',').map(|x| x.trim().to_string()).collect());
    if let Some(only) = &only {
        for n in only {
            assert!(NAMES.contains(&n.as_str()), "--only の名前が無い: {n}");
        }
    }
    // 【2026-11-10】--vs-opening td,tsd,seed3 のように並べると、1組ごとに順に使う(TD 前提の作戦を3種類の相手で調整する)
    let vs_openings: Vec<OpeningMode> = get("--vs-opening")
        .map(|s| s.split(',').map(|x| OpeningMode::parse(x).expect("--vs-opening の値")).collect())
        .unwrap_or_default();
    let own_opening = OpeningMode::parse(&get("--own-opening").unwrap_or_else(|| "no_c4w".to_string())).expect("--own-opening の値");
    let config = GameConfig {
        rules: rules.clone(),
        think: Duration::ZERO,
        max_pieces: num("--max-pieces", 300.0) as u32,
        adjudicate: true,
        block_margin: num("--block-margin", 0.0) as u32,
        timed: args.iter().any(|a| a == "--timed"),
        time_pct: [100, num("--vs-time-pct", 100.0) as u32],
    };

    let base = match get("--init") {
        Some(path) => toml::from_str(&fs::read_to_string(&path).expect("最初の重みを読めない")).expect("最初の重みの形式が違う"),
        None => Weights::default(),
    };
    // ずらし幅の基準(最初の値の大きさ。0 に近い重みも動けるよう下限を付ける)
    let start_params = if counter_profile {
        get_params(base.counter.as_deref().unwrap_or(&base))
    } else if rush_profile {
        get_params(base.rush.as_deref().unwrap_or(&base))
    } else {
        get_params(&base)
    };
    let scale: Vec<f64> = start_params.iter().map(|x| x.abs().max(0.5)).collect();
    let movable: Vec<bool> = NAMES.iter().map(|n| only.as_ref().is_none_or(|o| o.iter().any(|x| x == n))).collect();

    // 続きから再開
    let state_path = out.join("state.json");
    let (mut k, mut theta) = match fs::read_to_string(&state_path) {
        Ok(text) => {
            let v: serde_json::Value = serde_json::from_str(&text).expect("state.json が壊れている");
            let theta: Vec<f64> = serde_json::from_value(v["theta"].clone()).unwrap();
            println!("{} から再開(反復 {})", state_path.display(), v["k"]);
            (v["k"].as_u64().unwrap() as usize, theta)
        }
        Err(_) => (0, start_params.clone()),
    };
    let log_path = out.join("log.csv");
    if !log_path.exists() {
        fs::write(&log_path, format!("k,plus_wins,minus_wins,draws,avg_pieces,{}\n", NAMES.join(","))).unwrap();
    }

    println!(
        "自動調整: 1回 {games} 局、幅 {width}、同時 {threads} 局、最長 {hours} 時間、保存先 {}",
        out.display()
    );
    let started = Instant::now();
    let deadline = Duration::from_secs_f64(hours * 3600.0);
    let mut rng = StdRng::seed_from_u64(0x5EED ^ k as u64 ^ seed_offset.wrapping_mul(0x9E37_79B9));
    let mut recent = Vec::new();

    while started.elapsed() < deadline && k < max_iterations {
        let c = perturbation(c0, k);
        let delta: Vec<f64> =
            (0..NAMES.len()).map(|i| if rng.gen::<bool>() { 1.0 } else { -1.0 } * if movable[i] { 1.0 } else { 0.0 }).collect();
        let shift: Vec<f64> = (0..NAMES.len()).map(|i| c * scale[i] * delta[i]).collect();
        let plus = build(&base, &theta.iter().zip(&shift).map(|(t, s)| t + s).collect::<Vec<_>>(), rush_profile, counter_profile);
        let minus = build(&base, &theta.iter().zip(&shift).map(|(t, s)| t - s).collect::<Vec<_>>(), rush_profile, counter_profile);

        // 同じミノ列で手番を入れ替えた2局を1組にする
        let first_seed = 1_000_000 + (k * games) as u64 + seed_offset * 100_000_000;
        let next = AtomicUsize::new(0);
        let tally = Mutex::new((0usize, 0usize, 0usize, 0u64)); // (+勝, −勝, 引き分け, 手数の合計)
        std::thread::scope(|s| {
            for _ in 0..threads.min(games) {
                s.spawn(|| loop {
                    let g = next.fetch_add(1, Ordering::SeqCst);
                    if g >= games {
                        break;
                    }
                    let seed = first_seed + (g / 2) as u64;
                    let plus_first = g.is_multiple_of(2);
                    if let Some(vs) = &vs {
                        // 同じ相手・同じミノ列で +側と−側を1局ずつ
                        let mode = if vs_openings.is_empty() {
                            if (g / 2) % 3 == 2 { OpeningMode::TemplateOrPlain } else { OpeningMode::Seed3 }
                        } else {
                            vs_openings[(g / 2) % vs_openings.len()]
                        };
                        let mut own = EngineAgent::new(if plus_first { plus.clone() } else { minus.clone() }, rules.clone(), width, "own");
                        let mut rival = EngineAgent::new(vs.clone(), rules.clone(), width, "vs");
                        let w = if plus_first { &plus } else { &minus };
                        own.opening = Some(OpeningTracker::with_mode(seed, own_opening)
                                .with_after_pc(w.after_pc_seed, w.after_pc_no_rival)
                                .with_attacked_well(w.td_attacked_well, w.td_attacked_clears));
                        rival.opening = Some(OpeningTracker::with_mode(seed ^ 0x55, mode));
                        let result = play_game([&mut own, &mut rival], seed, &config).expect("同じプロセス内の対局は失敗しない");
                        let mut t = tally.lock().unwrap();
                        match (result.winner, plus_first) {
                            (Some(0), true) => t.0 += 1,
                            (Some(0), false) => t.1 += 1,
                            (None, _) => t.2 += 1,
                            _ => {}
                        }
                        t.3 += (result.stats[0].pieces + result.stats[1].pieces) as u64;
                        continue;
                    }
                    let mut p = EngineAgent::new(plus.clone(), rules.clone(), width, "+");
                    let mut m = EngineAgent::new(minus.clone(), rules.clone(), width, "-");
                    p.use_opponent = use_opponent;
                    m.use_opponent = use_opponent;
                    let result = if plus_first {
                        play_game([&mut p, &mut m], seed, &config)
                    } else {
                        play_game([&mut m, &mut p], seed, &config)
                    }
                    .expect("同じプロセス内の対局は失敗しない");
                    let plus_side = if plus_first { 0 } else { 1 };
                    let mut t = tally.lock().unwrap();
                    match result.winner {
                        Some(w) if w == plus_side => t.0 += 1,
                        Some(_) => t.1 += 1,
                        None => t.2 += 1,
                    }
                    t.3 += (result.stats[0].pieces + result.stats[1].pieces) as u64;
                });
            }
        });
        let (pw, mw, draws, pieces) = tally.into_inner().unwrap();
        let score = (pw as f64 - mw as f64) / games as f64;
        let lr = learning_rate(lr0, k);
        for i in 0..theta.len() {
            theta[i] += lr * shift[i] * score;
        }
        k += 1;

        let avg_pieces = pieces as f64 / (2 * games) as f64;
        let row: Vec<String> = theta.iter().map(|x| format!("{x:.4}")).collect();
        let line = format!("{k},{pw},{mw},{draws},{avg_pieces:.1},{}\n", row.join(","));
        let mut log = fs::OpenOptions::new().append(true).open(&log_path).unwrap();
        log.write_all(line.as_bytes()).unwrap();
        recent.push(score.abs());

        if k % 10 == 0 {
            let mean = recent.iter().sum::<f64>() / recent.len() as f64;
            recent.clear();
            println!(
                "反復 {k}: 経過 {:.0} 分、直近10回の |勝ち越し| 平均 {mean:.3}、1局あたり {avg_pieces:.0} 手、ずらし幅 {c:.3}",
                started.elapsed().as_secs_f64() / 60.0
            );
        }
        if k % 25 == 0 {
            save(&out, k, &build(&base, &theta, rush_profile, counter_profile), &theta);
        }
    }
    save(&out, k, &build(&base, &theta, rush_profile, counter_profile), &theta);
    println!("終了: 反復 {k}、経過 {:.1} 時間", started.elapsed().as_secs_f64() / 3600.0);
}

/// 途中経過(state.json)と、今の重み(latest.toml と、反復回数付きの控え)を保存する。
fn save(out: &Path, k: usize, w: &Weights, theta: &[f64]) {
    let state = json!({"k": k, "names": NAMES.to_vec(), "theta": theta});
    fs::write(out.join("state.json"), serde_json::to_string_pretty(&state).unwrap()).unwrap();
    let text = toml::to_string(w).unwrap();
    fs::write(out.join("latest.toml"), &text).unwrap();
    if k.is_multiple_of(100) {
        fs::write(out.join(format!("weights_k{k:05}.toml")), &text).unwrap();
    }
}
