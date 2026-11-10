//! 狙い筋の説明: 今の作戦と読み筋から、何を狙っているかを短い文言にする(TBA の情報パネルの「狙い」)。
//!
//! 【2026-11-01・ユーザー】「中あけRENで操作ミスをしてしまい即負けになった。TBAの情報パネルに狙い筋を記載してほしい」。
//! 情報パネルの狙いの欄は2行(1行に全角17文字くらい)なので、1行目に作戦、2行目に気をつけることを書く。
//! 中あけ REN を撃っている間は、操作ミスを防ぐために「毎手1ライン消す」ことをはっきり書く。

use std::collections::HashSet;

use tetrisai_core::{generate, Board, Move, Piece, Rules, Spin};

use crate::eval::ren_potential;
use crate::opening::Opening;
use crate::search::Root;
use crate::shape;
use crate::strategy::Strategy;

/// `downstack_ren` が調べる局面の上限(これを超えたら、それまでの最大を返す)
const DOWNSTACK_NODES: usize = 3_000;

/// Downstack(掘り)で続けられる REN の見込み。見えているミノ(操作ミノ・NEXT・HOLD)で毎手ラインを消し続けた時の、
/// 一番長い連続の消去回数(今続いている分を含む)を返す。2つ目は、見えているミノをすべて使って消し切れた(この先も続くかもしれない)か。
/// 【2026-11-07・ユーザー「掘り REN の最大の見込みを情報パネルに載せたい。Downstack で表記」】
pub fn downstack_ren(root: &Root, rules: &Rules) -> (u32, bool) {
    let mut seen = HashSet::new();
    let (mut best, mut open) = (0, false);
    walk(&root.state.board, &root.queue, root.hold, 0, rules, &mut seen, &mut best, &mut open);
    (root.state.consecutive_clears + best, open)
}

#[allow(clippy::too_many_arguments)]
fn walk(
    board: &Board,
    queue: &[Piece],
    hold: Option<Piece>,
    depth: u32,
    rules: &Rules,
    seen: &mut HashSet<(Board, usize, Option<Piece>)>,
    best: &mut u32,
    open: &mut bool,
) {
    if depth > *best {
        *best = depth;
    }
    let Some((&current, rest)) = queue.split_first() else {
        *open = true;
        return;
    };
    if seen.len() >= DOWNSTACK_NODES || !seen.insert((*board, queue.len(), hold)) {
        return;
    }
    let mut step = |piece: Piece, rest: &[Piece], hold: Option<Piece>| {
        for m in generate(board, piece, rules.spawn()).iter() {
            let mut b = *board;
            b.place(&m.placement.location);
            let full = b.full_lines();
            if full == 0 {
                continue;
            }
            b.remove_lines(full);
            walk(&b, rest, hold, depth + 1, rules, seen, best, open);
        }
    };
    step(current, rest, hold);
    match hold {
        Some(h) if h != current => step(h, rest, Some(current)),
        Some(_) => {}
        None => {
            if let Some((&next, rest2)) = rest.split_first() {
                step(next, rest2, Some(current));
            }
        }
    }
}

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
        Opening::Template(t) => return format!("{}\nTST→TSD→パフェまで", t.name),
        Opening::Plain => return "序盤(素の状態)\nC4Wは組まない".to_string(),
        Opening::Tsd => return "開幕TSD\nTSD を撃つまで".to_string(),
        Opening::Pco(_) => return "開幕パフェ\n10ミノで4段".to_string(),
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
            return format!("C4W 撃ち中\n毎手1ライン消す(残り{rows}行)");
        }
        let seed = shape::center_well_seed(&board) % 4;
        let fire = ren_potential(rules, &board, 0);
        return format!("C4W積み込み中\n種{seed}・{rows}行(撃てば火力{fire})");
    }
    // 【2026-11-08・ユーザー「相殺外しをする旨を情報パネルに記載してほしい」】この手でせり上がる予告があり、今のミノで
    // 攻撃して相殺できるのに、1手目では撃たずに後の手で大きく撃つ読み筋なら「相殺外し」と書く
    let due: u32 = root.incoming.iter().filter(|g| g.delay == 0).map(|g| g.lines).sum();
    if due > 0 && clears.first().is_some_and(|c| c.attack == 0) && can_attack_now(root, rules) {
        if let Some((i, kind)) = clears.iter().enumerate().skip(1).find_map(|(i, c)| big_clear(c).map(|k| (i, k))) {
            return format!("相殺外し(予告{due}行を受ける)\n{kind}({i}手後に撃つ)");
        }
    }
    for (i, c) in clears.iter().enumerate() {
        let Some(kind) = big_clear(c) else { continue };
        let when = if i == 0 { "この手".to_string() } else { format!("{i}手後") };
        return format!("{kind}({when})");
    }
    "積み込み(形を整える)".to_string()
}

/// 狙いとして書く大きな消し方の名前。1〜3ライン消しや Tスピンミニは「狙い」ではなく、形を整えるための消しなので None。
fn big_clear(c: &tetrisai_core::ClearResult) -> Option<String> {
    if c.lines == 0 {
        return None;
    }
    if c.perfect_clear {
        return Some("パーフェクトクリア".to_string());
    }
    match (c.spin, c.lines) {
        (Spin::Full, 3) => Some("TST".to_string()),
        (Spin::Full, 2) => Some("TSD".to_string()),
        (Spin::Full, 1) => Some("TSS".to_string()),
        (Spin::None, 4) => Some("テトリス".to_string()),
        _ => None,
    }
}

/// 今の手(操作ミノか HOLD のミノ)で、攻撃の出る消し方ができるか(相殺できるか)。
fn can_attack_now(root: &Root, rules: &Rules) -> bool {
    let mut pieces: Vec<Piece> = root.queue.first().copied().into_iter().collect();
    match root.hold {
        Some(h) => pieces.push(h),
        None => pieces.extend(root.queue.get(1).copied()),
    }
    pieces.into_iter().any(|p| {
        generate(&root.state.board, p, rules.spawn()).iter().any(|m| {
            let mut st = root.state;
            st.apply(&m.placement, rules).attack > 0
        })
    })
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
    fn 相殺外しを書く() {
        let rules = Rules::ppt2();
        // TSD の溝がある盤面に、すぐせり上がる予告が2行。O を右に置いて受け、次の T で TSD を撃つ読み筋
        let rows = ["##........", "#...######", "##.#######", "#########.", "#########."];
        let mut r = root(&rows);
        r.queue = vec![Piece::O, Piece::T];
        r.incoming = vec![crate::IncomingGarbage { lines: 2, delay: 0 }];
        let o = generate(&r.state.board, Piece::O, rules.spawn())
            .into_iter()
            .find(|m| {
                let mut st = r.state;
                st.apply(&m.placement, &rules).lines == 0 && m.placement.location.cells().iter().all(|&(x, _)| x >= 6)
            })
            .unwrap();
        let mut after = r.clone();
        after.state.apply(&o.placement, &rules);
        let tsd = find_move(&after, Piece::T, 2, Spin::Full);
        assert_eq!(describe(&r, &[o, tsd], &rules), "相殺外し(予告2行を受ける)\nTSD(1手後に撃つ)");
        // 予告が無ければ、ふつうの狙い
        r.incoming.clear();
        assert_eq!(describe(&r, &[o, tsd], &rules), "TSD(1手後)");
        // すぐ撃つなら相殺外しではない
        r.incoming = vec![crate::IncomingGarbage { lines: 2, delay: 0 }];
        let now = find_move(&r, Piece::T, 2, Spin::Full);
        assert_eq!(describe(&r, &[now], &rules), "TSD(この手)");
    }

    #[test]
    fn 序盤の作戦() {
        let rules = Rules::ppt2();
        let template = crate::opening::td_templates()[0].clone();
        let r = Root { opening: Opening::Template(template), ..Default::default() };
        assert_eq!(describe(&r, &[], &rules), "ホットケーキ積み\nTST→TSD→パフェまで");
        let r = Root { opening: Opening::Plain, ..Default::default() };
        assert_eq!(describe(&r, &[], &rules), "序盤(素の状態)\nC4Wは組まない");
    }

    #[test]
    fn 中あけrenを貯める_撃つ() {
        let rules = Rules::ppt2();
        let mut rows = vec!["###....###"; 7];
        rows.push("###.######");
        let r = root(&rows);
        // 消さない手なら貯め中(種3・8行。REN 表 0+0+1+1+2+2+3+3 = 12)
        let keep = find_move(&r, Piece::O, 0, Spin::None);
        assert_eq!(describe(&r, &[keep], &rules), "C4W積み込み中\n種3・8行(撃てば火力12)");
        // 1ライン消す手なら撃ち中
        let fire = find_move(&r, Piece::I, 1, Spin::None);
        assert_eq!(describe(&r, &[fire], &rules), "C4W 撃ち中\n毎手1ライン消す(残り8行)");
        // REN の途中(前の手で消した)なら、読み筋によらず撃ち中
        let mut during = r.clone();
        during.state.consecutive_clears = 2;
        assert_eq!(describe(&during, &[keep], &rules), "C4W 撃ち中\n毎手1ライン消す(残り8行)");
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
    fn downstack_ren_counts_clears_with_visible_pieces() {
        use tetrisai_core::Piece::*;
        let rules = Rules::ppt2();
        // 1列あいた行が4段: I を縦に入れると1手で消し切るので、続きは無い
        let board = Board::from_rows(&["#########."; 4]);
        let mut root = Root { hold: None, queue: vec![I, O], ..Root::default() };
        root.state.board = board;
        assert_eq!(downstack_ren(&root, &rules).0, 1);
        // 消せない形なら 0。今続いている REN は足す
        root.state.board = Board::default();
        root.state.consecutive_clears = 3;
        assert_eq!(downstack_ren(&root, &rules), (3, false));
        // 2段あいた列に O を2回: 2手続く
        let mut root = Root { hold: None, queue: vec![O, O], ..Root::default() };
        root.state.board = Board::from_rows(&["########..", "########..", "########..", "########.."]);
        assert_eq!(downstack_ren(&root, &rules), (2, true), "見えているミノをすべて使った");
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
