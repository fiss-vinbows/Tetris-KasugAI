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

use crate::shape;
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
    /// まだ残っている予告1行あたり
    pub pending_garbage: f32,
    /// 予告をすべて受けたときに20段を超える分(1段あたり)
    pub overflow: f32,

    // ---- 対戦の駆け引き(M4。相手の情報があるときだけ効く。`attack_sent` は常に効く) ----
    /// 相手に届く攻撃(相殺に使った分を除く)1行あたり
    pub attack_sent: f32,
    /// 相手に届く攻撃1行あたり × 相手の危なさ(相手の高さ + 相手への予告、20段で1)
    pub attack_sent_danger: f32,
    /// 届く攻撃(攻撃ゲージを含む)が相手の残りの余裕(20段 − 相手の危なさ)以上になる(とどめ)
    pub lethal: f32,
    /// 自分の高さ(予告を含む)+ 相手が次に撃てる攻撃の見積もり が20段を超える分(1段あたり)
    pub threat_overflow: f32,

    // ---- 中盤の作戦(M5。既定値はすべて0で効かない。試験用の重みは config/weights/experimental_m5.toml) ----
    /// 中あけの行(左右3列ずつが埋まり中央4列が空いた行)1行あたり。中あけ積みのときだけ効く
    pub center_well_rows: f32,
    /// 中あけの左右の高さの差1段あたり。中あけ積みのときだけ効く
    pub center_well_side_diff: f32,
    /// 中あけの上(空中)に TSD の溝がある。中あけ積みのときだけ効く
    pub air_tsd_slot: f32,
    /// 中あけ積みを始める高さ(探索の出発点の盤面がこの段数以下か、すでに中あけが2行以上あれば中あけ積み)
    pub center_well_start_height: f32,
    /// TSD の溝(空中のものを含む、一般的な形)1つあたり(2つまで)。`tslot` は CC2 と同じ単純な形しか見ない
    pub tsd_slot_any: f32,
    /// 中盤のパフェの見込みがある形(埋まりマスが偶数・穴なし・4段以内でそろえられる)
    pub pc_chance: f32,
    /// ソフトドロップで落とした段数1段あたり(低い盤面での Tスピンは落とす時間が長く効率が悪い)
    pub soft_drop_rows: f32,
}

/// 盤面の評価に渡す、局面ごとの値。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EvalContext {
    /// 今後使える T の目安(T溝の評価を何回まで重ねるか)
    pub t_available: usize,
    /// 予告の影響(高さに上乗せする)
    pub garbage: Garbage,
    /// 中あけ積みの作戦をとっているか
    pub center_well: bool,
}

/// 相手の情報から作る、評価に使う値(探索の最初に1回だけ計算する)。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BattleContext {
    /// 相手の危なさ(相手の盤面の高さ + 相手への予告の行数)。相手の情報が無ければ None
    pub opponent_danger: Option<u32>,
    /// 相手が次に撃てる攻撃の見積もり(行数)
    pub opponent_threat: u32,
    /// 自分の攻撃ゲージ(REN 中にためている攻撃。次にラインを消さない手を置くと相手へ送られる)
    pub gauge: u32,
}

/// 予告の影響(探索の中で計算する)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Garbage {
    /// 盤面の高さに上乗せする行数(せり上がったとみなした分 + まだ残っている予告)
    pub extra_height: u32,
    /// まだ残っている予告の行数
    pub pending: u32,
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
            pending_garbage: -0.1,
            overflow: -10.0,
            attack_sent: 0.0,
            attack_sent_danger: 0.5,
            lethal: 20.0,
            threat_overflow: -2.0,
            center_well_rows: 0.0,
            center_well_side_diff: 0.0,
            air_tsd_slot: 0.0,
            center_well_start_height: 5.0,
            tsd_slot_any: 0.0,
            pc_chance: 0.0,
            soft_drop_rows: 0.0,
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
    if mv.soft_drop_sections > 0 {
        // ソフトドロップで落とした段数の目安: 出現位置の高さから置いた位置まで
        let rows = (rules.spawn().y - mv.placement.location.y).max(0);
        r += w.soft_drop_rows * rows as f32;
    }
    r
}

/// 中あけ積みの作戦をとるか(探索の出発点の盤面で1回だけ決める)。中あけの重みが0なら、とらない。
pub fn center_well_mode(w: &Weights, board: &Board) -> bool {
    w.center_well_rows != 0.0
        && (board.max_height() as f32 <= w.center_well_start_height || shape::center_well_rows(board) >= 2)
}

/// 盤面の評価。
pub fn evaluate(w: &Weights, board: &Board, back_to_back: bool, ctx: &EvalContext) -> f32 {
    let (t_available, garbage) = (ctx.t_available, ctx.garbage);
    let mut e = 0.0;
    if back_to_back {
        e += w.has_back_to_back;
    }

    // 中盤の作戦(M5)
    if w.tsd_slot_any != 0.0 {
        e += w.tsd_slot_any * shape::tsd_slots(board).len().min(2) as f32;
    }
    if ctx.center_well {
        e += w.center_well_rows * shape::center_well_rows(board) as f32;
        e += w.center_well_side_diff * shape::center_well_side_diff(board) as f32;
        e += w.air_tsd_slot * shape::has_air_tsd(board).min(1) as f32;
    }
    if w.pc_chance != 0.0 && shape::pc_possible(board) {
        e += w.pc_chance;
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

    // 高さ(予告の分を上乗せ)
    e += w.pending_garbage * garbage.pending as f32;
    let top = board.max_height() + garbage.extra_height;
    if top > 20 {
        e += w.overflow * (top - 20) as f32;
    }
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

/// 対戦の駆け引きの評価。`outgoing` はこの局面までに相手に届く攻撃(相殺に使った分を除く)、
/// `our_top` は自分の盤面の高さ(予告の分を上乗せしたもの)。
pub fn battle_value(w: &Weights, ctx: &BattleContext, outgoing: u32, our_top: u32) -> f32 {
    let mut v = w.attack_sent * outgoing as f32;
    if let Some(danger) = ctx.opponent_danger {
        v += w.attack_sent_danger * outgoing as f32 * (danger as f32 / 20.0);
        let margin = 20u32.saturating_sub(danger);
        if outgoing > 0 && ctx.gauge + outgoing >= margin {
            v += w.lethal;
        }
        let threat_top = our_top + ctx.opponent_threat;
        if threat_top > 20 {
            v += w.threat_overflow * (threat_top - 20) as f32;
        }
    }
    v
}

/// 相手が次に撃てる攻撃の見積もり(行数)。相手の盤面の T溝(TSS/TSD/TST)とテトリスの井戸のうち
/// 大きいほうに、相手の攻撃ゲージと B2B の加算を足す。
pub fn opponent_threat(rules: &Rules, board: &Board, gauge: u32, back_to_back: bool) -> u32 {
    let mut best = 0;
    if let Some(loc) = tslot_left(board).or_else(|| tslot_right(board)) {
        let mut filled = *board;
        filled.place(&loc);
        let lines = filled.full_lines().count_ones();
        best = best.max(rules.line_attack(lines, Spin::Full));
    }
    // テトリスの井戸: 一番低い列以外がそろっている深さが4以上
    let (well_x, well_h) = (0..WIDTH).map(|x| (x, board.column_height(x))).min_by_key(|&(_, h)| h).unwrap();
    let full_except_well =
        board.cols.iter().enumerate().filter(|&(x, _)| x != well_x).fold(!0u64, |acc, (_, &c)| acc & c);
    if (full_except_well >> well_h).trailing_ones() >= 4 {
        best = best.max(rules.line_attack(4, Spin::None));
    }
    if best > 0 && back_to_back {
        best += rules.attack.back_to_back_bonus;
    }
    best + gauge
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
        let ctx = EvalContext { t_available: 1, ..Default::default() };
        assert!(evaluate(&w, &hole, false, &ctx) < evaluate(&w, &flat, false, &ctx));
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
        let with = evaluate(&w, &board, false, &EvalContext { t_available: 1, ..Default::default() });
        let without = evaluate(&w, &board, false, &EvalContext::default());
        assert!((with - without - w.tslot[2]).abs() < 1e-5, "TSD の溝は tslot[2] の点");
    }

    #[test]
    fn 相手の攻撃の準備を見積もる() {
        let rules = Rules::ppt2();
        // TSD の溝がある盤面
        let tsd = Board::from_rows(&["##........", "#...######", "##.#######"]);
        assert_eq!(opponent_threat(&rules, &tsd, 0, false), 4);
        assert_eq!(opponent_threat(&rules, &tsd, 0, true), 5, "B2B なら +1");
        assert_eq!(opponent_threat(&rules, &tsd, 3, false), 7, "ゲージの分を足す");
        // 4段そろった井戸(テトリス)
        let well = Board::from_rows(&["#########."; 4]);
        assert_eq!(opponent_threat(&rules, &well, 0, false), 4);
        assert_eq!(opponent_threat(&rules, &Board::new(), 0, false), 0);
    }

    #[test]
    fn とどめと相手の危なさ() {
        let w = Weights::default();
        // 相手は17段(余裕3段)。4行届けばとどめ、2行では足りない
        let ctx = BattleContext { opponent_danger: Some(17), opponent_threat: 0, gauge: 0 };
        let lethal = battle_value(&w, &ctx, 4, 5);
        let not = battle_value(&w, &ctx, 2, 5);
        assert!(lethal - not > w.lethal * 0.9, "{lethal} {not}");
        // 相手の情報が無ければ、届く攻撃の点(既定 0)だけ
        let none = BattleContext::default();
        assert_eq!(battle_value(&w, &none, 4, 5), 0.0);
    }

    #[test]
    fn 新しい作戦の重みは既定では効かない() {
        // 中あけ・空中TSD・パフェの見込みがそろった盤面でも、既定の重みでは中あけ積みにならず点も変わらない
        let w = Weights::default();
        let board = Board::from_rows(&["##........", "###....###", "###....###"]);
        assert!(!center_well_mode(&w, &board));
        let off = EvalContext { t_available: 1, ..Default::default() };
        let on = EvalContext { center_well: true, ..off };
        assert_eq!(evaluate(&w, &board, false, &off), evaluate(&w, &board, false, &on));
    }

    #[test]
    fn 中あけ積みの評価() {
        let w = Weights { center_well_rows: 1.0, center_well_side_diff: -0.5, ..Weights::default() };
        let well = Board::from_rows(&["###....###", "###....###"]);
        assert!(center_well_mode(&w, &well));
        let ctx = EvalContext { t_available: 1, center_well: true, ..Default::default() };
        let base = evaluate(&Weights::default(), &well, false, &ctx);
        assert!((evaluate(&w, &well, false, &ctx) - base - 2.0).abs() < 1e-4, "中あけ2行で +2");
        // 高い盤面で中あけが無ければ、中あけ積みの作戦はとらない
        assert!(!center_well_mode(&w, &Board::from_rows(&["#########."; 8])));
    }

    #[test]
    fn パフェの見込みの加点() {
        let w = Weights { pc_chance: 3.0, ..Weights::default() };
        let even = Board::from_rows(&["########.."]);
        let odd = Board::from_rows(&["#########."]);
        let ctx = EvalContext::default();
        let gain = |b: &Board| evaluate(&w, b, false, &ctx) - evaluate(&Weights::default(), b, false, &ctx);
        assert!((gain(&even) - 3.0).abs() < 1e-4);
        assert_eq!(gain(&odd), 0.0, "奇数は考えない");
    }

    #[test]
    fn 低い盤面のソフトドロップほど減点が大きい() {
        use tetrisai_core::{Placement, Rotation};
        let w = Weights { soft_drop_rows: -0.1, ..Weights::default() };
        let rules = Rules::ppt2();
        let mv = |y: i8| Move {
            placement: Placement { location: PieceLocation::new(Piece::T, Rotation::South, 4, y), spin: Spin::Full },
            mid_air_stops: 0,
            soft_drop_sections: 1,
            inputs: 2,
        };
        let result = ClearResult { lines: 2, spin: Spin::Full, ..Default::default() };
        let low = reward(&w, &rules, &mv(1), &result);
        let high = reward(&w, &rules, &mv(11), &result);
        assert!((high - low - 1.0).abs() < 1e-4, "10段の差で 0.1 × 10 = 1.0 の差: {low} {high}");
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
