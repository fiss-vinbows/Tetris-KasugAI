//! 作戦層: 相手の盤面を見て作戦を選ぶ(全自動で相手の情報があるときだけ)。
//!
//! 【2026-11-03・ユーザー】「全自動における C4W 決め打ちは弱いので状況による切り替えを搭載したい。中盤以降に相手が C4W を
//! 目指している場合における最善手を見極めたい(自分も C4W で対抗するか、TSD やテトリスを早々に乱発して組ませないか)」。
//!
//! 作戦は3つ。決まった段数ではなく、相手と自分の C4W の育ち具合を比べて選ぶ:
//! - 通常: 今までどおりの評価(C4W を選ぶかは貯めた REN の火力で決まる。8.9)
//! - C4W で対抗: 相手の C4W に対して、自分の C4W も同じくらい育っている(または自分の盤面が低く、これから育てられる)。
//!   C4W の REN は消すたびに予告を打ち消すので、相手の REN を受け止めながら撃ち返せる。貯めた REN の火力をより重く見る
//! - 早撃ち: 相手の C4W が自分より大きく育っている。同じ形で追いかけても間に合わないので、TSD・テトリスを早く撃って
//!   相手をせり上げ、積み込める段数を減らして組ませない。C4W は組まず、相手に届く攻撃を重く見る
//!
//! 切り替えには、すぐには戻らない幅を持たせる(同じ判断が `SWITCH_AFTER` 回続いたら切り替える)。
//! 判断の目安(行数)は暫定。実機の双方全自動で見ながら直す。

use tetrisai_core::board::WIDTH;
use tetrisai_core::Board;

use crate::shape;

/// 相手が C4W を目指しているとみなす、相手の中あけの行数(REN を続けられる見込みの行数)
pub const OPPONENT_C4W_ROWS: u32 = 3;
/// 自分の C4W がこの行数以上で、相手との差が `RIVAL_MARGIN` 以内なら対抗する
pub const RIVAL_MARGIN: u32 = 2;
/// 自分の盤面がこの段数以下なら、これから C4W を育てて対抗できるとみなす
pub const RIVAL_LOW_HEIGHT: u32 = 6;
/// 自分の C4W がこの行数以上あれば、早撃ちにはしない(貯めた分を撃ち切る)。
/// 【2026-11-07・ユーザー「C4W から REN を切らさずに発火してほしいが、積みなおしたり真ん中に置いて TSD に変えてしまう」】
/// 早撃ちは中あけの評価を丸ごと消すので、育てた C4W まで埋めてしまう
pub const KEEP_OWN_C4W_ROWS: u32 = 3;
/// 同じ判断がこの回数続いたら作戦を切り替える
pub const SWITCH_AFTER: u32 = 3;

/// 作戦。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Strategy {
    #[default]
    Normal,
    /// 自分も C4W で対抗する
    C4WRival,
    /// TSD・テトリスを早く撃って、相手に C4W を組ませない
    Rush,
    /// 迎え撃ち: 火力(TSD の溝・テトリスの縦穴)をためておき、相手の REN の攻撃が予告に来たら撃って打ち消す。C4W は組まない。
    /// 【2026-11-14・ユーザー「相手が中盤でC4Wを積み込む際に自分も合わせて積み込むのは相手の発火が早くて間に合わないことが
    /// あります。間に合わない場合は火力をため込んで相殺して迎え撃つ方針を考えたい」】重み一式 `Weights::counter` がある時だけ、
    /// 早撃ちの代わりに使う(`with_counter`)
    Counter,
}

impl Strategy {
    /// `c4w_response`(Weights)で、相手の C4W への答え方を固定する。
    pub fn forced(self, response: u32) -> Self {
        match (self, response) {
            (Strategy::Normal, _) => Strategy::Normal,
            (_, 1) => Strategy::C4WRival,
            (_, 2) => Strategy::Rush,
            (s, _) => s,
        }
    }

    /// 情報パネルに出す作戦の名前。
    pub fn label(self) -> &'static str {
        match self {
            Strategy::Normal => "通常",
            Strategy::C4WRival => "C4Wで対抗",
            Strategy::Rush => "早撃ち",
            Strategy::Counter => "迎え撃ち",
        }
    }

    /// 迎え撃ちの重み一式(`Weights::counter`)があれば、早撃ちの代わりに迎え撃ちにする。
    pub fn with_counter(self, counter: bool) -> Self {
        if counter && self == Strategy::Rush { Strategy::Counter } else { self }
    }

    /// C4W を組まない作戦か(早撃ち・迎え撃ち)。
    pub fn avoids_c4w(self) -> bool {
        matches!(self, Strategy::Rush | Strategy::Counter)
    }

    /// 相手の REN(続けて消した回数 `their_ren`)がこちら(`our_ren`)より `gap` 以上多ければ、C4W で張り合わずに早撃ちにする
    /// (`Weights::rival_ren_gap`。0 なら変えない。迎え撃ちの重みがあれば、この後の `with_counter` で迎え撃ちになる)。
    ///
    /// 【2026-11-14・ユーザー「C4Wにおいて相手より5RENの差(例えば自分が1REN目の時に相手が6REN目)がついていると押し切られるので
    /// 別な方針で迎撃してください」】C4W どうしは5REN差で後打ちが負ける(実機の知見)。
    pub fn with_ren_gap(self, our_ren: u32, their_ren: u32, gap: u32) -> Self {
        if gap > 0 && their_ren >= our_ren + gap && !self.avoids_c4w() { Strategy::Rush } else { self }
    }
}

/// 局面からその場で選ぶ作戦と、その理由(相手の C4W の行数, 自分の C4W の行数)。相手の盤面が無ければ通常。
pub fn decide(own: &Board, opponent: Option<&Board>) -> (Strategy, u32, u32) {
    let Some(opponent) = opponent else { return (Strategy::Normal, 0, 0) };
    let theirs = shape::ren_rows(opponent);
    let ours = shape::ren_rows(own);
    if theirs < OPPONENT_C4W_ROWS {
        return (Strategy::Normal, theirs, ours);
    }
    let rival = ours >= KEEP_OWN_C4W_ROWS || (ours > 0 && ours + RIVAL_MARGIN >= theirs) || (ours == 0 && own.max_height() <= RIVAL_LOW_HEIGHT && theirs <= 5);
    (if rival { Strategy::C4WRival } else { Strategy::Rush }, theirs, ours)
}

/// 相手の REN(続けて消した回数)を、受け取った相手の盤面のマスの数の変わり方から数える(bot 用。相手の REN は送られてこない)。
///
/// ミノを置いて消さなければマスが4増える(REN が切れる)。消せば `4 - 10 × 消した行数` だけ変わる(REN が続く)。
/// おじゃまのせり上がりは1行9マスずつ増える(REN は変えない。せり上がりと置いた手が重なれば `9k + 4` で切れる)。
/// 2手分をまとめて受け取ると数え違えることがあるが、5REN の差を見る目安には足りる。
#[derive(Clone, Debug, Default)]
pub struct OpponentRenTracker {
    last_cells: Option<i32>,
    ren: u32,
}

impl OpponentRenTracker {
    pub fn observe(&mut self, opponent: &Board) -> u32 {
        let cells = (0..WIDTH).map(|x| opponent.cols[x].count_ones() as i32).sum::<i32>();
        if let Some(last) = self.last_cells {
            let d = cells - last;
            if d < 0 {
                self.ren += 1;
            } else if d > 0 && d % 9 != 0 {
                self.ren = 0;
            }
        }
        self.last_cells = Some(cells);
        self.ren
    }
}

/// 作戦の切り替えを追う(すぐには戻らない幅を持たせる)。bot が局面を受け取るたびに `observe` を呼ぶ。
#[derive(Clone, Debug, Default)]
pub struct StrategyTracker {
    current: Strategy,
    candidate: Strategy,
    streak: u32,
    /// 最後に判断した時の (相手の C4W の行数, 自分の C4W の行数)
    pub reason: (u32, u32),
}

impl StrategyTracker {
    pub fn observe(&mut self, own: &Board, opponent: Option<&Board>) -> Strategy {
        let (next, theirs, ours) = decide(own, opponent);
        self.reason = (theirs, ours);
        if next == self.current {
            self.streak = 0;
        } else if next == self.candidate {
            self.streak += 1;
            if self.streak >= SWITCH_AFTER {
                self.current = next;
                self.streak = 0;
            }
        } else {
            self.candidate = next;
            self.streak = 1;
            if SWITCH_AFTER <= 1 {
                self.current = next;
            }
        }
        self.current
    }

    pub fn current(&self) -> Strategy {
        self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c4w(rows: usize) -> Board {
        let mut text = vec!["###....###"; rows];
        text.push("###.######");
        Board::from_rows(&text)
    }

    #[test]
    fn 相手がc4wでなければ通常() {
        assert_eq!(decide(&c4w(5), None).0, Strategy::Normal, "相手の情報が無い");
        assert_eq!(decide(&c4w(5), Some(&Board::from_rows(&["#########."; 6]))).0, Strategy::Normal);
    }

    #[test]
    fn 自分のc4wが同じくらいなら対抗() {
        assert_eq!(decide(&c4w(6), Some(&c4w(7))).0, Strategy::C4WRival);
        // 自分の盤面が低く、相手の C4W もまだ小さいなら、これから育てて対抗する
        assert_eq!(decide(&Board::from_rows(&["####..####"; 2]), Some(&c4w(4))).0, Strategy::C4WRival);
    }

    #[test]
    fn 相手のc4wが大きく育っていたら早撃ち() {
        assert_eq!(decide(&Board::from_rows(&["####..####"; 2]), Some(&c4w(9))), (Strategy::Rush, 10, 0));
        assert_eq!(decide(&c4w(1), Some(&c4w(9))).0, Strategy::Rush, "自分の C4W が小さすぎる");
        assert_eq!(decide(&c4w(3), Some(&c4w(12))).0, Strategy::C4WRival, "育てた C4W は早撃ちで埋めない");
    }

    #[test]
    fn 迎え撃ちの重みがあれば早撃ちの代わりに迎え撃ち() {
        // 【2026-11-14・ユーザー「間に合わない場合は火力をため込んで相殺して迎え撃つ方針を考えたい」】
        assert_eq!(Strategy::Rush.with_counter(true), Strategy::Counter);
        assert_eq!(Strategy::Rush.with_counter(false), Strategy::Rush);
        assert_eq!(Strategy::C4WRival.with_counter(true), Strategy::C4WRival, "間に合う時は C4W で対抗のまま");
        assert_eq!(Strategy::Normal.with_counter(true), Strategy::Normal);
        assert!(Strategy::Counter.avoids_c4w() && Strategy::Rush.avoids_c4w() && !Strategy::C4WRival.avoids_c4w());
        assert_eq!(Strategy::Counter.label(), "迎え撃ち");
    }

    #[test]
    fn 相手のrenが5多ければc4wで張り合わない() {
        // 【2026-11-14・ユーザー「C4Wにおいて相手より5RENの差(例えば自分が1REN目の時に相手が6REN目)がついていると押し切られるので
        // 別な方針で迎撃してください」】
        assert_eq!(Strategy::C4WRival.with_ren_gap(1, 6, 5), Strategy::Rush, "5差なら迎え撃つ");
        assert_eq!(Strategy::C4WRival.with_ren_gap(0, 5, 5), Strategy::Rush, "撃ち始める前に5差でも迎え撃つ");
        assert_eq!(Strategy::C4WRival.with_ren_gap(2, 6, 5), Strategy::C4WRival, "4差なら張り合う");
        assert_eq!(Strategy::Normal.with_ren_gap(0, 7, 5), Strategy::Rush, "通常でも相手の長い REN には C4W を組まない");
        assert_eq!(Strategy::C4WRival.with_ren_gap(0, 9, 0), Strategy::C4WRival, "0 なら使わない");
        assert_eq!(Strategy::Counter.with_ren_gap(0, 9, 5), Strategy::Counter);
        assert_eq!(Strategy::C4WRival.with_ren_gap(1, 6, 5).with_counter(true), Strategy::Counter);
    }

    #[test]
    fn 相手のrenを盤面の変わり方から数える() {
        let mut t = OpponentRenTracker::default();
        let well = |rows: usize| Board::from_rows(&vec!["###....###"; rows]);
        assert_eq!(t.observe(&well(8)), 0);
        // 中あけに置いて1行ずつ消す(マスが 4 - 10 = 6 減る)
        let mut b = well(8);
        for k in 1..=6 {
            b = well(8 - k);
            assert_eq!(t.observe(&b), k as u32);
        }
        // おじゃまのせり上がり(1行9マス)では切れない
        let mut rows = vec!["###....###"; 2];
        rows.push("####.#####");
        assert_eq!(t.observe(&Board::from_rows(&rows)), 6);
        // 消さずに置いたら切れる
        let mut rows = vec!["###....###"; 2];
        rows.push("####.#####");
        rows.insert(0, "###.....##");
        rows.insert(0, "###.###...");
        assert_eq!(t.observe(&Board::from_rows(&rows)), 0);
        let _ = b;
    }

    #[test]
    fn すぐには切り替えない() {
        let mut tracker = StrategyTracker::default();
        let own = Board::from_rows(&["####..####"; 2]);
        let big = c4w(9);
        assert_eq!(tracker.observe(&own, Some(&big)), Strategy::Normal, "1回目");
        assert_eq!(tracker.observe(&own, Some(&big)), Strategy::Normal, "2回目");
        assert_eq!(tracker.observe(&own, Some(&big)), Strategy::Rush, "3回続いたら切り替える");
        // 一度だけ違う判断が出ても戻らない
        assert_eq!(tracker.observe(&own, None), Strategy::Rush);
        assert_eq!(tracker.observe(&own, Some(&big)), Strategy::Rush);
    }
}
