//! ゲームのルール(出現位置・攻撃量・おじゃまの扱い)と、1手置いたときの結果の計算。
//!
//! 数値はコードに埋め込まず `config/rules/*.toml` から読む。既定のぷよテト2のルールは
//! ビルド時に取り込んでおき(`Rules::ppt2`)、設定ファイルが無くても動くようにする。

use crate::board::Board;
use crate::movegen::{Placement, Spawn};
use crate::srs::Spin;
use serde::Deserialize;

const PPT2_TOML: &str = include_str!("../../../config/rules/ppt2.toml");

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Rules {
    pub name: String,
    pub spawn: SpawnConfig,
    pub attack: AttackTable,
    pub garbage: GarbageRules,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq)]
pub struct SpawnConfig {
    pub x: i8,
    pub y: i8,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct AttackTable {
    pub single: u32,
    pub double: u32,
    pub triple: u32,
    pub tetris: u32,
    pub tspin_mini_single: u32,
    pub tspin_mini_double: u32,
    pub tspin_single: u32,
    pub tspin_double: u32,
    pub tspin_triple: u32,
    pub back_to_back_bonus: u32,
    pub perfect_clear: u32,
    /// 添字が REN 数。表より長い REN は最後の値。
    pub combo: Vec<u32>,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct GarbageRules {
    /// 1回のミノ固定でせり上がる行数の上限(0 は上限なし)
    pub max_rise_per_piece: u32,
    /// 1回の攻撃の中で穴の列が行ごとに変わるか
    pub hole_changes_within_attack: bool,
    /// 攻撃が届いてからせり上がれるようになるまでの、受け手のミノ数
    pub delay_pieces: u32,
    /// REN 中の攻撃をゲージにため、ラインを消さない手を置いたときにまとめて送るか
    pub send_after_ren_ends: bool,
    /// 攻撃を自分への予告の打ち消し(相殺)に先に使うか
    pub cancel_incoming_first: bool,
}

impl Rules {
    /// ぷよテト2 テトリス vs テトリスの既定ルール。
    pub fn ppt2() -> Rules {
        Rules::from_toml(PPT2_TOML).expect("同梱のルール設定が読めない")
    }

    pub fn from_toml(text: &str) -> Result<Rules, toml::de::Error> {
        toml::from_str(text)
    }

    pub fn spawn(&self) -> Spawn {
        Spawn { x: self.spawn.x, y: self.spawn.y }
    }

    /// ライン消去の種類ごとの基本の攻撃量(B2B・REN・パフェを除く)。
    pub fn line_attack(&self, lines: u32, spin: Spin) -> u32 {
        let a = &self.attack;
        match (spin, lines) {
            (_, 0) => 0,
            (Spin::None, 1) => a.single,
            (Spin::None, 2) => a.double,
            (Spin::None, 3) => a.triple,
            (Spin::None, _) => a.tetris,
            (Spin::Mini, 1) => a.tspin_mini_single,
            (Spin::Mini, _) => a.tspin_mini_double,
            (Spin::Full, 1) => a.tspin_single,
            (Spin::Full, 2) => a.tspin_double,
            (Spin::Full, _) => a.tspin_triple,
        }
    }

    pub fn combo_attack(&self, ren: u32) -> u32 {
        let table = &self.attack.combo;
        match table.get(ren as usize) {
            Some(&v) => v,
            None => table.last().copied().unwrap_or(0),
        }
    }
}

/// 1人分の、盤面以外も含めた状態(B2B・REN)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PlayerState {
    pub board: Board,
    /// 直前のライン消去がテトリスかTスピン(ミニを含む)だったか。
    pub back_to_back: bool,
    /// 続けてラインを消した回数(直前の手で消していなければ 0)。REN 数はこれ − 1。
    pub consecutive_clears: u32,
}

/// 1手置いた結果。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClearResult {
    pub lines: u32,
    pub spin: Spin,
    pub perfect_clear: bool,
    /// B2B のボーナスが付いたか。
    pub back_to_back: bool,
    /// この手での REN 数(消していなければ 0)。
    pub ren: u32,
    /// この手で生まれた攻撃量(相殺前)。
    pub attack: u32,
}

impl PlayerState {
    pub fn new(board: Board) -> Self {
        PlayerState { board, ..Default::default() }
    }

    /// ミノを置き、ライン消去・B2B・REN・攻撃量を計算する。
    pub fn apply(&mut self, placement: &Placement, rules: &Rules) -> ClearResult {
        self.board.place(&placement.location);
        let lines = self.board.clear_full_lines();
        if lines == 0 {
            self.consecutive_clears = 0;
            // ラインを消さないTスピンは B2B を切らない
            return ClearResult { spin: placement.spin, ..Default::default() };
        }

        let hard = lines >= 4 || placement.spin != Spin::None;
        let back_to_back = hard && self.back_to_back;
        self.back_to_back = hard;
        self.consecutive_clears += 1;
        let ren = self.consecutive_clears - 1;
        let perfect_clear = self.board.is_empty();

        let base = if perfect_clear { rules.attack.perfect_clear } else { rules.line_attack(lines, placement.spin) };
        let b2b = if back_to_back { rules.attack.back_to_back_bonus } else { 0 };
        let attack = base + b2b + rules.combo_attack(ren);

        ClearResult { lines, spin: placement.spin, perfect_clear, back_to_back, ren, attack }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::piece::{Piece, PieceLocation, Rotation};

    fn place(piece: Piece, rotation: Rotation, x: i8, y: i8, spin: Spin) -> Placement {
        Placement { location: PieceLocation::new(piece, rotation, x, y), spin }
    }

    #[test]
    fn 同梱のルールが読める() {
        let rules = Rules::ppt2();
        assert_eq!(rules.spawn(), Spawn { x: 4, y: 19 });
        assert_eq!(rules.line_attack(4, Spin::None), 4);
        assert_eq!(rules.line_attack(2, Spin::Full), 4);
        assert_eq!(rules.line_attack(3, Spin::Full), 6);
        assert_eq!(rules.combo_attack(0), 0);
        assert_eq!(rules.combo_attack(2), 1);
        assert_eq!(rules.combo_attack(10), 5, "10REN(11回目)は5段");
        assert_eq!(rules.combo_attack(100), 5, "表より長いRENは最後の値");
    }

    #[test]
    fn テトリスのb2bとren() {
        let rules = Rules::ppt2();
        // 右端1列が空いた8段
        let rows = ["#########."; 8];
        let mut state = PlayerState::new(Board::from_rows(&rows));
        let i = place(Piece::I, Rotation::East, 9, 2, Spin::None); // (9,0)〜(9,3)
        let r = state.apply(&i, &rules);
        assert_eq!((r.lines, r.attack, r.back_to_back, r.ren), (4, 4, false, 0));
        let r = state.apply(&i, &rules);
        // 2回目: テトリス4 + B2B1 + 1REN目0 = 5、盤面が空になるのでパフェ(10+1+0)
        assert!(r.perfect_clear);
        assert_eq!((r.lines, r.back_to_back, r.ren, r.attack), (4, true, 1, 11));
    }

    #[test]
    fn 消さない手でrenが切れる() {
        let rules = Rules::ppt2();
        let mut state = PlayerState::new(Board::from_rows(&["#########.", "#########."]));
        state.consecutive_clears = 3;
        let o = place(Piece::O, Rotation::North, 0, 2, Spin::None);
        let r = state.apply(&o, &rules);
        assert_eq!(r.lines, 0);
        assert_eq!(state.consecutive_clears, 0);
    }
}
