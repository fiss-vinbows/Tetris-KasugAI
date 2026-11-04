//! C4W の壁の高さごとに、探索が REN を撃つか(次の手で消すか)を調べる道具(2026-11-04)。
//! 使い方(TetrisAI フォルダで): cargo run --release -p tetrisai-engine --example c4w_fire_probe -- <重みの TOML>

use tetrisai_core::{Board, Piece, PlayerState, Rules};
use tetrisai_engine::{search, Root, SearchOptions, Weights};

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "config/weights/concept.toml".to_string());
    let weights: Weights = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let rules = Rules::ppt2();
    for q in ["JTZOIL", "OJLTSI", "LSTIJZ"] {
        for h in [8usize, 10, 12, 14, 16, 18, 19] {
            // 下は種3(残り3マス)、その上は高さ h までの壁
            let mut rows: Vec<&str> = vec!["###....###"; h - 1];
            rows.push("####...###");
            let board = Board::from_rows(&rows);
            let queue: Vec<Piece> = q.chars().map(|c| Piece::from_char(c).unwrap()).collect();
            let root = Root { state: PlayerState::new(board), queue, ..Default::default() };
            let r = search(&root, &SearchOptions { width: 256, max_depth: 6, speculate: false }, &weights, &rules, &|| false)
                .expect("手が無い");
            let mut state = root.state;
            let plan: Vec<String> = r
                .plan
                .iter()
                .map(|m| {
                    let res = state.apply(&m.placement, &rules);
                    format!("{:?}y{}消{}", m.placement.location.piece, m.placement.location.y, res.lines)
                })
                .collect();
            println!("{q} 高さ{h:>2}: {}", plan.join(" "));
        }
    }
}
