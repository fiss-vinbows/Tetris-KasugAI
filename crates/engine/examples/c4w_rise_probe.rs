//! C4W を積んでいる時におじゃまがせり上がると、REN の見込みと選ぶ手がどう変わるかを調べる道具(2026-11-05)。
//! 使い方(TetrisAI フォルダで): cargo run --release -p tetrisai-engine --example c4w_rise_probe -- <重みの TOML>

use tetrisai_core::{Board, Piece, PlayerState, Rules};
use tetrisai_engine::eval::ren_potential;
use tetrisai_engine::{search, shape, Root, SearchOptions, Weights};

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "config/weights/concept.toml".to_string());
    let weights: Weights = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let rules = Rules::ppt2();
    let wall = "###....###";
    let seed = ["###.######", "####.#####"];
    // おじゃまの穴: 左の壁の下・中央の下
    for (name, garbage) in [("なし", vec![]), ("穴が壁1行", vec!["#.########"]), ("穴が中央1行", vec!["####.#####"]), ("穴が壁2行", vec!["#.########"; 2]), ("穴が壁3行", vec!["#########."; 3]), ("穴が中央3行", vec!["#####.####"; 3])] {
        let mut rows: Vec<&str> = vec![wall; 10];
        rows.extend(seed);
        rows.extend(garbage.iter().copied());
        let board = Board::from_rows(&rows);
        for q in ["TLJSZI", "OJTZIL"] {
            let queue: Vec<Piece> = q.chars().map(|c| Piece::from_char(c).unwrap()).collect();
            let root = Root { state: PlayerState::new(board), hold: Some(Piece::Z), queue, ..Default::default() };
            let r = search(&root, &SearchOptions { width: 256, max_depth: 6, speculate: false }, &weights, &rules, &|| false).expect("手が無い");
            let mut state = root.state;
            let mut moves = String::new();
            let mut lines = 0;
            for m in &r.plan {
                let res = state.apply(&m.placement, &rules);
                lines += (res.lines > 0) as u32;
                moves += &format!("{:?}x{}y{}消{} ", m.placement.location.piece, m.placement.location.x, m.placement.location.y, res.lines);
            }
            println!(
                "{name} {q}: 行{} 種{} 中あけ行{} 火力{} 読み筋の消す手{lines}/{} : {moves}",
                shape::ren_rows(&board), shape::center_well_seed(&board), shape::center_well_rows(&board), ren_potential(&rules, &board, 0), r.plan.len()
            );
        }
    }
}
