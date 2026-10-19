//! SRS(スーパーローテーションシステム)の回転と、Tスピンの判定。
//!
//! ぷよテト2 は通常の SRS(180度回転なし)。キック表は「向きごとのオフセット表」の差として
//! 求める(Cold Clear 2 と同じ方式)。ミノの中心の取り方が TBP 準拠のため、I ミノの
//! オフセットはガイドラインのキック表と見かけの数値が違うが、回転後の4マスは同じになる
//! (TBA の SRS 実装との突き合わせで確認する: tools/crosscheck)。

use crate::board::Board;
use crate::piece::{Piece, PieceLocation, Rotation};
use serde::{Deserialize, Serialize};

/// Tスピンの種類。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Spin {
    #[default]
    None,
    Mini,
    Full,
}

const fn offsets(piece: Piece, rotation: Rotation) -> [(i8, i8); 5] {
    match piece {
        Piece::O => match rotation {
            Rotation::North => [(0, 0); 5],
            Rotation::East => [(0, -1); 5],
            Rotation::South => [(-1, -1); 5],
            Rotation::West => [(-1, 0); 5],
        },
        Piece::I => match rotation {
            Rotation::North => [(0, 0), (-1, 0), (2, 0), (-1, 0), (2, 0)],
            Rotation::East => [(-1, 0), (0, 0), (0, 0), (0, 1), (0, -2)],
            Rotation::South => [(-1, 1), (1, 1), (-2, 1), (1, 0), (-2, 0)],
            Rotation::West => [(0, 1), (0, 1), (0, 1), (0, -1), (0, 2)],
        },
        _ => match rotation {
            Rotation::North => [(0, 0); 5],
            Rotation::East => [(0, 0), (1, 0), (1, -1), (0, 2), (1, 2)],
            Rotation::South => [(0, 0); 5],
            Rotation::West => [(0, 0), (-1, 0), (-1, -1), (0, 2), (-1, 2)],
        },
    }
}

/// `from` から `to` へ回すときに試す5つのずらし(試す順)。
pub const fn kicks(piece: Piece, from: Rotation, to: Rotation) -> [(i8, i8); 5] {
    let a = offsets(piece, from);
    let b = offsets(piece, to);
    let mut k = [(0, 0); 5];
    let mut i = 0;
    while i < 5 {
        k[i] = (a[i].0 - b[i].0, a[i].1 - b[i].1);
        i += 1;
    }
    k
}

/// 回転の結果。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rotated {
    pub location: PieceLocation,
    /// 何番目のずらしで入ったか(0〜4)。
    pub kick_index: u8,
    /// この回転で固定したときのTスピンの種類(T以外は常に None)。
    pub spin: Spin,
}

/// 回転を試す。`clockwise` が false なら左回転。O は回転しても位置が変わらないので None。
pub fn rotate(board: &Board, from: &PieceLocation, clockwise: bool) -> Option<Rotated> {
    if from.piece == Piece::O {
        return None;
    }
    let to = if clockwise { from.rotation.cw() } else { from.rotation.ccw() };
    for (i, (dx, dy)) in kicks(from.piece, from.rotation, to).into_iter().enumerate() {
        let target = PieceLocation { rotation: to, x: from.x + dx, y: from.y + dy, ..*from };
        if board.fits(&target) {
            let spin = if target.piece == Piece::T { t_spin(board, &target, i as u8) } else { Spin::None };
            return Some(Rotated { location: target, kick_index: i as u8, spin });
        }
    }
    None
}

/// Tスピンの判定(3コーナー規則)。回転で `loc` に入った直後に呼ぶ。
///
/// - Tの中心の斜め4マスのうち3つ以上が埋まっていればTスピン
/// - Tの出っ張り側(前)の2マスが両方埋まっていれば Full、そうでなければ Mini
/// - ただし最後のずらし(5番目、TST の入れ方)で入った場合は Full
///
/// ぷよテト2 もガイドラインどおりこの規則と考えている(要実機確認: 設計書4章)。
pub fn t_spin(board: &Board, loc: &PieceLocation, kick_index: u8) -> Spin {
    debug_assert_eq!(loc.piece, Piece::T);
    let occupied = |(dx, dy): (i8, i8)| board.occupied((loc.x + dx, loc.y + dy));
    let corners = [(-1, -1), (1, -1), (-1, 1), (1, 1)].into_iter().filter(|&c| occupied(c)).count();
    if corners < 3 {
        return Spin::None;
    }
    let front = [(-1, 1), (1, 1)].into_iter().map(|c| loc.rotation.rotate_cell(c)).filter(|&c| occupied(c)).count();
    if front == 2 || kick_index == 4 {
        Spin::Full
    } else {
        Spin::Mini
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ガイドラインのキック表(TBA の rules.py と同じ。縦は上向きが正)。
    /// JLSTZ は中心の取り方が同じなので数値がそのまま一致する。
    #[test]
    fn jlstzのキック表はガイドラインと一致() {
        let n = Rotation::North;
        let e = Rotation::East;
        let s = Rotation::South;
        let w = Rotation::West;
        let table = [
            (n, e, [(0, 0), (-1, 0), (-1, 1), (0, -2), (-1, -2)]),
            (e, n, [(0, 0), (1, 0), (1, -1), (0, 2), (1, 2)]),
            (e, s, [(0, 0), (1, 0), (1, -1), (0, 2), (1, 2)]),
            (s, e, [(0, 0), (-1, 0), (-1, 1), (0, -2), (-1, -2)]),
            (s, w, [(0, 0), (1, 0), (1, 1), (0, -2), (1, -2)]),
            (w, s, [(0, 0), (-1, 0), (-1, -1), (0, 2), (-1, 2)]),
            (w, n, [(0, 0), (-1, 0), (-1, -1), (0, 2), (-1, 2)]),
            (n, w, [(0, 0), (1, 0), (1, 1), (0, -2), (1, -2)]),
        ];
        for (from, to, expected) in table {
            assert_eq!(kicks(Piece::T, from, to), expected, "{from:?}→{to:?}");
        }
    }

    /// I は中心の取り方が違うので、ガイドラインの表で回した4マスと一致するかで確かめる。
    #[test]
    fn iのキックはガイドラインと同じ4マスになる() {
        // ガイドラインの I の形(4×4 の枠内、行は上から、TBA の rules.py と同じ)
        let shapes: [[(i8, i8); 4]; 4] = [
            [(1, 0), (1, 1), (1, 2), (1, 3)],
            [(0, 2), (1, 2), (2, 2), (3, 2)],
            [(2, 0), (2, 1), (2, 2), (2, 3)],
            [(0, 1), (1, 1), (2, 1), (3, 1)],
        ];
        // ガイドラインの I のキック表(縦は上向きが正)
        let guideline = |from: usize, to: usize| -> [(i8, i8); 5] {
            match (from, to) {
                (0, 1) => [(0, 0), (-2, 0), (1, 0), (-2, -1), (1, 2)],
                (1, 0) => [(0, 0), (2, 0), (-1, 0), (2, 1), (-1, -2)],
                (1, 2) => [(0, 0), (-1, 0), (2, 0), (-1, 2), (2, -1)],
                (2, 1) => [(0, 0), (1, 0), (-2, 0), (1, -2), (-2, 1)],
                (2, 3) => [(0, 0), (2, 0), (-1, 0), (2, 1), (-1, -2)],
                (3, 2) => [(0, 0), (-2, 0), (1, 0), (-2, -1), (1, 2)],
                (3, 0) => [(0, 0), (1, 0), (-2, 0), (1, -2), (-2, 1)],
                (0, 3) => [(0, 0), (-1, 0), (2, 0), (-1, 2), (2, -1)],
                _ => unreachable!(),
            }
        };
        // 枠の左上を (col0, row0) に置いたときの盤面座標(y は上向き)
        let guideline_cells = |orient: usize, col0: i8, top: i8| -> Vec<(i8, i8)> {
            let mut v: Vec<(i8, i8)> = shapes[orient].iter().map(|&(r, c)| (col0 + c, top - r)).collect();
            v.sort_by_key(|&(x, y)| (y, x));
            v
        };
        let rots = Rotation::ALL;
        for from in 0..4 {
            for to in [(from + 1) % 4, (from + 3) % 4] {
                // 回転前の位置が同じ4マスになるよう、TBP 形式の位置を探す
                let (col0, top) = (3, 20);
                let before = guideline_cells(from, col0, top);
                let loc = (-5..15)
                    .flat_map(|x| (-5..45).map(move |y| (x, y)))
                    .map(|(x, y)| PieceLocation::new(Piece::I, rots[from], x, y))
                    .find(|l| l.sorted_cells().to_vec() == before)
                    .unwrap();
                let ours = kicks(Piece::I, rots[from], rots[to]);
                let theirs = guideline(from, to);
                for i in 0..5 {
                    let moved = PieceLocation { rotation: rots[to], x: loc.x + ours[i].0, y: loc.y + ours[i].1, ..loc };
                    let expected = guideline_cells(to, col0 + theirs[i].0, top + theirs[i].1);
                    assert_eq!(moved.sorted_cells().to_vec(), expected, "{from}→{to} の {i} 番目");
                }
            }
        }
    }

    #[test]
    fn tsdの判定() {
        // T溝(出っ張りが下)。北向きのTを右回転→南向きで溝に入れる場面を作る
        let board = Board::from_rows(&[
            "##........",
            "#...######",
            "##.#######",
        ]);
        let t = PieceLocation::new(Piece::T, Rotation::South, 2, 1);
        assert!(board.fits(&t));
        assert_eq!(t_spin(&board, &t, 0), Spin::Full);
    }

    #[test]
    fn ミニの判定() {
        // 前側の角が1つしか埋まっていない
        let board = Board::from_rows(&[
            "..........",
            "#.........",
            "#..#######",
        ]);
        // 東向きのT(出っ張りが右): (1,1),(1,0),(1,2),(2,1)
        let t = PieceLocation::new(Piece::T, Rotation::East, 1, 1);
        assert!(board.fits(&t));
        // 斜め: (0,0)埋, (2,0)空, (0,2)空, (2,2)空 → 3つ未満
        assert_eq!(t_spin(&board, &t, 0), Spin::None);
        let board = Board::from_rows(&[
            "#.........",
            "#.........",
            "#..#######",
        ]);
        // 斜め: (0,0)埋, (2,0)空, (0,2)埋, 床側…3つ未満なので None のまま
        assert_eq!(t_spin(&board, &t, 0), Spin::None);
        // 床に接した北向きT: 下の角2つは床(埋まり扱い)、左上だけ埋まり → 前側は1つ → Mini
        let board = Board::from_rows(&["#.........", ".........."]);
        let t = PieceLocation::new(Piece::T, Rotation::North, 1, 0);
        assert_eq!(t_spin(&board, &t, 0), Spin::Mini);
        // 同じ形でも5番目のずらしで入ったなら Full
        assert_eq!(t_spin(&board, &t, 4), Spin::Full);
    }
}
