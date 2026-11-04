//! 実機で負けた局面(C4W の壁が天井近く)で、探索が何を選ぶかを調べる道具(2026-11-04)。
//! 使い方(TetrisAI フォルダで): cargo run --release -p tetrisai-engine --example topout_probe -- <重みの TOML>

use tetrisai_core::{Board, Piece, PlayerState, Rules};
use tetrisai_engine::{search, Root, SearchOptions, Weights};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let path = std::env::args().nth(1).unwrap_or_else(|| "config/weights/concept.toml".to_string());
    let weights: Weights = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let rules = Rules::ppt2();
    // 局面(21:07:06 の画面、操作 O、HOLD Z、NEXT JTZOI)と、その下の種を整えた比較用
    let real: Vec<&str> = vec!["..#.......","..#.......","#.#....###","####...###"]
        .into_iter().chain(std::iter::repeat("###....###").take(12)).chain(["####...###","#####..###","#########.","#########."]).collect();
    let mut clean = real.clone(); clean[18]="##########"; clean[19]="##########";
    let _ = &args;
    for rows in [real, clean] {
    let board = Board::from_rows(&rows);
    let queue: Vec<Piece> = "OJTZOI".chars().map(|c| Piece::from_char(c).unwrap()).collect();
    let root = Root { state: PlayerState::new(board), hold: Some(Piece::Z), queue, ..Default::default() };
    let r = search(&root, &SearchOptions { width: 256, max_depth: 6, speculate: false }, &weights, &rules, &|| false)
        .expect("手が無い");
    let mut state = root.state;
    for m in &r.plan {
        let res = state.apply(&m.placement, &rules);
        println!("{:?} x={} y={} 消した行 {} 高さ {}", m.placement.location.piece, m.placement.location.x, m.placement.location.y, res.lines, state.board.max_height());
    }
    println!("{}", state.board.to_rows().join("\n"));
    println!("---");
    }
}
