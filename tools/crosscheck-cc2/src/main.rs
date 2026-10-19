//! ランダムな盤面で、Cold Clear 2 と tetrisai-core の手の生成結果(配置 + Tスピンの種類)を突き合わせる。
//!
//! 途中で止める手(mid_air_stops > 0)は CC2 が作らないので比較から外す。
//!
//! 使い方: cargo run --release --manifest-path tools/crosscheck-cc2/Cargo.toml -- [盤面数] [乱数の種] [tall]
//!
//! - 既定: 0〜18段のランダムな盤面。CC2 の高速モード(16段以下)の簡略化で、こちらだけにある手が出る。
//!   CC2 だけにある手が無いこと(CC2 の手はすべてこちらにもあること)を確かめる。
//! - tall: 17〜18段の盤面だけ。CC2 が高速モードを使わず全探索するので、完全に一致することを確かめる。
//!
//! CC2 は出現位置が y=19 なので、こちらも y=19 にそろえて比べる(ぷよテト2 の y=20 とは違う)。
//! 出現位置の差が効かないよう、盤面の高さは18段までにする。

use std::collections::BTreeSet;

use cold_clear_2::data as cc;
use rand::rngs::StdRng;
use rand::{Rng, SeedableRng};
use tetrisai_core::{generate, Board, Piece, PieceLocation, Rotation, Spawn, Spin};

const SPAWN: Spawn = Spawn { x: 4, y: 19 };
const MAX_HEIGHT: u32 = 18;

type Key = (Piece, Rotation, i8, i8, Spin);

fn to_cc_piece(p: Piece) -> cc::Piece {
    match p {
        Piece::I => cc::Piece::I,
        Piece::O => cc::Piece::O,
        Piece::T => cc::Piece::T,
        Piece::L => cc::Piece::L,
        Piece::J => cc::Piece::J,
        Piece::S => cc::Piece::S,
        Piece::Z => cc::Piece::Z,
    }
}

fn from_cc(p: &cc::Placement) -> Key {
    let piece = match p.location.piece {
        cc::Piece::I => Piece::I,
        cc::Piece::O => Piece::O,
        cc::Piece::T => Piece::T,
        cc::Piece::L => Piece::L,
        cc::Piece::J => Piece::J,
        cc::Piece::S => Piece::S,
        cc::Piece::Z => Piece::Z,
    };
    let rotation = match p.location.rotation {
        cc::Rotation::North => Rotation::North,
        cc::Rotation::East => Rotation::East,
        cc::Rotation::South => Rotation::South,
        cc::Rotation::West => Rotation::West,
    };
    let spin = match p.spin {
        cc::Spin::None => Spin::None,
        cc::Spin::Mini => Spin::Mini,
        cc::Spin::Full => Spin::Full,
    };
    (piece, rotation, p.location.x, p.location.y, spin)
}

/// CC2 はソフトドロップを常に底まで落とす前提なので、途中で止めない手だけを比べる。
fn ours(board: &Board, piece: Piece) -> BTreeSet<Key> {
    generate(board, piece, SPAWN)
        .into_iter()
        .filter(|m| m.mid_air_stops == 0)
        .map(|m| {
            let l = m.placement.location;
            (l.piece, l.rotation, l.x, l.y, m.placement.spin)
        })
        .collect()
}

fn theirs(board: &Board, piece: Piece) -> BTreeSet<Key> {
    let cc_board = cc::Board { cols: board.cols };
    cold_clear_2::movegen::find_moves(&cc_board, to_cc_piece(piece))
        .iter()
        .map(|(p, _)| from_cc(p))
        .collect()
}

/// ミノをランダムに積み、ところどころ穴・張り出しを作った盤面。
fn random_board(rng: &mut StdRng, min_height: u32) -> Board {
    loop {
        let mut board = Board::new();
        let pieces = rng.gen_range(0..40);
        for _ in 0..pieces {
            let piece = Piece::ALL[rng.gen_range(0..7)];
            let moves = generate(&board, piece, SPAWN);
            if moves.is_empty() {
                break;
            }
            let m = moves[rng.gen_range(0..moves.len())];
            board.place(&m.placement.location);
            board.clear_full_lines();
        }
        // ランダムなマスを抜いたり足したりして、Tスピンの溝や張り出しを作りやすくする
        let noise = rng.gen_range(0..12);
        for _ in 0..noise {
            let x = rng.gen_range(0..10);
            let h = board.column_height(x).max(1);
            let y = rng.gen_range(0..h + 1);
            board.cols[x] ^= 1 << y;
        }
        board.clear_full_lines();
        if (min_height..=MAX_HEIGHT).contains(&board.max_height()) {
            return board;
        }
    }
}

fn show(key: &Key) -> String {
    let loc = PieceLocation::new(key.0, key.1, key.2, key.3);
    format!("{:?} {:?} ({},{}) {:?} マス={:?}", key.0, key.1, key.2, key.3, key.4, loc.sorted_cells())
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let boards: usize = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(2000);
    let seed: u64 = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(1);
    let tall = args.get(3).map(|s| s == "tall").unwrap_or(false);
    let min_height = if tall { 17 } else { 0 };
    let mut rng = StdRng::seed_from_u64(seed);

    let mut checked = 0usize;
    let mut placements = 0usize;
    let mut spins = 0usize;
    let mut known = 0usize;
    let mut mismatches = 0usize;
    for _ in 0..boards {
        let board = random_board(&mut rng, min_height);
        for piece in Piece::ALL {
            let a = ours(&board, piece);
            let b = theirs(&board, piece);
            checked += 1;
            placements += a.len();
            spins += a.iter().filter(|k| k.4 != Spin::None).count();
            // 高速モードの盤面では、こちらだけにある手は既知の違いとして数える
            let fast_mode = (0..10).all(|x| board.column_height(x) <= 16);
            let only_ours: Vec<_> = a.difference(&b).filter(|_| !fast_mode).collect();
            let only_theirs: Vec<_> = b.difference(&a).collect();
            known += a.difference(&b).count() - only_ours.len();
            if !only_ours.is_empty() || !only_theirs.is_empty() {
                mismatches += 1;
                if mismatches <= 5 {
                    println!("不一致: {piece:?}");
                    for row in board.to_rows() {
                        println!("  {row}");
                    }
                    for k in only_ours {
                        println!("  こちらだけ: {}", show(k));
                    }
                    for k in only_theirs {
                        println!("  CC2だけ:   {}", show(k));
                    }
                }
            }
        }
    }
    println!("盤面 {boards} 個 × 7種 = {checked} 回、配置 {placements} 個(うちTスピン {spins} 個)を比較");
    println!("既知の違い(CC2 の高速モードが積みより上の位置を探さないため、こちらだけにある手): {known} 個");
    if mismatches == 0 {
        println!("既知の違いを除いてすべて一致");
    } else {
        println!("不一致 {mismatches} 回");
        std::process::exit(1);
    }
}
