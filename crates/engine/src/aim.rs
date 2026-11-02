//! 狙い筋の説明: 今の作戦と読み筋から、何を狙っているかを短い文言にする(TBA の情報パネルの「狙い」)。
//!
//! 【2026-11-01・ユーザー】「中あけRENで操作ミスをしてしまい即負けになった。TBAの情報パネルに狙い筋を記載してほしい」。
//! 情報パネルの狙いの欄は2行(1行に全角17文字くらい)なので、1行目に作戦、2行目に気をつけることを書く。
//! 中あけ REN を撃っている間は、操作ミスを防ぐために「毎手1ライン消す」ことをはっきり書く。

use tetrisai_core::{Move, Rules, Spin};

use crate::eval::ren_potential;
use crate::opening::Opening;
use crate::search::Root;
use crate::shape;
use crate::strategy::Strategy;

/// 狙い筋の文言(改行で2行まで)。`plan` は最善の読み筋(1手目から)。
///
/// 【2026-11-03】作戦層で「C4W で対抗」「早撃ち」を選んでいる時は、1行目を作戦と理由(相手の C4W の行数。情報パネルの
/// 1行に収まるよう「(相手9行)」と短く書く)にし、
/// 2行目に読み筋の狙い(下の describe_plan の1行目)を書く。
pub fn describe(root: &Root, plan: &[Move], rules: &Rules) -> String {
    let aim = describe_plan(root, plan, rules);
    if root.strategy == Strategy::Normal || root.opening.is_active() {
        return aim;
    }
    let theirs = root.opponent.map_or(0, |o| shape::ren_rows(&o.board));
    let first = aim.lines().next().unwrap_or("");
    format!("{}(相手{theirs}行)\n{first}", root.strategy.label())
}

fn describe_plan(root: &Root, plan: &[Move], rules: &Rules) -> String {
    match &root.opening {
        // 1行目はテンプレの名前だけにする(長い名前「タンドリーチキン積み」でも1行に収まるように)
        Opening::Template(t) => return format!("{}\nテンプレ・Tスピンを撃つまで", t.name),
        Opening::Plain => return "序盤(素の状態)\n中あけRENは組まない".to_string(),
        Opening::None => {}
    }
    let board = root.state.board;
    // 読み筋を置いていき、手ごとの消し方を調べる
    let mut state = root.state;
    let clears: Vec<_> = plan.iter().map(|m| state.apply(&m.placement, rules)).collect();
    let rows = shape::ren_rows(&board);
    if rows > 0 {
        let first_clears = clears.first().is_some_and(|c| c.lines > 0);
        if root.state.consecutive_clears > 0 || first_clears {
            return format!("中あけREN 撃ち中\n毎手1ライン消す(残り{rows}行)");
        }
        let seed = shape::center_well_seed(&board) % 4;
        let fire = ren_potential(rules, &board, 0);
        return format!("中あけREN 貯め中\n種{seed}・{rows}行(撃てば火力{fire})");
    }
    for (i, c) in clears.iter().enumerate() {
        if c.lines == 0 {
            continue;
        }
        let kind = if c.perfect_clear {
            "パーフェクトクリア".to_string()
        } else {
            match (c.spin, c.lines) {
                (Spin::Full, 3) => "TST".to_string(),
                (Spin::Full, 2) => "TSD".to_string(),
                (Spin::Full, 1) => "TSS".to_string(),
                (Spin::None, 4) => "テトリス".to_string(),
                // 1〜3ライン消しや Tスピンミニは「狙い」ではなく、形を整えるための消し
                _ => continue,
            }
        };
        let when = if i == 0 { "この手".to_string() } else { format!("{i}手後") };
        return format!("{kind}({when})");
    }
    "積み込み(形を整える)".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use tetrisai_core::{generate, Board, Piece, PlayerState};

    fn root(rows: &[&str]) -> Root {
        Root { state: PlayerState::new(Board::from_rows(rows)), queue: vec![Piece::I], ..Default::default() }
    }

    /// 盤面 `rows` に置く手のうち、`lines` ライン消す(0 なら消さない)最初の手。
    fn find_move(root: &Root, piece: Piece, lines: u32, spin: Spin) -> Move {
        let rules = Rules::ppt2();
        generate(&root.state.board, piece, rules.spawn())
            .into_iter()
            .find(|m| {
                let mut st = root.state;
                let r = st.apply(&m.placement, &rules);
                r.lines == lines && (lines == 0 || r.spin == spin)
            })
            .unwrap()
    }

    #[test]
    fn 序盤の作戦() {
        let rules = Rules::ppt2();
        let template = crate::opening::td_templates()[0].clone();
        let r = Root { opening: Opening::Template(template), ..Default::default() };
        assert_eq!(describe(&r, &[], &rules), "ホットケーキ積み\nテンプレ・Tスピンを撃つまで");
        let r = Root { opening: Opening::Plain, ..Default::default() };
        assert_eq!(describe(&r, &[], &rules), "序盤(素の状態)\n中あけRENは組まない");
    }

    #[test]
    fn 中あけrenを貯める_撃つ() {
        let rules = Rules::ppt2();
        let mut rows = vec!["###....###"; 7];
        rows.push("###.######");
        let r = root(&rows);
        // 消さない手なら貯め中(種3・8行。REN 表 0+0+1+1+2+2+3+3 = 12)
        let keep = find_move(&r, Piece::O, 0, Spin::None);
        assert_eq!(describe(&r, &[keep], &rules), "中あけREN 貯め中\n種3・8行(撃てば火力12)");
        // 1ライン消す手なら撃ち中
        let fire = find_move(&r, Piece::I, 1, Spin::None);
        assert_eq!(describe(&r, &[fire], &rules), "中あけREN 撃ち中\n毎手1ライン消す(残り8行)");
        // REN の途中(前の手で消した)なら、読み筋によらず撃ち中
        let mut during = r.clone();
        during.state.consecutive_clears = 2;
        assert_eq!(describe(&during, &[keep], &rules), "中あけREN 撃ち中\n毎手1ライン消す(残り8行)");
    }

    #[test]
    fn 作戦層の作戦を1行目に書く() {
        let rules = Rules::ppt2();
        let mut opponent_rows = vec!["###....###"; 8];
        opponent_rows.push("###.######");
        let opponent = crate::search::Opponent { board: Board::from_rows(&opponent_rows), ..Default::default() };
        let mut r = root(&["##........", "#...######", "##.#######"]);
        r.opponent = Some(opponent);
        r.strategy = Strategy::Rush;
        let t = find_move(&r, Piece::T, 2, Spin::Full);
        assert_eq!(describe(&r, &[t], &rules), "早撃ち(相手9行)\nTSD(この手)");
        r.strategy = Strategy::C4WRival;
        assert_eq!(describe(&r, &[t], &rules), "C4Wで対抗(相手9行)\nTSD(この手)");
    }

    #[test]
    fn 読み筋の大きな手() {
        let rules = Rules::ppt2();
        // 2手目でテトリス
        let well = root(&["#########."; 4]);
        let stack = find_move(&well, Piece::O, 0, Spin::None);
        let mut after = well.clone();
        after.state.apply(&stack.placement, &rules);
        let tetris = find_move(&after, Piece::I, 4, Spin::None);
        assert_eq!(describe(&well, &[stack, tetris], &rules), "テトリス(1手後)");
        // TSD(この手)
        let tsd = root(&["##........", "#...######", "##.#######"]);
        let t = find_move(&tsd, Piece::T, 2, Spin::Full);
        assert_eq!(describe(&tsd, &[t], &rules), "TSD(この手)");
        // 大きな手が無ければ積み込み
        assert_eq!(describe(&root(&[]), &[], &rules), "積み込み(形を整える)");
    }
}
