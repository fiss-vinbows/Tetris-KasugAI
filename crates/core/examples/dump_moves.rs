//! 手の生成結果を文字で出力する(TBA との突き合わせ tools/crosscheck_tba.py から使う)。
//!
//! 入力(標準入力、1行1問): `<ミノ> <行/行/.../行>`(行は上から順、各10文字、`.` が空き)
//! 出力(1行1問): 配置を `|` で区切って並べる。配置は `x,y x,y x,y x,y;spin;止める回数;区間数`。
//! spin は None / Mini / Full。出現位置はぷよテト2(y=20)。

use std::io::{self, BufRead, Write};
use tetrisai_core::{generate, Board, Piece, Rules};

fn main() {
    let spawn = Rules::ppt2().spawn();
    let stdin = io::stdin();
    let mut out = io::BufWriter::new(io::stdout());
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        let mut parts = line.split_whitespace();
        let (Some(p), Some(rows)) = (parts.next(), parts.next()) else { continue };
        let piece = Piece::from_char(p.chars().next().unwrap()).expect("ミノの文字が不正");
        let rows: Vec<&str> = rows.split('/').collect();
        let board = Board::from_rows(&rows);
        let moves: Vec<String> = generate(&board, piece, spawn)
            .iter()
            .map(|m| {
                let cells: Vec<String> =
                    m.placement.location.sorted_cells().iter().map(|(x, y)| format!("{x},{y}")).collect();
                format!("{};{:?};{};{}", cells.join(" "), m.placement.spin, m.mid_air_stops, m.soft_drop_sections)
            })
            .collect();
        writeln!(out, "{}", moves.join("|")).unwrap();
    }
}
