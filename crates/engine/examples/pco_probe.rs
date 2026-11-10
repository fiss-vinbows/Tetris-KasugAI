//! 開幕パフェ(pco.rs)の成功率と1手の時間を測る(2026-11-09)。
//! 使い方: cargo run --release -p tetrisai-engine --example pco_probe -- [局数] [1手の持ち時間ms]

use std::time::{Duration, Instant};

use tetrisai_core::{Board, Piece, Rules};
use tetrisai_engine::pco::best_action;

fn bag_sequence(seed: u64, n: usize) -> Vec<Piece> {
    let mut rng = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut out = Vec::new();
    while out.len() < n {
        let mut bag = Piece::ALL.to_vec();
        for i in (1..bag.len()).rev() {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            bag.swap(i, (rng % (i as u64 + 1)) as usize);
        }
        out.extend(bag);
    }
    out
}

fn main() {
    if std::env::var("PCO_EXACT").is_ok() {
        exact_check();
        return;
    }
    let args: Vec<String> = std::env::args().collect();
    let games: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(100);
    let budget = Duration::from_millis(args.get(2).and_then(|s| s.parse().ok()).unwrap_or(60));
    let first_budget = Duration::from_millis(args.get(3).and_then(|s| s.parse().ok()).unwrap_or(400));
    // PCO_SOFT があればソフトドロップ1回の置き方も使う(2026-11-09)
    let soft = std::env::var("PCO_SOFT").is_ok();
    let spawn = soft.then(|| Rules::ppt2().spawn());
    let (mut soft_moves, mut moves) = (0u32, 0u32);
    let mut first_ok = 0u32;
    let (mut ok, mut max_ms, mut total_ms, mut calls) = (0, 0u128, 0u128, 0u128);
    let mut fail_at = [0u32; 12];
    // PCO_SEQS=SJIZTOL...,OTZ...: 実機ログから組み立てたミノの順番で試す(足りない所は乱数の袋で埋める)
    let given: Vec<Vec<Piece>> = std::env::var("PCO_SEQS").map_or(Vec::new(), |s| {
        s.split(',').map(|t| t.chars().filter_map(|c| Piece::ALL.iter().copied().find(|p| format!("{p:?}") == c.to_string())).collect()).collect()
    });
    let games = if given.is_empty() { games } else { given.len() as u64 };
    for g in 0..games {
        let mut seq = bag_sequence(g + 1, 40);
        if let Some(head) = given.get(g as usize) {
            // 実機の順番の後ろは、最後の袋の残りから続ける
            let mut s = head.clone();
            let k = s.len() % 7;
            let rest: Vec<Piece> = Piece::ALL.iter().copied().filter(|p| !s[s.len() - k..].contains(p)).collect();
            if k > 0 {
                s.extend(rest);
            }
            s.extend(bag_sequence(g + 1, 40));
            seq = s;
        }
        let mut first_p = -1.0f32;
        let mut board = Board::new();
        let mut hold: Option<Piece> = None;
        let mut next = 0usize; // seq での操作ミノの位置
        let mut placed = 0u32;
        // PCO_DOUBLE があれば、1回目のパフェの後も続けて2回目を狙う(2026-11-09)
        let mut pcs = 0u32;
        let success = loop {
            let queue = &seq[next..next + 6];
            // 見えている最後のミノの後で、今の袋に残っているミノ
            let seen = next + 6;
            let bag_start = seen / 7 * 7;
            let bag: Vec<Piece> = if seen % 7 == 0 { Vec::new() } else { seq[seen..bag_start + 7].to_vec() };
            let t = Instant::now();
            let r = best_action(&board, tetrisai_engine::pco::rows_left(&board, placed, soft).unwrap_or(0), hold, queue, &bag, if placed == 0 { first_budget } else { budget }, spawn);
            let ms = t.elapsed().as_millis();
            max_ms = max_ms.max(ms);
            total_ms += ms;
            calls += 1;
            let Some((a, _p)) = r else { break false };
            if placed == 0 {
                first_p = _p;
            }
            let piece = a.loc.piece;
            let mv = tetrisai_engine::pco::to_move(&board, &a, &Rules::ppt2()).expect("入る手があるはず");
            moves += 1;
            soft_moves += (mv.soft_drop_sections > 0) as u32;
            if piece == seq[next] {
                next += 1;
            } else if hold == Some(piece) {
                hold = Some(seq[next]);
                next += 1;
            } else {
                assert!(hold.is_none() && seq[next + 1] == piece, "置けないミノ");
                hold = Some(seq[next]);
                next += 2;
            }
            board.place(&a.loc);
            board.clear_full_lines();
            if std::env::var("PCO_SHOW").is_ok() {
                eprintln!("手{} {:?} 見込み{:.2} queue {:?} hold {:?}", placed + 1, a.loc, _p, queue, hold);
                for r in board.to_rows().iter().skip_while(|r| !r.contains('#')) {
                    eprintln!("  {r}");
                }
            }
            placed += 1;
            if board.is_empty() {
                pcs += 1;
                if pcs == 1 && std::env::var("PCO_DOUBLE").is_ok() {
                    placed = 0;
                    continue;
                }
                break true;
            }
            if placed > 11 {
                break false;
            }
        };
        if !given.is_empty() {
            println!("{}局目 最初の見込み {first_p:.2} {} {placed}手", g + 1, if success { "パフェ" } else { "失敗" });
        }
        if pcs >= 1 {
            first_ok += 1;
        }
        if success {
            ok += 1;
        } else {
            fail_at[placed.min(11) as usize] += 1;
        }
    }
    if std::env::var("PCO_DOUBLE").is_ok() {
        println!("1回目のパフェ {first_ok}/{games} / 2回目まで {ok}({:.1}%)", ok as f64 / first_ok.max(1) as f64 * 100.0);
    }
    println!(
        "パフェ {ok}/{games}({:.1}%) 1手の時間: 平均 {:.1}ms 最大 {max_ms}ms / 失敗した手数ごと {:?} / ソフトドロップの手 {soft_moves}/{moves}",
        ok as f64 / games as f64 * 100.0,
        total_ms as f64 / calls as f64,
        fail_at
    );
}

#[allow(dead_code)]
pub fn exact_check() {
    let spawn = std::env::var("PCO_SOFT").is_ok().then(|| Rules::ppt2().spawn());
    let mut solved = 0;
    let t = Instant::now();
    for g in 0..100u64 {
        let seq = bag_sequence(g + 1, 14);
        if best_action(&Board::new(), 4, None, &seq[..11], &[], Duration::from_secs(10), spawn).is_some() {
            solved += 1;
        }
    }
    println!("全部見えている時のパフェ: {solved}/100 時間 {:?}", t.elapsed());
}
