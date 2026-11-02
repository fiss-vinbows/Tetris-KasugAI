//! 空中TSD の形を作れるかを総当たりで調べる(調べるための道具)。
//! C4W(種3 + 中あけ3行)の盤面に、S・Z・S を順に置くすべての置き方で、空中TSD の形(shape::has_air_tsd)になるものを数える。
//! 使い方(TetrisAI フォルダで): cargo run --release -p tetrisai-engine --example air_tsd_probe

use tetrisai_core::{generate, Board, Piece, Rules};
use tetrisai_engine::shape;

fn main() {
    let rules = Rules::ppt2();
    let args: Vec<String> = std::env::args().collect();
    let rows: Vec<&str> = args.get(1).map(|a| a.split(',').collect()).unwrap_or(vec!["###....###", "###....###", "###....###", "###.######"]);
    let start = Board::from_rows(&rows);
    let pieces: Vec<Piece> = args.get(2).map_or("SZS".to_string(), |s| s.clone()).chars().map(|c| Piece::from_char(c).unwrap()).collect();
    let mut found = 0;
    let mut shown = 0;
    let mut stack = vec![(start, 0usize)];
    while let Some((board, depth)) = stack.pop() {
        if depth == pieces.len() {
            continue;
        }
        for m in generate(&board, pieces[depth], rules.spawn()) {
            let mut b = board;
            b.place(&m.placement.location);
            if b.full_lines() != 0 {
                continue;
            }
            if shape::has_air_tsd(&b) > 0 && shape::ren_rows(&b) >= 2 {
                found += 1;
                if shown < 3 {
                    shown += 1;
                    println!("{} 手目で空中TSD の形:\n  {}", depth + 1, b.to_rows().join("\n  "));
                }
            }
            stack.push((b, depth + 1));
        }
    }
    println!("空中TSD の形になった置き方: {found}");
}
