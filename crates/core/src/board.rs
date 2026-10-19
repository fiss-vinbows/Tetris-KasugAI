//! 盤面(10列 × 40行のビットボード)。
//!
//! 1列を `u64` で持ち、下から y ビット目が (x, y) のマス。行単位の操作(ライン消去・おじゃまの
//! せり上がり)も、列ごとのビット演算で済む。高さ・穴などの評価項目もこの形のまま速く計算できる。

use crate::piece::PieceLocation;

pub const WIDTH: usize = 10;
pub const HEIGHT: usize = 40;
const FULL_COLUMN: u64 = (1 << HEIGHT) - 1;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Board {
    pub cols: [u64; WIDTH],
}

impl Board {
    pub const fn new() -> Self {
        Board { cols: [0; WIDTH] }
    }

    /// 文字列の行(上の行から順)から盤面を作る。`.` と空白は空き、それ以外は埋まり。
    /// 最後の行が y=0(最下段)になる。テストや局面の記述に使う。
    pub fn from_rows(rows: &[&str]) -> Self {
        let mut board = Board::new();
        for (i, row) in rows.iter().rev().enumerate() {
            assert!(i < HEIGHT, "行が多すぎる");
            let chars: Vec<char> = row.chars().collect();
            assert_eq!(chars.len(), WIDTH, "1行は10文字: {row:?}");
            for (x, c) in chars.into_iter().enumerate() {
                if c != '.' && c != ' ' {
                    board.cols[x] |= 1 << i;
                }
            }
        }
        board
    }

    /// 埋まっている一番上の行までを文字列にする(デバッグ表示用。上の行が先)。
    pub fn to_rows(&self) -> Vec<String> {
        let top = self.max_height();
        (0..top)
            .rev()
            .map(|y| (0..WIDTH).map(|x| if self.cols[x] >> y & 1 != 0 { '#' } else { '.' }).collect())
            .collect()
    }

    /// (x, y) が埋まっているか。盤面の外(左右の壁・床・上端より上)は埋まりとして扱う。
    #[inline]
    pub const fn occupied(&self, (x, y): (i8, i8)) -> bool {
        if x < 0 || x >= WIDTH as i8 || y < 0 || y >= HEIGHT as i8 {
            return true;
        }
        self.cols[x as usize] >> y & 1 != 0
    }

    /// ミノがこの位置に置けるか(4マスがすべて空いているか)。
    #[inline]
    pub fn fits(&self, loc: &PieceLocation) -> bool {
        loc.cells().iter().all(|&c| !self.occupied(c))
    }

    /// ミノがこの位置から真下へ何マス落ちられるか。
    pub fn drop_distance(&self, loc: &PieceLocation) -> i8 {
        loc.cells()
            .iter()
            .map(|&(x, y)| {
                // y より下で一番上の埋まりマスまでの距離
                let below = self.cols[x as usize] & ((1u64 << y) - 1);
                let ground = if below == 0 { -1 } else { 63 - below.leading_zeros() as i8 };
                y - ground - 1
            })
            .min()
            .unwrap()
    }

    /// 列の高さ(一番上の埋まりマスの y + 1。空なら 0)。
    #[inline]
    pub const fn column_height(&self, x: usize) -> u32 {
        64 - self.cols[x].leading_zeros()
    }

    pub fn max_height(&self) -> u32 {
        (0..WIDTH).map(|x| self.column_height(x)).max().unwrap()
    }

    pub fn place(&mut self, loc: &PieceLocation) {
        for (x, y) in loc.cells() {
            debug_assert!(!self.occupied((x, y)), "埋まっているマスに置こうとした: {loc:?}");
            self.cols[x as usize] |= 1 << y;
        }
    }

    /// そろっている行のビット列(y ビット目が立っていれば y 行目がそろっている)。
    #[inline]
    pub fn full_lines(&self) -> u64 {
        self.cols.iter().fold(FULL_COLUMN, |acc, &c| acc & c)
    }

    /// 指定した行を消し、上の行を下へ詰める。
    pub fn remove_lines(&mut self, lines: u64) {
        if lines == 0 {
            return;
        }
        for c in &mut self.cols {
            *c = remove_bits(*c, lines);
        }
    }

    /// そろった行を消し、消した行数を返す。
    pub fn clear_full_lines(&mut self) -> u32 {
        let lines = self.full_lines();
        self.remove_lines(lines);
        lines.count_ones()
    }

    pub fn is_empty(&self) -> bool {
        self.cols.iter().all(|&c| c == 0)
    }

    /// おじゃまを下から `lines` 行せり上げる(穴は `hole_x` 列)。
    /// 上端からあふれたマスがあれば true を返す(呼び出し側で負けの判定に使う)。
    pub fn add_garbage(&mut self, lines: u32, hole_x: usize) -> bool {
        if lines == 0 {
            return false;
        }
        assert!(hole_x < WIDTH);
        let lines = lines.min(HEIGHT as u32);
        let fill = (1u64 << lines) - 1;
        let mut overflow = false;
        for (x, c) in self.cols.iter_mut().enumerate() {
            let shifted = *c << lines;
            overflow |= shifted & !FULL_COLUMN != 0;
            *c = shifted & FULL_COLUMN;
            if x != hole_x {
                *c |= fill;
            }
        }
        overflow
    }
}

/// `col` から `lines` のビットを取り除き、上のビットを下へ詰める。
#[inline]
fn remove_bits(mut col: u64, mut lines: u64) -> u64 {
    // 上の行から消すと、下の行の位置がずれない
    while lines != 0 {
        let i = 63 - lines.leading_zeros();
        let low = col & ((1u64 << i) - 1);
        col = (col >> (i + 1) << i) | low;
        lines &= !(1u64 << i);
    }
    col
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::piece::{Piece, Rotation};

    #[test]
    fn 文字列との相互変換() {
        let rows = ["#.........", "##.......#", "##########"];
        let board = Board::from_rows(&rows);
        assert_eq!(board.to_rows(), rows);
        assert!(board.occupied((0, 0)));
        assert!(!board.occupied((1, 2)));
        assert!(board.occupied((-1, 5)), "左の壁は埋まり扱い");
        assert!(board.occupied((3, -1)), "床は埋まり扱い");
    }

    #[test]
    fn 離れた行のライン消去() {
        let mut board = Board::from_rows(&[
            "#.........",
            "##########",
            ".#........",
            "##########",
            "..#.......",
        ]);
        assert_eq!(board.clear_full_lines(), 2);
        assert_eq!(board.to_rows(), ["#.........", ".#........", "..#......."]);
    }

    #[test]
    fn 落下距離() {
        let board = Board::from_rows(&["..#.......", ".........."]);
        let t = PieceLocation::new(Piece::T, Rotation::North, 2, 10);
        // 中央下のマス(2,10)は(2,1)の上の(2,2)まで落ちられる
        assert_eq!(board.drop_distance(&t), 8);
        let t = PieceLocation::new(Piece::T, Rotation::North, 6, 10);
        assert_eq!(board.drop_distance(&t), 10);
    }

    #[test]
    fn おじゃまのせり上がり() {
        let mut board = Board::from_rows(&["#........."]);
        let overflow = board.add_garbage(2, 3);
        assert!(!overflow);
        assert_eq!(board.to_rows(), ["#.........", "###.######", "###.######"]);
    }

    #[test]
    fn おじゃまで上端からあふれる() {
        let mut board = Board::new();
        board.cols[0] = 1 << (HEIGHT - 1);
        assert!(board.add_garbage(1, 5));
    }
}
