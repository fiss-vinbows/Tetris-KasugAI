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

use tetrisai_core::Board;

use crate::shape;

/// 相手が C4W を目指しているとみなす、相手の中あけの行数(REN を続けられる見込みの行数)
pub const OPPONENT_C4W_ROWS: u32 = 3;
/// 自分の C4W がこの行数以上で、相手との差が `RIVAL_MARGIN` 以内なら対抗する
pub const RIVAL_MARGIN: u32 = 2;
/// 自分の盤面がこの段数以下なら、これから C4W を育てて対抗できるとみなす
pub const RIVAL_LOW_HEIGHT: u32 = 6;
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
}

impl Strategy {
    /// 情報パネルに出す作戦の名前。
    pub fn label(self) -> &'static str {
        match self {
            Strategy::Normal => "通常",
            Strategy::C4WRival => "C4Wで対抗",
            Strategy::Rush => "早撃ち",
        }
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
    let rival = (ours > 0 && ours + RIVAL_MARGIN >= theirs) || (ours == 0 && own.max_height() <= RIVAL_LOW_HEIGHT && theirs <= 5);
    (if rival { Strategy::C4WRival } else { Strategy::Rush }, theirs, ours)
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
        assert_eq!(decide(&c4w(2), Some(&c4w(9))).0, Strategy::Rush, "自分の C4W が小さすぎる");
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
