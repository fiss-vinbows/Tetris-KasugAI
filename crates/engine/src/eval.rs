//! 評価関数。
//!
//! 局面の点数 = これまでの手で得た「報酬」の合計 + 最後の盤面の「評価」。
//! - 報酬(reward): その手で起きたこと(ライン消去の種類・B2B・REN・パフェ・Tの無駄使い・操作の難しさ)
//! - 評価(eval): 盤面の形の良し悪し(穴・高さ・凸凹・井戸・T溝・B2B を保っているか)
//!
//! 項目名と既定値は Cold Clear 2 の default.json に合わせてある。CC2 用の設定 JSON
//! (TBA の config/cold_clear_beginner.json など)の `freestyle_weights` をそのまま読める。
//! 独自の項目(`soft_drop_section` など)は CC2 の設定には無いので既定値を使う。

use serde::{Deserialize, Serialize};
use tetrisai_core::board::WIDTH;
use tetrisai_core::{Board, ClearResult, Move, Piece, PieceLocation, Rotation, Rules, Spin};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct Weights {
    // ---- 盤面の評価 ----
    /// 穴の上に被さっているマスの数(穴ごとに `max_cell_covered_height` まで数える)
    pub cell_coveredness: f32,
    pub max_cell_covered_height: u32,
    pub holes: f32,
    /// 横方向に「埋まり↔空き」が切り替わる回数(凸凹の目安)
    pub row_transitions: f32,
    pub height: f32,
    /// 10段を超えた分
    pub height_upper_half: f32,
    /// 15段を超えた分
    pub height_upper_quarter: f32,
    /// 一番低い列を井戸とみなしたときの、テトリスで消せる深さ
    pub tetris_well_depth: f32,
    /// T溝に T を入れたときに消える行数(0〜3)ごとの点
    pub tslot: [f32; 4],
    pub has_back_to_back: f32,

    // ---- 手の報酬 ----
    /// T を TSD/TST 以外で使った
    pub wasted_t: f32,
    /// CC2 の「ソフトドロップのマス数」への減点。こちらでは使わない(読み込めるように残している)
    pub softdrop: f32,
    /// 消した行数(0〜4)ごとの点(Tスピンでない場合)
    pub normal_clears: [f32; 5],
    pub mini_spin_clears: [f32; 3],
    pub spin_clears: [f32; 4],
    pub back_to_back_clear: f32,
    /// REN の攻撃量(ルール設定の REN 表)1行あたりの点
    pub combo_attack: f32,
    pub perfect_clear: f32,
    /// パフェのときは消し方の点を数えない
    pub perfect_clear_override: bool,

    // ---- 独自の項目 ----
    /// ソフトドロップの区間数1つあたり(TBA の難度 ◎○△ と同じ数え方)
    pub soft_drop_section: f32,
    /// ソフトドロップを接地前に止める回数1つあたり
    pub mid_air_stop: f32,
}

impl Default for Weights {
    fn default() -> Self {
        Weights {
            cell_coveredness: -0.2,
            max_cell_covered_height: 6,
            holes: -1.5,
            row_transitions: -0.2,
            height: -0.4,
            height_upper_half: -1.5,
            height_upper_quarter: -5.0,
            tetris_well_depth: 0.3,
            tslot: [0.1, 1.5, 2.0, 4.0],
            has_back_to_back: 0.5,
            wasted_t: -1.5,
            softdrop: -0.2,
            normal_clears: [0.0, -2.0, -1.5, -1.0, 3.5],
            mini_spin_clears: [0.0, -1.5, -1.0],
            spin_clears: [0.0, 1.0, 4.0, 6.0],
            back_to_back_clear: 1.0,
            combo_attack: 1.5,
            perfect_clear: 15.0,
            perfect_clear_override: true,
            soft_drop_section: -0.5,
            mid_air_stop: -1.5,
        }
    }
}

/// 手の報酬。`mv` は置いた手(操作の難しさを含む)、`result` はその結果。
pub fn reward(w: &Weights, rules: &Rules, mv: &Move, result: &ClearResult) -> f32 {
    let mut r = 0.0;
    if result.perfect_clear {
        r += w.perfect_clear;
    }
    if !result.perfect_clear || !w.perfect_clear_override {
        if result.back_to_back {
            r += w.back_to_back_clear;
        }
        let lines = result.lines as usize;
        r += match result.spin {
            Spin::None => w.normal_clears[lines.min(4)],
            Spin::Mini => w.mini_spin_clears[lines.min(2)],
            Spin::Full => w.spin_clears[lines.min(3)],
        };
        if result.lines > 0 {
            r += w.combo_attack * rules.combo_attack(result.ren) as f32;
        }
    }
    if mv.placement.location.piece == Piece::T && (result.lines < 2 || result.spin != Spin::Full) {
        r += w.wasted_t;
    }
    r += w.soft_drop_section * mv.soft_drop_sections as f32;
    r += w.mid_air_stop * mv.mid_air_stops as f32;
    r
}

/// 盤面の評価。`t_available` は今後使える T の目安(T溝の評価を何回まで重ねるか)。
pub fn evaluate(w: &Weights, board: &Board, back_to_back: bool, t_available: usize) -> f32 {
    let mut e = 0.0;
    if back_to_back {
        e += w.has_back_to_back;
    }

    // T溝: 見つかった溝に T を入れたと仮定し、消える行数で点を付ける(2行以上消えるなら消して次を探す)
    let mut b = *board;
    for _ in 0..t_available {
        let Some(loc) = tslot_left(&b).or_else(|| tslot_right(&b)) else { break };
        let mut filled = b;
        filled.place(&loc);
        let lines = filled.full_lines().count_ones() as usize;
        e += w.tslot[lines.min(3)];
        if lines > 1 {
            filled.remove_lines(filled.full_lines());
            b = filled;
        }
    }

    // 穴と、穴の上に被さったマス
    let mut holes = 0;
    let mut coveredness = 0;
    for &c in &board.cols {
        let height = 64 - c.leading_zeros();
        let mut h = !c & ((1u64 << height) - 1);
        holes += h.count_ones();
        while h != 0 {
            let y = h.trailing_zeros();
            coveredness += (height - y).min(w.max_cell_covered_height);
            h &= h - 1;
        }
    }
    e += w.holes * holes as f32;
    e += w.cell_coveredness * coveredness as f32;

    // テトリスの井戸の深さ
    let (well_x, well_h) = (0..WIDTH).map(|x| (x, board.column_height(x))).min_by_key(|&(_, h)| h).unwrap();
    let full_except_well =
        board.cols.iter().enumerate().filter(|&(x, _)| x != well_x).fold(!0u64, |acc, (_, &c)| acc & c);
    let depth = (full_except_well >> well_h).trailing_ones();
    e += w.tetris_well_depth * depth as f32;

    // 高さ
    let top = board.max_height();
    e += w.height * top as f32;
    if top > 10 {
        e += w.height_upper_half * (top - 10) as f32;
    }
    if top > 15 {
        e += w.height_upper_quarter * (top - 15) as f32;
    }

    // 行方向の切り替わり(左右の壁は埋まり扱い)
    let full = (1u64 << tetrisai_core::board::HEIGHT) - 1;
    let mut transitions = (full ^ board.cols[0]).count_ones() + (full ^ board.cols[WIDTH - 1]).count_ones();
    for pair in board.cols.windows(2) {
        transitions += (pair[0] ^ pair[1]).count_ones();
    }
    e += w.row_transitions * transitions as f32;

    e
}

/// 左側に屋根がある T溝(CC2 の well_known_tslot_left と同じ)。
fn tslot_left(board: &Board) -> Option<PieceLocation> {
    for x in 0..WIDTH - 2 {
        let y = board.column_height(x) as i8;
        if board.column_height(x + 1) as i8 >= y {
            continue;
        }
        let x = x as i8;
        if !board.occupied((x + 2, y - 1)) || board.occupied((x + 2, y)) || !board.occupied((x + 2, y + 1)) {
            continue;
        }
        return Some(PieceLocation::new(Piece::T, Rotation::South, x + 1, y));
    }
    None
}

/// 右側に屋根がある T溝(CC2 の well_known_tslot_right と同じ)。
fn tslot_right(board: &Board) -> Option<PieceLocation> {
    for x in 0..WIDTH - 2 {
        let y = board.column_height(x + 2) as i8;
        if board.column_height(x + 1) as i8 >= y {
            continue;
        }
        let x = x as i8;
        if !board.occupied((x, y - 1)) || board.occupied((x, y)) || !board.occupied((x, y + 1)) {
            continue;
        }
        return Some(PieceLocation::new(Piece::T, Rotation::South, x + 1, y));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 穴があると評価が下がる() {
        let w = Weights::default();
        let flat = Board::from_rows(&["#########."]);
        let hole = Board::from_rows(&["#########.", "####.#####"]);
        assert!(evaluate(&w, &hole, false, 1) < evaluate(&w, &flat, false, 1));
    }

    #[test]
    fn t溝を見つける() {
        // 左に屋根のある TSD の溝
        let board = Board::from_rows(&[
            "##........",
            "#...######",
            "##.#######",
        ]);
        let loc = tslot_left(&board).or_else(|| tslot_right(&board));
        assert!(loc.is_some());
        let w = Weights::default();
        let with = evaluate(&w, &board, false, 1);
        let without = evaluate(&w, &board, false, 0);
        assert!((with - without - w.tslot[2]).abs() < 1e-5, "TSD の溝は tslot[2] の点");
    }

    #[test]
    fn cc2の設定jsonの重みを読める() {
        // TBA の config/cold_clear_beginner.json と同じ形(独自の項目は既定値になる)
        let json = r#"{"holes": -2.0, "tslot": [0.0, 1.0, 2.0, 3.0], "mini_spin_clears": [0.0, 0.0, 0.0]}"#;
        let w: Weights = serde_json::from_str(json).unwrap();
        assert_eq!(w.holes, -2.0);
        assert_eq!(w.tslot, [0.0, 1.0, 2.0, 3.0]);
        assert_eq!(w.soft_drop_section, Weights::default().soft_drop_section);
    }
}
