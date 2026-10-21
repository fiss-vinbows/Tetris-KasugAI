//! 盤面の形の認識(M5): T溝(空中のものを含む)、中あけの行、中盤のパフェの見込み。
//!
//! どれも行ごとの「空きマスの並び」(10ビット)だけで判定するので速い(評価関数の中で毎回呼べる)。

use tetrisai_core::board::WIDTH;
use tetrisai_core::Board;

const ROW_FULL: u16 = (1 << WIDTH) - 1;
/// 中あけ4列(左から4〜7列目、x=3〜6)
const CENTER: u16 = 0b0001111000;
/// 中あけの左右(x=0〜2 と x=7〜9)
const SIDES: u16 = ROW_FULL & !CENTER;

/// y 行目の埋まりマス(x ビット目が立っていれば埋まり)。
#[inline]
fn row(board: &Board, y: u32) -> u16 {
    let mut r = 0u16;
    for (x, &c) in board.cols.iter().enumerate() {
        r |= ((c >> y & 1) as u16) << x;
    }
    r
}

/// TSD の溝(南向きの T を入れると2行消える形)の、T の中心 (x, y) の一覧。
///
/// 条件: y 行目の空きがちょうど x−1〜x+1 の3マス、y−1 行目の空きがちょうど x の1マス、
/// T の肩の上 (x−1, y+1) か (x+1, y+1) の少なくとも一方が埋まっている(屋根があるので回転で入れてTスピンになる)。
/// 盤面のどこにあってもよいので、中あけの上に渡したミノの上の溝(空中TSD)も見つかる。
/// 入れ方(屋根の下へ回転で届くか)までは確かめない。
pub fn tsd_slots(board: &Board) -> Vec<(i8, i8)> {
    let top = board.max_height();
    let mut slots = Vec::new();
    let mut below = row(board, 0);
    for y in 1..top {
        let here = row(board, y);
        let empty = !here & ROW_FULL;
        let empty_below = !below & ROW_FULL;
        if empty.count_ones() == 3 && empty_below.count_ones() == 1 {
            let x = empty_below.trailing_zeros() as i8;
            if x >= 1 && x <= WIDTH as i8 - 2 && empty == 0b111u16 << (x - 1) {
                let roof = board.occupied((x - 1, y as i8 + 1)) || board.occupied((x + 1, y as i8 + 1));
                if roof {
                    slots.push((x, y as i8));
                }
            }
        }
        below = here;
    }
    slots
}

/// 中あけの行(左右の3列ずつが埋まり、中央4列がすべて空いている行)の数。REN に使える行の目安。
pub fn center_well_rows(board: &Board) -> u32 {
    (0..board.max_height()).filter(|&y| row(board, y) == SIDES).count() as u32
}

/// 中あけの左右の高さの差(左3列の一番高いところと、右3列の一番高いところの差)。
pub fn center_well_side_diff(board: &Board) -> u32 {
    let left = (0..3).map(|x| board.column_height(x)).max().unwrap();
    let right = (7..WIDTH).map(|x| board.column_height(x)).max().unwrap();
    left.abs_diff(right)
}

/// 一番下の中あけの行より上に、TSD の溝があるか(空中TSD)。
pub fn has_air_tsd(board: &Board) -> usize {
    let Some(lowest_well) = (0..board.max_height()).find(|&y| row(board, y) == SIDES) else { return 0 };
    tsd_slots(board).iter().filter(|&&(_, y)| (y as u32) > lowest_well + 1).count()
}

/// 中盤のパフェの見込みがある形か。
///
/// パフェは「そろえる行の数 × 10 − 埋まりマス」がミノ4マスの倍数のときだけ取れる。10は偶数なので、
/// 埋まりマスが奇数ならどの高さでも取れない(ユーザーの方針: 奇数なら考えない)。
/// 偶数で、穴が無く、4段以内でそろえられる(4段まで積んで取れる)ときに true。
pub fn pc_possible(board: &Board) -> bool {
    let cells: u32 = board.cols.iter().map(|c| c.count_ones()).sum();
    if cells == 0 || cells % 2 == 1 {
        return false;
    }
    let h = board.max_height();
    if h > 4 {
        return false;
    }
    // 穴(上に埋まりマスがある空きマス)があれば取れない
    let holes: u32 = board.cols.iter().map(|&c| (!c & ((1u64 << (64 - c.leading_zeros())) - 1)).count_ones()).sum();
    if holes > 0 {
        return false;
    }
    (h.max(1)..=4).any(|rows| (rows * 10).checked_sub(cells).is_some_and(|rest| rest % 4 == 0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 普通のtsdの溝() {
        let board = Board::from_rows(&["##........", "#...######", "##.#######"]);
        assert_eq!(tsd_slots(&board), vec![(2, 1)]);
    }

    #[test]
    fn 屋根が無ければtスピンにならないので溝ではない() {
        let board = Board::from_rows(&["#...######", "##.#######"]);
        assert!(tsd_slots(&board).is_empty());
    }

    /// input/practice_20261019_214345_5397d0.png: 中あけの上(空中)の TSD の溝。
    fn air_board() -> Board {
        Board::from_rows(&[
            "..#....##.",
            ".##....###",
            "##...#####",
            "###.######",
            "###....###",
            "###....###",
        ])
    }

    #[test]
    fn 空中tsdの溝と中あけの行() {
        let board = air_board();
        assert_eq!(tsd_slots(&board), vec![(3, 3)]);
        assert_eq!(center_well_rows(&board), 2);
        assert_eq!(has_air_tsd(&board), 1);
        assert_eq!(center_well_side_diff(&board), 0, "左右とも6段");
    }

    #[test]
    fn パフェの見込み() {
        // 埋まりマスが奇数(9)ならどの高さでも取れない
        assert!(!pc_possible(&Board::from_rows(&["#########."])), "9マスは奇数");
        // 2段で10マス埋まっている(残り10マス)のは4の倍数にならないが、4段まで積めば30マスで取れる
        assert!(pc_possible(&Board::from_rows(&["#####.....", "#####....."])));
        // 1段に8マス(残り2)は1段では取れないが、2段なら残り12 = ミノ3つで取れる
        assert!(pc_possible(&Board::from_rows(&["########.."])));
        // 穴があれば取れない
        assert!(!pc_possible(&Board::from_rows(&["##########", "##.#######"])));
        // 5段以上は考えない
        assert!(!pc_possible(&Board::from_rows(&["##........"; 5])));
        assert!(!pc_possible(&Board::new()), "空の盤面はパフェ済み");
    }
}
