//! 手の生成の速さを測る(探索 M2 の設計の目安にする)。
//! 使い方: cargo run --release --example bench_movegen

use std::time::Instant;
use tetrisai_core::{generate, Board, Piece, Rules};

fn main() {
    let spawn = Rules::ppt2().spawn();
    // 中盤らしい凸凹の盤面(input/ の空中TSDの局面)と、空の盤面
    let boards = [
        ("空", Board::new()),
        (
            "空中TSD",
            Board::from_rows(&["..#....##.", ".##....###", "##...#####", "###.######", "###....###", "###....###"]),
        ),
    ];
    for (name, board) in boards {
        let n = 20_000;
        let start = Instant::now();
        let mut total = 0;
        for i in 0..n {
            total += generate(&board, Piece::ALL[i % 7], spawn).len();
        }
        let per = start.elapsed().as_secs_f64() / n as f64 * 1e6;
        println!("{name}: 1回 {per:.1} µs(配置 平均 {:.1} 個)", total as f64 / n as f64);
    }
}
