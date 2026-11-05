//! 序盤: TD系テンプレか、素の状態(テンプレを使わない)を対局ごとにランダムで選ぶ。
//!
//! 【2026-11-01・ユーザー】「いきなりRENを組むのは対策されると弱いので、序盤はTDテンプレか素の状態をランダムで
//! 選ぶ」。決めたこと: テンプレは TBA の TD系6種(ホットケーキ積み・くろみつ砲・PC-Spin・皐月積み・ベーカリーTD・
//! タンドリーチキン積み)、知識は KasugAI の中に持つ、序盤は「テンプレを撃つまで」(素の状態は同じくらいの
//! 14手まで)、割合は半々(テンプレの中では、見えているミノで組めるものから均等に選ぶ)。
//!
//! 【2026-11-04・ユーザー】タンドリーチキン積みは Oミノの入れ方が失敗しやすいので候補から外した(図のデータは残す)。
//!
//! テンプレの図は TBA から書き出したもの(`data/td_templates.json`、`tools/export_td_templates.py`)。
//! テンプレの進め方は探索に任せる: 図のどれかに合うように、図のマスへ図と同じミノを置く手に加点する
//! (`Weights::template_piece`)。図は巡ごとにあり、次の巡の図は「既に置いてあるブロック」が今の盤面と
//! 一致するものが自然に選ばれる(TBA の openers.py と同じ考え方)。図から外れた手か、ラインを消した手
//! (テンプレを撃った)で序盤は終わる。序盤の間は中あけ REN の評価を使わない(`EvalContext::opening`)。

use std::sync::{Arc, OnceLock};

use serde::Deserialize;
use tetrisai_core::board::WIDTH;
use tetrisai_core::{generate, Board, Piece, PieceLocation, Rules};

use crate::search::Root;

/// 素の状態の序盤の手数(TD テンプレで撃つまでと同じくらい: 2巡)
pub const PLAIN_PIECES: u32 = 14;

/// 図の1つ(ある巡の完成形)。
#[derive(Clone, Debug, PartialEq)]
pub struct Form {
    pub section: String,
    /// 既に置いてあるべきブロック
    pub existing: [u64; WIDTH],
    /// この図で置くマスすべて(既に置いてあるブロックを含まない)
    pub cells: [u64; WIDTH],
    /// この図で置くミノ(ミノの種類と、マスを並べたもの)
    pub items: Vec<(Piece, [(i8, i8); 4])>,
    /// 図の他のミノをすべて置いた後に入れる T(Tスピン)の番号
    pub last: Vec<usize>,
}

/// テンプレ1つ(左右反転の図を含む)。
#[derive(Clone, Debug, PartialEq)]
pub struct Template {
    pub name: String,
    pub forms: Vec<Form>,
}

#[derive(Deserialize)]
struct RawTemplate {
    name: String,
    forms: Vec<RawForm>,
}

#[derive(Deserialize)]
struct RawForm {
    section: String,
    existing: Vec<[i8; 2]>,
    items: Vec<RawItem>,
}

#[derive(Deserialize)]
struct RawItem {
    piece: char,
    cells: Vec<[i8; 2]>,
    last: bool,
}

fn bits(cells: impl IntoIterator<Item = (i8, i8)>) -> [u64; WIDTH] {
    let mut b = [0u64; WIDTH];
    for (x, y) in cells {
        b[x as usize] |= 1 << y;
    }
    b
}

/// 図のデータにあっても、序盤の候補にしないテンプレ。
const EXCLUDED_TEMPLATES: [&str; 1] = ["タンドリーチキン積み"];

/// TD系テンプレ5種(engine に埋め込んだ図を初回に読む。`EXCLUDED_TEMPLATES` は除く)。
pub fn td_templates() -> &'static [Arc<Template>] {
    static TEMPLATES: OnceLock<Vec<Arc<Template>>> = OnceLock::new();
    TEMPLATES.get_or_init(|| {
        let raw: Vec<RawTemplate> =
            serde_json::from_str(include_str!("../data/td_templates.json")).expect("td_templates.json を読めない");
        raw.into_iter()
            .filter(|t| !EXCLUDED_TEMPLATES.contains(&t.name.as_str()))
            .map(|t| {
                let forms = t
                    .forms
                    .into_iter()
                    .filter_map(|f| {
                        let mut items = Vec::new();
                        let mut last = Vec::new();
                        for item in &f.items {
                            let cells: [(i8, i8); 4] =
                                item.cells.iter().map(|c| (c[0], c[1])).collect::<Vec<_>>().try_into().ok()?;
                            if item.last {
                                last.push(items.len());
                            }
                            items.push((Piece::from_char(item.piece)?, sorted(cells)));
                        }
                        Some(Form {
                            section: f.section,
                            existing: bits(f.existing.iter().map(|c| (c[0], c[1]))),
                            cells: bits(items.iter().flat_map(|(_, c)| c.iter().copied())),
                            items,
                            last,
                        })
                    })
                    .collect();
                Arc::new(Template { name: t.name, forms })
            })
            .collect()
    })
}

/// `PieceLocation::sorted_cells` と同じ (y, x) の昇順に並べる。
fn sorted(mut cells: [(i8, i8); 4]) -> [(i8, i8); 4] {
    cells.sort_by_key(|&(x, y)| (y, x));
    cells
}

fn subset(a: &[u64; WIDTH], b: &[u64; WIDTH]) -> bool {
    a.iter().zip(b).all(|(x, y)| x & !y == 0)
}

impl Form {
    /// 盤面がこの図の途中か(既に置いてあるべきブロックがすべてあり、図に無いマスが無い)。
    pub fn consistent(&self, board: &Board) -> bool {
        subset(&self.existing, &board.cols)
            && board.cols.iter().zip(self.existing.iter().zip(&self.cells)).all(|(&c, (&e, &f))| c & !(e | f) == 0)
    }

    /// `board` にミノ `loc` を置く手が、この図どおりか(図のマスへ図と同じミノを置き、Tスピンの T は最後)。
    pub fn accepts(&self, board: &Board, loc: &PieceLocation) -> bool {
        if !self.consistent(board) {
            return false;
        }
        let cells = loc.sorted_cells();
        let Some(i) = self.items.iter().position(|&(p, c)| p == loc.piece && c == cells) else { return false };
        // 置くマスが空いている(図のマスでも、既に別のミノで埋まっていれば置けない)
        if cells.iter().any(|&(x, y)| board.cols[x as usize] >> y & 1 == 1) {
            return false;
        }
        if self.last.contains(&i) {
            // 他のミノ(最後に入れる T 以外)をすべて置いた後
            let others = bits(
                self.items.iter().enumerate().filter(|(j, _)| !self.last.contains(j)).flat_map(|(_, (_, c))| c.iter().copied()),
            );
            return subset(&others, &board.cols);
        }
        true
    }

    /// 1巡目の図か(既に置いてあるブロックが無い)。
    pub fn is_first(&self) -> bool {
        self.existing.iter().all(|&c| c == 0)
    }
}

impl Template {
    /// 盤面がこのテンプレのどれかの図の途中か。
    pub fn consistent(&self, board: &Board) -> bool {
        self.forms.iter().any(|f| f.consistent(board))
    }

    /// `board` にミノ `loc` を置く手が、このテンプレのどれかの図どおりか。
    pub fn accepts(&self, board: &Board, loc: &PieceLocation) -> bool {
        self.forms.iter().any(|f| f.accepts(board, loc))
    }

    /// 空の盤面から、見えているミノ(操作ミノ・NEXT・HOLD)で1巡目の図のどれかを組めるか。
    /// 見えているミノを順に(HOLD を使って)図のマスへ置き続けられれば組めるとみなす(袋の残りは見ない)。
    pub fn buildable(&self, queue: &[Piece], hold: Option<Piece>, rules: &Rules) -> bool {
        self.forms.iter().filter(|f| f.is_first()).any(|f| build(f, Board::default(), queue, hold, rules))
    }
}

/// 図 `form` を、ミノ順 `queue`(先頭が操作ミノ)と HOLD で置き続けられるか(深さ優先)。
fn build(form: &Form, board: Board, queue: &[Piece], hold: Option<Piece>, rules: &Rules) -> bool {
    if subset(&form.cells, &board.cols) {
        return true; // 図を組み終えた
    }
    let Some((&current, rest)) = queue.split_first() else { return true }; // 見えているミノはすべて置けた
    let place = |piece: Piece, rest: &[Piece], hold: Option<Piece>| {
        generate(&board, piece, rules.spawn()).iter().any(|m| {
            form.accepts(&board, &m.placement.location) && {
                let mut b = board;
                b.place(&m.placement.location);
                b.full_lines() == 0 && build(form, b, rest, hold, rules)
            }
        })
    };
    if place(current, rest, hold) {
        return true;
    }
    match hold {
        Some(h) if h != current => place(h, rest, Some(current)),
        Some(_) => false,
        None => match rest.split_first() {
            Some((&next, rest2)) => place(next, rest2, Some(current)),
            None => false,
        },
    }
}

/// 探索に渡す序盤の作戦。
#[derive(Clone, Debug, Default, PartialEq)]
pub enum Opening {
    /// 序盤ではない(または序盤が終わった)
    #[default]
    None,
    /// 素の状態の序盤(テンプレは使わず、中あけ REN も組まない)
    Plain,
    /// テンプレの序盤(図どおりの手に加点する)
    Template(Arc<Template>),
    /// 開幕TSD の序盤(TSD に加点する。最初に消すか14手で終わる。2026-11-05)
    Tsd,
}

/// 開幕に何を目指すか(2026-11-05・ユーザー。全自動の設定で選ぶ)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OpeningMode {
    /// これまでの既定: TD系テンプレか素の状態を半々で選ぶ
    #[default]
    TemplateOrPlain,
    /// TDテンプレ・開幕TSD・種3C4W から均等に選ぶ
    Random,
    /// TDテンプレ(組めるものが無ければ素の状態)
    Td,
    /// 開幕TSD
    Tsd,
    /// 種3C4W(序盤の作戦を使わず、最初から中あけの評価で種3を作る)
    Seed3,
}

impl OpeningMode {
    /// bot の `--opening` の値から。知らない値は None。
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "default" => Some(Self::TemplateOrPlain),
            "random" => Some(Self::Random),
            "td" => Some(Self::Td),
            "tsd" => Some(Self::Tsd),
            "seed3" => Some(Self::Seed3),
            _ => None,
        }
    }
}

impl Opening {
    pub fn is_active(&self) -> bool {
        !matches!(self, Opening::None)
    }
}

#[derive(Clone, Debug, PartialEq)]
enum State {
    /// 序盤ではない(次に空の盤面を見たら選び直す)
    Done,
    /// 素の状態。選んでから置いたミノの数
    Plain(u32),
    Template(Arc<Template>),
    /// 開幕TSD。選んでから置いたミノの数
    Tsd(u32),
}

/// 対局の序盤の作戦を選び、進み具合を追う(bot と対戦シミュレーターで使う)。
///
/// 局面を受け取るたびに `observe` を呼ぶ。空の盤面を見たら(対局の始め。パフェの後も含む)、半々で
/// テンプレか素の状態を選ぶ。テンプレは図から外れるかラインを消すまで、素の状態は14手まで続く。
#[derive(Clone, Debug)]
pub struct OpeningTracker {
    state: State,
    /// 選んだときから、直前に見た盤面
    last_board: Option<Board>,
    rng: u64,
    mode: OpeningMode,
}

impl OpeningTracker {
    pub fn new(seed: u64) -> Self {
        Self::with_mode(seed, OpeningMode::default())
    }

    pub fn with_mode(seed: u64, mode: OpeningMode) -> Self {
        OpeningTracker { state: State::Done, last_board: None, rng: seed | 1, mode }
    }

    fn next_random(&mut self) -> u64 {
        // xorshift64
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 7;
        self.rng ^= self.rng << 17;
        self.rng
    }

    /// 局面を受け取ったときに呼ぶ(作戦を選ぶ・進める・終える)。返り値をそのまま `root.opening` に入れる。
    pub fn observe(&mut self, root: &Root, rules: &Rules) -> Opening {
        let board = root.state.board;
        let changed = self.last_board.is_some_and(|b| b != board);
        let fewer_cells = self.last_board.is_some_and(|b| count(&b) > count(&board));
        if board.is_empty() && !root.queue.is_empty() {
            let fresh = match &self.state {
                State::Done => true,
                State::Plain(n) | State::Tsd(n) => *n > 0,
                State::Template(_) => changed,
            };
            if fresh {
                self.choose(root, rules);
            }
        } else if changed {
            self.state = match std::mem::replace(&mut self.state, State::Done) {
                State::Plain(n) if n + 1 < PLAIN_PIECES => State::Plain(n + 1),
                State::Tsd(n) if !fewer_cells && n + 1 < PLAIN_PIECES => State::Tsd(n + 1),
                State::Template(t) if !fewer_cells && t.consistent(&board) => State::Template(t),
                _ => State::Done,
            };
        }
        self.last_board = Some(board);
        self.opening()
    }

    fn choose(&mut self, root: &Root, rules: &Rules) {
        let mode = match self.mode {
            OpeningMode::Random => [OpeningMode::Td, OpeningMode::Tsd, OpeningMode::Seed3][(self.next_random() % 3) as usize],
            mode => mode,
        };
        self.state = State::Plain(0);
        match mode {
            OpeningMode::Tsd => {
                self.state = State::Tsd(0);
                return;
            }
            OpeningMode::Seed3 => {
                self.state = State::Done;
                return;
            }
            OpeningMode::TemplateOrPlain if self.next_random().is_multiple_of(2) => return,
            _ => {}
        }
        let candidates: Vec<&Arc<Template>> =
            td_templates().iter().filter(|t| t.buildable(&root.queue, root.hold, rules)).collect();
        if !candidates.is_empty() {
            let i = (self.next_random() % candidates.len() as u64) as usize;
            self.state = State::Template(candidates[i].clone());
        }
    }

    /// 今の作戦。
    pub fn opening(&self) -> Opening {
        match &self.state {
            State::Done => Opening::None,
            State::Plain(_) => Opening::Plain,
            State::Template(t) => Opening::Template(t.clone()),
            State::Tsd(_) => Opening::Tsd,
        }
    }

    /// 選んだテンプレの名前(素の状態・序盤でなければ None)。記録用。
    pub fn template_name(&self) -> Option<&str> {
        match &self.state {
            State::Template(t) => Some(&t.name),
            _ => None,
        }
    }
}

fn count(board: &Board) -> u32 {
    board.cols.iter().map(|c| c.count_ones()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn td系テンプレ5種を読める() {
        let names: Vec<&str> = td_templates().iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["ホットケーキ積み", "くろみつ砲", "PC-Spin", "皐月積み", "ベーカリーTD"]);
        for t in td_templates() {
            assert!(t.forms.iter().any(|f| f.is_first()), "{} に1巡目の図が無い", t.name);
            assert!(t.forms.iter().any(|f| !f.last.is_empty()), "{} に Tスピンの図が無い", t.name);
        }
    }

    #[test]
    fn 図どおりの手だけを受け付ける() {
        let pancake = &td_templates()[0];
        let first = pancake.forms.iter().find(|f| f.is_first()).unwrap();
        let empty = Board::default();
        // 図の一番下にあるミノ(支えが要らない)を図の位置に置くのは図どおり
        let (piece, cells) = *first.items.iter().find(|(_, c)| c.iter().any(|&(_, y)| y == 0)).unwrap();
        let loc = tetrisai_core::generate(&empty, piece, Rules::ppt2().spawn())
            .into_iter()
            .map(|m| m.placement.location)
            .find(|l| l.sorted_cells() == cells)
            .expect("図のミノを置く手が無い");
        assert!(pancake.accepts(&empty, &loc));
        // 同じミノでも図に無い位置は図どおりではない
        let other = tetrisai_core::generate(&empty, piece, Rules::ppt2().spawn())
            .into_iter()
            .map(|m| m.placement.location)
            .find(|l| !pancake.forms.iter().any(|f| f.items.iter().any(|&(p, c)| p == piece && c == l.sorted_cells())))
            .unwrap();
        assert!(!pancake.accepts(&empty, &other));
        // 置いた後の盤面は図の途中
        let mut b = empty;
        b.place(&loc);
        assert!(pancake.consistent(&b));
        assert!(!pancake.consistent(&Board::from_rows(&["##########"])));
    }

    #[test]
    fn 空の盤面で半々に選ぶ() {
        let rules = Rules::ppt2();
        let root = Root {
            queue: vec![Piece::I, Piece::O, Piece::T, Piece::L, Piece::J, Piece::S],
            ..Default::default()
        };
        let (mut plain, mut template) = (0, 0);
        for seed in 1..=200u64 {
            let mut tracker = OpeningTracker::new(seed * 0x9e37_79b9_7f4a_7c15);
            match tracker.observe(&root, &rules) {
                Opening::Plain => plain += 1,
                Opening::Template(_) => template += 1,
                Opening::None => panic!("空の盤面で序盤にならない"),
                Opening::Tsd => panic!("既定では開幕TSDを選ばない"),
            }
        }
        // 組めるテンプレがあるミノ順なので、おおよそ半々
        assert!((70..=130).contains(&template), "テンプレ {template} / 素 {plain}");
    }

    #[test]
    fn 素の状態は14手で終わる() {
        let rules = Rules::ppt2();
        let mut root = Root { queue: vec![Piece::I; 6], ..Default::default() };
        let mut tracker = (1..)
            .map(OpeningTracker::new)
            .find(|t| matches!(t.clone().observe(&root, &rules), Opening::Plain))
            .unwrap();
        assert_eq!(tracker.observe(&root, &rules), Opening::Plain);
        // 同じ盤面を何度受け取っても手数は進まない
        assert_eq!(tracker.observe(&root, &rules), Opening::Plain);
        for n in 1..=PLAIN_PIECES {
            // 1手ごとに盤面を変える(左下から1マスずつ埋める。消えない)
            root.state.board.cols[(n as usize - 1) % 9] |= 1 << ((n as usize - 1) / 9);
            let o = tracker.observe(&root, &rules);
            assert_eq!(o.is_active(), n < PLAIN_PIECES, "{n} 手目");
        }
    }
}
