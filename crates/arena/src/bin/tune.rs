//! 評価の重みの自動調整(SPSA)。
//!
//! 1回の反復:
//! 1. 調整する重みすべてを、ランダムな向き δ(各 ±1)へ幅 c だけずらした「+側」と「−側」を作る
//! 2. +側と−側を、同じミノ列で手番を入れ替えて何局も対戦させる(同じプロセス内の探索、幅は固定)
//! 3. 勝ち越した側へ重みを動かす: θ += 学習率 × c × (+側の勝ち − −側の勝ち)/局数 × δ
//!
//! 初心者向けの減点(ソフトドロップ・途中で止める)は、強さのために削られないよう調整しない。
//!
//! 使い方(TetrisAI フォルダで):
//!     cargo run --release -p tetrisai-arena --bin tune -- [オプション]
//!   --out <フォルダ>       途中経過の保存先(既定 records/tune)。state.json があれば続きから再開する
//!   --hours <時間>         この時間が過ぎたら止める(既定 1)
//!   --iterations <数>      反復回数の上限(既定 なし)
//!   --games <数>           1回の反復の対局数(偶数。既定 24)
//!   --width <数>           探索の幅(既定 64)
//!   --threads <数>         同時に進める対局数(既定: 論理コア数)
//!   --max-pieces <数>      1人あたりの手数の上限(既定 400)
//!   --init <file>          最初の重み(TOML。既定は既定値)

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
use tetrisai_engine::Weights;

/// 調整する重みの名前(Weights のフィールド。配列は [添字])。
const NAMES: [&str; 25] = [
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
}

/// 学習率とずらし幅の減り方(反復回数 k に対して)
fn learning_rate(k: usize) -> f64 {
    2.0 / (1.0 + k as f64 / 500.0).powf(0.602)
}
fn perturbation(k: usize) -> f64 {
    0.25 / (1.0 + k as f64 / 500.0).powf(0.101)
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
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(2);
    let threads = num("--threads", cores as f64) as usize;
    let rules = Rules::ppt2();
    let config = GameConfig { rules: rules.clone(), think: Duration::ZERO, max_pieces: num("--max-pieces", 400.0) as u32 };

    let base = match get("--init") {
        Some(path) => toml::from_str(&fs::read_to_string(&path).expect("最初の重みを読めない")).expect("最初の重みの形式が違う"),
        None => Weights::default(),
    };
    // ずらし幅の基準(最初の値の大きさ。0 に近い重みも動けるよう下限を付ける)
    let scale: Vec<f64> = get_params(&base).iter().map(|x| x.abs().max(0.5)).collect();

    // 続きから再開
    let state_path = out.join("state.json");
    let (mut k, mut theta) = match fs::read_to_string(&state_path) {
        Ok(text) => {
            let v: serde_json::Value = serde_json::from_str(&text).expect("state.json が壊れている");
            let theta: Vec<f64> = serde_json::from_value(v["theta"].clone()).unwrap();
            println!("{} から再開(反復 {})", state_path.display(), v["k"]);
            (v["k"].as_u64().unwrap() as usize, theta)
        }
        Err(_) => (0, get_params(&base)),
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
    let mut rng = StdRng::seed_from_u64(0x5EED ^ k as u64);
    let mut recent = Vec::new();

    while started.elapsed() < deadline && k < max_iterations {
        let c = perturbation(k);
        let delta: Vec<f64> = (0..NAMES.len()).map(|_| if rng.gen::<bool>() { 1.0 } else { -1.0 }).collect();
        let shift: Vec<f64> = (0..NAMES.len()).map(|i| c * scale[i] * delta[i]).collect();
        let mut plus = base.clone();
        set_params(&mut plus, &theta.iter().zip(&shift).map(|(t, s)| t + s).collect::<Vec<_>>());
        let mut minus = base.clone();
        set_params(&mut minus, &theta.iter().zip(&shift).map(|(t, s)| t - s).collect::<Vec<_>>());

        // 同じミノ列で手番を入れ替えた2局を1組にする
        let first_seed = 1_000_000 + (k * games) as u64;
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
                    let plus_first = g % 2 == 0;
                    let mut p = EngineAgent::new(plus.clone(), rules.clone(), width, "+");
                    let mut m = EngineAgent::new(minus.clone(), rules.clone(), width, "-");
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
        let lr = learning_rate(k);
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
            save(&out, k, &theta, &base);
        }
    }
    save(&out, k, &theta, &base);
    println!("終了: 反復 {k}、経過 {:.1} 時間", started.elapsed().as_secs_f64() / 3600.0);
}

/// 途中経過(state.json)と、今の重み(latest.toml と、反復回数付きの控え)を保存する。
fn save(out: &Path, k: usize, theta: &[f64], base: &Weights) {
    let state = json!({"k": k, "names": NAMES, "theta": theta});
    fs::write(out.join("state.json"), serde_json::to_string_pretty(&state).unwrap()).unwrap();
    let mut w = base.clone();
    set_params(&mut w, theta);
    let text = toml::to_string(&w).unwrap();
    fs::write(out.join("latest.toml"), &text).unwrap();
    if k % 100 == 0 {
        fs::write(out.join(format!("weights_k{k:05}.toml")), &text).unwrap();
    }
}
