//! 開幕の選択(`OpeningMode`)ごとに、相手なしで置かせて、狙いどおりの開幕になるかを数える道具(2026-11-05)。
//! 使い方(TetrisAI フォルダで): cargo run --release -p tetrisai-engine --example opening_mode_probe -- <重みの TOML> [局数]

use tetrisai_core::{Piece, PlayerState, Rules, Spin};
use tetrisai_engine::{search, OpeningMode, OpeningTracker, Root, SearchOptions, Weights};

const PIECES: [Piece; 7] = [Piece::I, Piece::O, Piece::T, Piece::L, Piece::J, Piece::S, Piece::Z];

/// 7種1巡の袋でミノ列を作る。
fn sequence(seed: u64, len: usize) -> Vec<Piece> {
    let mut x = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut out = Vec::new();
    while out.len() < len {
        let mut bag = PIECES;
        for i in (1..7).rev() {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            bag.swap(i, (x % (i as u64 + 1)) as usize);
        }
        out.extend(bag);
    }
    out
}

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "config/weights/concept.toml".to_string());
    let games: u64 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(100);
    let weights: Weights = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let rules = Rules::ppt2();
    for (name, mode) in [("TDテンプレ", OpeningMode::Td), ("開幕TSD", OpeningMode::Tsd), ("種3C4W", OpeningMode::Seed3), ("ランダム", OpeningMode::Random)] {
        let (mut first_tsd, mut tsd_by_20, mut tsd_moves, mut seed3, mut ren3) = (0, 0, 0u32, 0, 0);
        for g in 0..games {
            let seq = sequence(g + 1, 80);
            let mut tracker = OpeningTracker::with_mode(g * 2 + 1, mode);
            let mut state = PlayerState::new(Default::default());
            let (mut hold, mut next, mut first_clear) = (None, 0usize, None);
            let (mut got_tsd, mut got_seed, mut got_ren) = (false, false, false);
            for moved in 0..30u32 {
                let queue: Vec<Piece> = seq[next..next + 6].to_vec();
                let mut root = Root { state, hold, queue: queue.clone(), ..Default::default() };
                root.opening = tracker.observe(&root, &rules);
                let Some(r) = search(&root, &SearchOptions { width: 128, max_depth: 6, speculate: false }, &weights, &rules, &|| false) else { break };
                let Some(m) = r.plan.first() else { break };
                if m.placement.location.piece != queue[0] {
                    if hold.is_none() {
                        next += 1;
                    }
                    hold = Some(queue[0]);
                }
                next += 1;
                let res = state.apply(&m.placement, &rules);
                let tsd = res.spin == Spin::Full && res.lines == 2;
                if res.lines > 0 && first_clear.is_none() {
                    first_clear = Some(tsd);
                }
                if tsd && moved < 20 {
                    got_tsd = true;
                }
                if tsd {
                    tsd_moves = tsd_moves.max(moved);
                }
                got_seed |= tetrisai_engine::shape::center_well_seed(&state.board) == 3;
                got_ren |= res.ren >= 3;
            }
            first_tsd += first_clear.unwrap_or(false) as u32;
            tsd_by_20 += got_tsd as u32;
            seed3 += got_seed as u32;
            ren3 += got_ren as u32;
        }
        println!("{name}: 最初の消去が TSD {first_tsd}/{games} / 20手以内に TSD {tsd_by_20}/{games} / 30手以内に種3の形 {seed3}/{games} / 30手以内に 3REN 以上 {ren3}/{games}");
    }
}
