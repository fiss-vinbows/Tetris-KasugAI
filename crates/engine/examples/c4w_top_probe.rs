//! 実機の 20:13:33(C4W の壁が天井近く、操作 O、HOLD Z)で、探索が何を選ぶかを調べる道具(2026-11-05)。
//! 使い方(TetrisAI フォルダで): cargo run --release -p tetrisai-engine --example c4w_top_probe -- <重みの TOML>

use tetrisai_core::{Board, Piece, PlayerState, Rules};
use tetrisai_engine::{search, Root, SearchOptions, Weights};

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "config/weights/concept.toml".to_string());
    let weights: Weights = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let rules = Rules::ppt2();
    let rows = vec![
        ".#........", "###....#..", "###....###", "###....###", "###....###", "###....###", "###....###",
        "###..#.###", "###.######", "###..#####", "###...####", "###...####", "###..#####", "###..#####",
        "####.#####", ".#########", ".#########", "#####.####", "###.######", "##.#######",
    ];
    let queue: Vec<Piece> = "OIJSLT".chars().map(|c| Piece::from_char(c).unwrap()).collect();
    let root = Root { state: PlayerState::new(Board::from_rows(&rows)), hold: Some(Piece::Z), queue, ..Default::default() };
    let r = search(&root, &SearchOptions { width: 256, max_depth: 6, speculate: false }, &weights, &rules, &|| false).expect("手が無い");
    let mut state = root.state;
    for m in &r.plan {
        let res = state.apply(&m.placement, &rules);
        println!("{:?} x={} y={} rot={:?} 消した行 {} REN {} 高さ {}", m.placement.location.piece, m.placement.location.x, m.placement.location.y, m.placement.location.rotation, res.lines, res.ren, state.board.max_height());
    }
    println!("{}", state.board.to_rows().join("\n"));
}
