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
//! 一致するものが自然に選ばれる(TBA の openers.py と同じ考え方)。図から外れた手か、`TEMPLATE_CLEARS` 回目に
//! ラインを消した手(TST→TSD を撃ち切った)で序盤は終わる。おじゃまがせり上がった時は図をずらして続ける(2026-11-08)。序盤の間は中あけ REN の評価を使わない(`EvalContext::opening`)。

use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, OnceLock};

use serde::Deserialize;
use tetrisai_core::board::WIDTH;
use tetrisai_core::{generate, Board, Piece, PieceLocation, PlayerState, Rules, Spin};

use crate::search::Root;

/// 素の状態の序盤の手数(TD テンプレで撃つまでと同じくらい: 2巡)
pub const PLAIN_PIECES: u32 = 14;

/// テンプレの間に消してよい回数の上限(TST→TSD→パフェ。パフェは2回に分けて消すこともある)。
/// 【2026-11-08・ユーザー「テンプレだけに関しては TST→TSD までは打ち切ってください」「相手が開幕C4Wの時こそ TD テンプレが
/// 活きる。TST→TSD→パフェで21段せり上げにより相手の構想をつぶせる」】図から外れるか、パフェで盤面が空になるまで続ける。
/// これは図が想定しない消し方を延々と続けないための上限。
pub const TEMPLATE_CLEARS: u32 = 5;

/// `Template::continue_rate` で、分かっているミノの先に足して試す見えていないミノの数(環境変数 TD_LOOKAHEAD。比べる時用)
fn continue_lookahead() -> usize {
    static N: OnceLock<usize> = OnceLock::new();
    *N.get_or_init(|| std::env::var("TD_LOOKAHEAD").ok().and_then(|s| s.parse().ok()).unwrap_or(4))
}

/// おじゃまのせり上がりで図をずらす段数の上限
const MAX_RISE: u32 = 12;

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
    /// ミノごとの、置く前に消えている行(図の座標の行のビット。TBA の gap_rows。消える前の座標で間の行をまたいで描かれた
    /// ミノ。2026-11-10)。行が消えて図をずらすと減り、空になれば普通のミノ
    pub gaps: Vec<u64>,
    /// 図の下にせり上がったおじゃまの行(この行のブロックは図に無くてもよい。行ごとのビット)
    pub floor: u64,
}

/// テンプレ1つ(左右反転の図を含む)。
#[derive(Clone, Debug, PartialEq)]
pub struct Template {
    pub name: String,
    pub forms: Vec<Form>,
    /// これまでに撃った(ラインを消した)回数。`TEMPLATE_CLEARS` 回でテンプレは終わり
    pub clears: u32,
    /// せり上がりで図を上にずらした段数
    pub rise: u32,
    /// TST の後、TSD を撃たずにパフェ(TST→パフェ)を狙うテンプレか(データの `pc_after_tst`。2026-11-13、DESIGN.md 8.14)。
    /// 偽なら TST の後は TSD を撃つまでのパフェを組めたとみなさない(search.rs の `tsd_pending`)
    pub pc_after_tst: bool,
}

#[derive(Deserialize)]
struct RawTemplate {
    name: String,
    forms: Vec<RawForm>,
    #[serde(default)]
    pc_after_tst: bool,
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
    /// 置く前に消えている行(図の y)。【2026-11-10】行が消えた後に置くミノも書き出した
    #[serde(default)]
    gap_rows: Vec<i8>,
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

/// 組めれば優先して狙うテンプレ。優先度の順(2026-11-07・ユーザー)。迷走砲の図は Iミノが左側のものだけ。
/// 【2026-11-11・ユーザー「パフェができる確率が高いものを用いたい」】1人で組ませた時(`td_solo_probe`、1000局、1巡目を
/// ソフトドロップなしで組める局の中で)の TST→TSD→パフェ は、はちみつ砲 38.3%・くろみつ砲 36.2%・迷走砲 23.8%・
/// ホットケーキ積み 21.1%・PC-Spin 20.5%・ベーカリーTD 17.1%・皐月積み 9.6%。くろみつ砲を迷走砲の前に入れた
/// (全体の TST→TSD→パフェ 30.3% → 37.7%。くろみつ砲は 98% の並びで1巡目をソフトドロップなしで組める)。
/// 【2026-11-11・実機 debug_log_20261111_210217 の9局目】1巡目を組めるかの判定(`build`)が HOLD に残したミノを見ておらず、
/// くろみつ砲を組めない並びでも選んでいた。直すと、くろみつ砲は 75% の並びで組めて、組めば TST 100%・TST→TSD→パフェ 54.0%
/// (はちみつ砲 42.5%・迷走砲 34.0%)なので、くろみつ砲を先にした(全体の TST→TSD→パフェ 46.1% → 49.6%、1人で1000局)。
/// 【2026-11-13・DESIGN.md 8.14 段階2b】土台から自動で探した「自動探索TD-A」(`data/td_generated.json`)を、くろみつ砲を組めない
/// 並び(5%ほど)の次の候補にした。土台と TST の場所は迷走砲(Zも置く形)の左右反転と同じで、2・3巡目の図だけを自動で作ったもの。くろみつ砲を組める並びではすべて組めて1人の TST→TSD→パフェもほぼ同じだが、先に選ぶと
/// 開幕C4W 相手の勝率が 21.7% → 20.2% に下がった(6000局。TSD を飛ばすパフェの逃げ道がほぼ無いため。開幕TSD 相手は 35.3% → 38.8%)。
const PRIORITY_TEMPLATES: [&str; 4] = ["くろみつ砲", "自動探索TD-A", "はちみつ砲", "迷走砲"];

/// 優先テンプレがどれも組めない時に狙う順(2026-11-11)。1人で組ませた時(`td_solo_probe`、1000局)の TST→TSD→パフェは
/// PC-Spin 31.4%・ベーカリーTD 28.3%・ホットケーキ積み 26.9%・皐月積み 12.0%。
const OTHER_ORDER: [&str; 4] = ["PC-Spin", "ベーカリーTD", "ホットケーキ積み", "皐月積み"];

/// 優先して狙うテンプレの名前(優先度の順)。環境変数 TD_PRIORITY(','区切り)で入れ替えられる(比べる時用)
fn priority_names() -> &'static [String] {
    static NAMES: OnceLock<Vec<String>> = OnceLock::new();
    NAMES.get_or_init(|| match std::env::var("TD_PRIORITY") {
        Ok(list) => list.split(',').map(str::to_string).collect(),
        Err(_) => PRIORITY_TEMPLATES.iter().map(|s| s.to_string()).collect(),
    })
}

/// ほかの TD系テンプレ(`EXCLUDED_TEMPLATES` と優先テンプレは除く)。
pub fn td_templates() -> &'static [Arc<Template>] {
    static TEMPLATES: OnceLock<Vec<Arc<Template>>> = OnceLock::new();
    TEMPLATES.get_or_init(|| {
        all_templates().iter().filter(|t| !priority_names().contains(&t.name)).cloned().collect()
    })
}

/// 優先して狙うテンプレ(優先度の順)。
pub fn priority_templates() -> &'static [Arc<Template>] {
    static TEMPLATES: OnceLock<Vec<Arc<Template>>> = OnceLock::new();
    TEMPLATES.get_or_init(|| {
        priority_names()
            .iter()
            .filter_map(|name| all_templates().iter().find(|t| t.name == *name).cloned())
            .collect()
    })
}

/// engine に埋め込んだ図を初回に読む(`EXCLUDED_TEMPLATES` は除く)。
fn all_templates() -> &'static [Arc<Template>] {
    static TEMPLATES: OnceLock<Vec<Arc<Template>>> = OnceLock::new();
    TEMPLATES.get_or_init(|| {
        let mut raw: Vec<RawTemplate> =
            serde_json::from_str(include_str!("../data/td_templates.json")).expect("td_templates.json を読めない");
        // 自動で探して作ったテンプレ(`examples/td_gen.rs`、DESIGN.md 8.14。TBA の図には無く、全自動だけで使う)
        let generated: Vec<RawTemplate> =
            serde_json::from_str(include_str!("../data/td_generated.json")).expect("td_generated.json を読めない");
        raw.extend(generated);
        raw.into_iter()
            .filter(|t| !EXCLUDED_TEMPLATES.contains(&t.name.as_str()))
            .map(|t| {
                let forms = t
                    .forms
                    .into_iter()
                    .filter_map(|f| {
                        let mut items = Vec::new();
                        let mut last = Vec::new();
                        let mut gaps = Vec::new();
                        for item in &f.items {
                            let cells: [(i8, i8); 4] =
                                item.cells.iter().map(|c| (c[0], c[1])).collect::<Vec<_>>().try_into().ok()?;
                            if item.last {
                                last.push(items.len());
                            }
                            items.push((Piece::from_char(item.piece)?, sorted(cells)));
                            gaps.push(item.gap_rows.iter().fold(0u64, |a, &y| a | 1 << y));
                        }
                        if last.is_empty() {
                            last = firing_t(&items, &f.existing);
                        }
                        Some(Form {
                            section: f.section,
                            existing: bits(f.existing.iter().map(|c| (c[0], c[1]))),
                            cells: bits(items.iter().flat_map(|(_, c)| c.iter().copied())),
                            items,
                            last,
                            gaps,
                            floor: 0,
                        })
                    })
                    .collect();
                Arc::new(Template { name: t.name, forms, clears: 0, rise: 0, pc_after_tst: t.pc_after_tst })
            })
            .collect()
    })
}

/// 「最後に置く」印の無い図で、撃つ T(図をすべて置くと、その T の行が2行以上そろい、その行にほかのミノのマスもある)を
/// 最後に置くミノとする。図に T が1つだけの時に限る(TST と TSD の T が両方ある図は、どちらが先かを決めないため印を足さない)。
///
/// 【2026-11-08・実機 debug_log_20261108_202040 の1P 局3】はちみつ砲の2巡目の図(78個中40個)に印が無く、TST の溝へ
/// T を先に入れる手(Tスピンでない1列消し)も図の手として受け付けて、TST を崩した。
fn firing_t(items: &[(Piece, [(i8, i8); 4])], existing: &[[i8; 2]]) -> Vec<usize> {
    let ts: Vec<usize> = items.iter().enumerate().filter(|(_, (p, _))| *p == Piece::T).map(|(i, _)| i).collect();
    let [i] = ts[..] else { return Vec::new() };
    let all: Vec<(i8, i8)> =
        existing.iter().map(|c| (c[0], c[1])).chain(items.iter().flat_map(|(_, c)| c.iter().copied())).collect();
    let mut rows: Vec<i8> = items[i].1.iter().map(|&(_, y)| y).collect();
    rows.sort_unstable();
    rows.dedup();
    let full: Vec<i8> = rows.into_iter().filter(|&y| all.iter().filter(|&&(_, cy)| cy == y).count() >= WIDTH).collect();
    // そろう行に、ほかのミノのマスがある(そのミノを先に置かないと撃てない)。既に置いてあるブロックと T だけでそろう図
    // (3巡目の TSD → パフェ。T を先に撃ってから残りを置く)には印を付けない
    let needs_others =
        items.iter().enumerate().any(|(j, (_, c))| j != i && c.iter().any(|&(_, y)| full.contains(&y)));
    if full.len() >= 2 && needs_others { vec![i] } else { Vec::new() }
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
            && board.cols.iter().zip(self.existing.iter().zip(&self.cells)).all(|(&c, (&e, &f))| c & !(e | f | self.floor) == 0)
    }

    /// 図の途中で `rows`(図の座標の行。昇順)が揃って消えた後の図。消えた行より上のマスは下へずれる。
    /// 消えた行に掛かるミノは置き終えているので、置くミノから外す(残ったマスは `cells` に残す)。
    /// 【2026-11-08】図は消える前の座標で描かれている(TSD の T と、その後に置くパフェのミノが同じ図にある。TBA の
    /// openers.py の shift_cells_for_clears と同じ考え方)。
    fn collapsed(&self, rows: &[u32]) -> Form {
        let squeeze = |mut c: u64| {
            for &r in rows.iter().rev() {
                c = (c & ((1u64 << r) - 1)) | ((c >> (r + 1)) << r);
            }
            c
        };
        let mut items = Vec::new();
        let mut last = Vec::new();
        let mut gaps = Vec::new();
        for (i, &(piece, cells)) in self.items.iter().enumerate() {
            if cells.iter().any(|&(_, y)| rows.contains(&(y as u32))) {
                continue;
            }
            if self.last.contains(&i) {
                last.push(items.len());
            }
            let down = |y: i8| y - rows.iter().filter(|&&r| (r as i8) < y).count() as i8;
            items.push((piece, cells.map(|(x, y)| (x, down(y)))));
            // 消えた行を除いた、まだ消えていない「置く前に消える行」
            let removed = rows.iter().fold(0u64, |a, &r| a | 1 << r);
            gaps.push(squeeze(self.gaps.get(i).copied().unwrap_or(0) & !removed));
        }
        Form {
            section: self.section.clone(),
            existing: self.existing.map(squeeze),
            cells: self.cells.map(squeeze),
            items,
            last,
            gaps,
            floor: squeeze(self.floor),
        }
    }

    /// 図の完成形で揃う行(消えうる行。おじゃまの行は除く)。
    fn full_rows(&self) -> Vec<u32> {
        (0..64u32)
            .filter(|&r| self.floor >> r & 1 == 0)
            .filter(|&r| self.existing.iter().zip(&self.cells).all(|(&e, &c)| (e | c) >> r & 1 == 1))
            .collect()
    }

    /// 図を `rows` 段上にずらしたもの(下に増えた行はおじゃま)。
    fn shifted(&self, rows: u32) -> Form {
        Form {
            section: self.section.clone(),
            existing: self.existing.map(|c| c << rows),
            cells: self.cells.map(|c| c << rows),
            items: self.items.iter().map(|&(p, c)| (p, c.map(|(x, y)| (x, y + rows as i8)))).collect(),
            last: self.last.clone(),
            gaps: self.gaps.iter().map(|g| g << rows).collect(),
            floor: (self.floor << rows) | ((1u64 << rows) - 1),
        }
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
        // 行が消えた後に置くミノは、その行が消えるまで置かない(消える前の座標では分かれていて、置く手と合わないので普通は来ない)
        if self.gaps.get(i).is_some_and(|&g| g != 0) {
            return false;
        }
        if self.last.contains(&i) {
            // 他のミノ(最後に入れる T 以外。行が消えた後に置くミノも除く)をすべて置いた後
            let others = bits(
                self.items
                    .iter()
                    .enumerate()
                    .filter(|(j, _)| !self.last.contains(j) && self.gaps.get(*j).is_none_or(|&g| g == 0))
                    .flat_map(|(_, (_, c))| c.iter().copied()),
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
    /// 盤面が変わった後も、このテンプレを続けられるか。続けられるなら続けるテンプレ(撃った回数・ずらした段数を進めたもの)。
    /// `cleared` はラインを消して盤面のブロックが減ったか。
    ///
    /// 【2026-11-08・ユーザー「相手の攻撃が早いと TST をあきらめて別の形にしようとする癖がある。テンプレだけに関しては
    /// TST→TSD までは打ち切ってください」】以前は、ラインを消すか、おじゃまがせり上がって図とずれた時点でテンプレを終えていた。
    /// 下におじゃまの行が増えた時は図を同じ段数だけ上にずらして続ける。`lines` はこの手で消した行数(消していなければ 0)。
    ///
    /// 【2026-11-08・ユーザー「TST→TSD→パフェで21段せり上げにより相手の構想をつぶす」】ラインを消した後は、
    /// 消した後の盤面から始まる図(TST の後の3巡目など)と、図の途中で消えた行を取り除いた図(TSD の後のパフェなど)で続ける。
    pub fn follow(self: &Arc<Self>, board: &Board, lines: u32) -> Option<Arc<Template>> {
        let base = if lines > 0 {
            if self.clears + 1 >= TEMPLATE_CLEARS {
                return None;
            }
            // 消した後の盤面から始まる図(1巡目の図は空の盤面から始まるので使わない)。
            // 【2026-11-13】合う図だけを複製する(図の多いテンプレで、すべて複製してから絞ると先読みが重かった)
            let mut forms: Vec<Form> = all_templates().iter().find(|t| t.name == self.name).map_or(Vec::new(), |t| {
                t.forms
                    .iter()
                    .filter_map(|f| {
                        if self.rise > 0 {
                            Some(f.shifted(self.rise)).filter(|s| !s.is_first() && s.consistent(board))
                        } else {
                            (!f.is_first() && f.consistent(board)).then(|| f.clone())
                        }
                    })
                    .collect()
            });
            for f in &self.forms {
                let full = f.full_rows();
                for rows in combinations(&full, lines as usize) {
                    let c = f.collapsed(&rows);
                    if c.consistent(board) && !c.items.is_empty() && !forms.contains(&c) {
                        forms.push(c);
                    }
                }
            }
            Arc::new(Template {
                name: self.name.clone(),
                forms,
                clears: self.clears + 1,
                rise: self.rise,
                pc_after_tst: self.pc_after_tst,
            })
        } else {
            self.clone()
        };
        if base.consistent(board) {
            return Some(base);
        }
        if lines > 0 {
            return None;
        }
        let garbage_row = |y: u32| board.cols.iter().filter(|&&c| c >> y & 1 == 1).count() == WIDTH - 1;
        for rows in 1..=MAX_RISE {
            if !garbage_row(rows - 1) {
                break;
            }
            let shifted = Template {
                name: base.name.clone(),
                forms: base.forms.iter().map(|f| f.shifted(rows)).collect(),
                clears: base.clears,
                rise: base.rise + rows,
                pc_after_tst: base.pc_after_tst,
            };
            if shifted.consistent(board) {
                return Some(Arc::new(shifted));
            }
        }
        None
    }

    /// 盤面 `board` の後も図どおりに戻れない図(図の外にブロックがある図。ラインを消すまで合わない)を除いたテンプレ。
    /// 除くものが無ければそのまま。ラインを消した後の図は `follow` が図のデータから作り直すので、除いても困らない。
    ///
    /// 【2026-11-13】図の多いテンプレ(自動探索TD-D は8千個)では、探索の中で図をすべて調べる所(`accepts`・`targets`)が
    /// 重い。実際に1手置くたび(`OpeningTracker`)に絞る。
    pub fn narrowed(self: &Arc<Self>, board: &Board) -> Arc<Template> {
        let fits = |f: &Form| board.cols.iter().zip(f.existing.iter().zip(&f.cells)).all(|(&c, (&e, &x))| c & !(e | x | f.floor) == 0);
        if self.forms.iter().all(fits) {
            return self.clone();
        }
        Arc::new(Template {
            name: self.name.clone(),
            forms: self.forms.iter().filter(|f| fits(f)).cloned().collect(),
            clears: self.clears,
            rise: self.rise,
            pc_after_tst: self.pc_after_tst,
        })
    }

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
        self.forms.iter().filter(|f| f.is_first()).any(|f| build(f, Board::default(), queue, hold, rules, false))
    }

    /// `buildable` の、1巡目をソフトドロップなし(ハードドロップだけ)で組める場合。
    pub fn buildable_hard_drop(&self, queue: &[Piece], hold: Option<Piece>, rules: &Rules) -> bool {
        self.forms.iter().filter(|f| f.is_first()).any(|f| build(f, Board::default(), queue, hold, rules, true))
    }

    /// 盤面 `board` から、分かっているミノ(`queue` の後に袋の残り `bag`)で、1巡目の図をソフトドロップなしで組み続けられるか
    /// (`buildable_hard_drop_known` と同じ数え方。盤面が図の途中でなければ false)。
    /// 【2026-11-11】1巡目の間は、置いた後もソフトドロップなしで組み続けられる手だけを図どおりとみなす(探索が使う)。
    pub fn continues_hard_drop(&self, board: &Board, queue: &[Piece], hold: Option<Piece>, bag: &[Piece], rules: &Rules) -> bool {
        let orders = if bag.is_empty() || bag.len() > 3 { vec![Vec::new()] } else { permutations(bag) };
        orders.iter().all(|order| {
            let mut known = queue.to_vec();
            known.extend_from_slice(order);
            self.forms.iter().filter(|f| f.is_first() && f.consistent(board)).any(|f| build(f, *board, &known, hold, rules, true))
        })
    }

    /// 盤面 `board` から、分かっているミノ(`queue`)の後に、袋の残り(`bag`)とその次の袋をランダムに並べて足した
    /// `samples` 通りのミノ順で、すべてを図どおりに置き続けられる割合(0〜1)。足すのは見えていないミノ `continue_lookahead` 個まで。
    /// 消した後は図を進めて続ける(TST の後の TSD・パフェの段も)。パフェで盤面が空になるか、撃った後にテンプレが終われば
    /// そこで置き続けられたとみなす。`hard` なら T 以外はソフトドロップなし。`need_tsd` なら、TSD(Tスピンダブル以上)を撃つ前の
    /// パフェは組めたとみなさない(TST の後)。並べ方は盤面から決める(同じ局面では同じ値)。
    ///
    /// 【2026-11-11・ユーザー「2巡目でもソフトドロップしないのが好ましい。パフェができる確率が高いものを用いたい」】
    /// 探索が図どおりの手から選ぶ時に使う(search.rs の `template_allowed`)。
    #[allow(clippy::too_many_arguments)]
    pub fn continue_rate(
        self: &Arc<Self>,
        board: &Board,
        queue: &[Piece],
        hold: Option<Piece>,
        bag: &[Piece],
        rules: &Rules,
        samples: u32,
        hard: bool,
        need_tsd: bool,
    ) -> f32 {
        let mut rng = board.cols.iter().fold(0x2545_F491_4F6C_DD1Du64, |a, &c| (a ^ c).wrapping_mul(0x9E37_79B9_7F4A_7C15)) | 1;
        let mut shuffled = |items: &[Piece]| {
            let mut v = items.to_vec();
            for i in (1..v.len()).rev() {
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                v.swap(i, (rng % (i as u64 + 1)) as usize);
            }
            v
        };
        let mut ok = 0;
        // 標本どうしで、置き方の使い回しと「置き続けられない局面」を分け合う(残りのミノも含めて覚えるので混ざらない)
        let mut memo = WalkMemo::default();
        for _ in 0..samples {
            let mut seq = queue.to_vec();
            seq.extend(shuffled(bag));
            seq.extend(shuffled(&Piece::ALL));
            seq.truncate(queue.len() + continue_lookahead());
            memo.nodes = 0;
            ok += walk(self, *board, &seq, hold, rules, hard, need_tsd, &mut memo) as u32;
            WALK_NODES_MAX.fetch_max(memo.nodes, std::sync::atomic::Ordering::Relaxed);
        }
        ok as f32 / samples.max(1) as f32
    }

    /// 見えているミノに、袋の残り(`bag`。NEXT の後で今の袋に残っているミノ)も足して、1巡目をソフトドロップなしで組めるか。
    /// 袋の残りが2〜3個で来る順が分からない時は、どの順でも組めること。4個以上か空(次は新しい袋)なら見えているミノだけで見る。
    ///
    /// 【2026-11-11・ユーザー「開幕で迷走砲を選んだ時、1巡目にソフトドロップを伴うことがある。ソフトドロップになってしまう場合は
    /// 採用しないようにしたい。はちみつ砲も同様。TDテンプレの中で1巡目にソフトドロップが不要になるものを選ぶ」】開幕では NEXT の後の
    /// 袋の残りは1個なので、1巡目の7個がすべて分かる。以前は見えている6個だけで確かめ、7個目でソフトドロップが要る並びでも選んでいた。
    pub fn buildable_hard_drop_known(&self, queue: &[Piece], hold: Option<Piece>, bag: &[Piece], rules: &Rules) -> bool {
        if bag.is_empty() || bag.len() > 3 {
            return self.buildable_hard_drop(queue, hold, rules);
        }
        permutations(bag).iter().all(|order| {
            let mut known = queue.to_vec();
            known.extend_from_slice(order);
            self.buildable_hard_drop(&known, hold, rules)
        })
    }
}

/// `walk` が使い回す、盤面とミノごとの置き方(同じ盤面を何度も通るので)。
type MoveCache = HashMap<(Board, Piece), Rc<Vec<tetrisai_core::Move>>>;

/// `walk` が使い回す、テンプレ(Arc の番地)・盤面・ミノごとの図どおりの置き場所。
type TargetCache = HashMap<(usize, Board, Piece), Rc<Vec<[(i8, i8); 4]>>>;

/// `walk` の作業用(置き続けられないと分かった局面と、置き方の使い回し)。
#[derive(Default)]
struct WalkMemo {
    /// 置き続けられないと分かった局面(盤面・HOLD・残りのミノ・撃った回数)
    dead: HashSet<(Board, Option<Piece>, Vec<Piece>, u32, bool)>,
    moves: MoveCache,
    /// テンプレ(Arc の番地)・盤面・ミノごとの図どおりの置き場所。【2026-11-13】図の多いテンプレ(自動探索TD-D は8千個)で、
    /// 同じ局面を通るたびに図をすべて調べ直して探索1回が平均 0.6 秒・最大 34 秒かかったので覚えておく
    targets: TargetCache,
    /// テンプレ(Arc の番地)・消した後の盤面・消した行数ごとの続くテンプレ(`follow`。消した後は図を作り直すので重い)。
    /// 続くテンプレをここで持っておくので、番地は使い終わるまで変わらない
    follows: HashMap<(usize, Board, u32), Option<Arc<Template>>>,
    /// 今の1通りのミノ順で調べた局面の数(`WALK_NODES_MAX` 用)
    nodes: u32,
}

/// これまでに1通りのミノ順で `walk` が調べた局面の数の最大(調べる用)
pub static WALK_NODES_MAX: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

impl Template {
    /// 盤面 `board` に `piece` を図どおりに置けるマス(4マスを並べたもの)。`accepts` をまとめて調べる形。
    fn targets(&self, board: &Board, piece: Piece) -> Vec<[(i8, i8); 4]> {
        let mut out: Vec<[(i8, i8); 4]> = Vec::new();
        for f in self.forms.iter().filter(|f| f.consistent(board)) {
            for (i, &(p, cells)) in f.items.iter().enumerate() {
                if p != piece || out.contains(&cells) {
                    continue;
                }
                if cells.iter().any(|&(x, y)| board.cols[x as usize] >> y & 1 == 1) {
                    continue;
                }
                if f.gaps.get(i).is_some_and(|&g| g != 0) {
                    continue;
                }
                if f.last.contains(&i) {
                    let others = bits(
                        f.items
                            .iter()
                            .enumerate()
                            .filter(|(j, _)| !f.last.contains(j) && f.gaps.get(*j).is_none_or(|&g| g == 0))
                            .flat_map(|(_, (_, c))| c.iter().copied()),
                    );
                    if !subset(&others, &board.cols) {
                        continue;
                    }
                }
                out.push(cells);
            }
        }
        out
    }
}

/// `Template::continue_rate` の1通りのミノ順を、深さ優先で図どおりに置き続けられるか。
#[allow(clippy::too_many_arguments)]
fn walk(
    t: &Arc<Template>,
    board: Board,
    queue: &[Piece],
    hold: Option<Piece>,
    rules: &Rules,
    hard: bool,
    need_tsd: bool,
    memo: &mut WalkMemo,
) -> bool {
    let Some((&current, rest)) = queue.split_first() else { return true }; // 分かっているミノはすべて置けた
    let key = (board, hold, queue.to_vec(), t.clears, need_tsd);
    if memo.dead.contains(&key) {
        return false;
    }
    memo.nodes += 1;
    let mut options = vec![(current, rest, hold)];
    match hold {
        Some(h) if h != current => options.push((h, rest, Some(current))),
        None => {
            if let Some((&next, rest2)) = rest.split_first() {
                if next != current {
                    options.push((next, rest2, Some(current)));
                }
            }
        }
        _ => {}
    }
    let filled: u32 = board.cols.iter().map(|c| c.count_ones()).sum();
    let tkey = Arc::as_ptr(t) as usize;
    for (piece, rest, hold) in options {
        let targets = memo.targets.entry((tkey, board, piece)).or_insert_with(|| Rc::new(t.targets(&board, piece))).clone();
        // パフェになりうるのは、置いた後のマスの数が10の倍数の時だけ
        let maybe_pc = (filled + 4).is_multiple_of(10);
        if targets.is_empty() && piece != Piece::T && !maybe_pc {
            continue;
        }
        let moves = memo.moves.entry((board, piece)).or_insert_with(|| Rc::new(generate(&board, piece, rules.spawn()))).clone();
        // 1巡目(まだ撃っておらず盤面が28マスに満たない間)は、`hard` でなくても T 以外のソフトドロップを使わない(探索の
        // 1巡目の縛り `first_stage_soft` と同じ)。【2026-11-11・実機 debug_log_20261111_210217 の9局目】これが無いと、1巡目に
        // ソフトドロップが要る道筋を「置き続けられる」と数えて選び、次の手で縛りの手が無くなってテンプレが崩れた
        let first_stage = t.clears == 0 && filled < 28;
        for m in moves.iter() {
            if (hard || first_stage) && piece != Piece::T && m.soft_drop_sections > 0 {
                continue;
            }
            let on_form = targets.contains(&m.placement.location.sorted_cells());
            if !on_form && piece != Piece::T && !maybe_pc {
                continue;
            }
            let mut s = PlayerState::new(board);
            let r = s.apply(&m.placement, rules);
            // 図に無くても Tスピンダブル以上とパフェは撃てる(探索と同じ)
            let shot = r.perfect_clear || (r.spin == Spin::Full && r.lines >= 2);
            if !shot && !on_form {
                continue;
            }
            let tsd = r.spin == Spin::Full && r.lines >= 2;
            // TST の後、TSD を撃たずにパフェにする道筋は組めたとみなさない
            if r.perfect_clear {
                if need_tsd && !tsd {
                    continue;
                }
                return true;
            }
            let need_tsd = need_tsd && !tsd;
            let next = memo.follows.entry((tkey, s.board, r.lines)).or_insert_with(|| t.follow(&s.board, r.lines)).clone();
            match next {
                Some(next) => {
                    if walk(&next, s.board, rest, hold, rules, hard, need_tsd, memo) {
                        return true;
                    }
                }
                // 撃った後にテンプレが終わった(図の想定より先へ進んだ)
                None if r.lines > 0 && shot && !need_tsd => return true,
                None => {}
            }
        }
    }
    memo.dead.insert(key);
    false
}

/// 図 `form` を、ミノ順 `queue`(先頭が操作ミノ)と HOLD で置き続けられるか(深さ優先)。
fn build(form: &Form, board: Board, queue: &[Piece], hold: Option<Piece>, rules: &Rules, hard_only: bool) -> bool {
    if subset(&form.cells, &board.cols) {
        return true; // 図を組み終えた
    }
    let place = |piece: Piece, rest: &[Piece], hold: Option<Piece>| {
        generate(&board, piece, rules.spawn()).iter().any(|m| {
            (!hard_only || m.soft_drop_sections == 0) && form.accepts(&board, &m.placement.location) && {
                let mut b = board;
                b.place(&m.placement.location);
                b.full_lines() == 0 && build(form, b, rest, hold, rules, hard_only)
            }
        })
    };
    // 見えているミノはすべて置けた。HOLD に残したミノも、図を組み終えていなければ今の盤面に置けること
    // 【2026-11-11・実機 debug_log_20261111_210217 の9局目(くろみつ砲、S T I Z L O J)】以前は HOLD に1巡目のミノ(I)を残したまま
    // 「組める」とみなし、最後に I をハードドロップで入れられずに図から外れた
    let Some((&current, rest)) = queue.split_first() else {
        return hold.is_none_or(|h| place(h, &[], None));
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
    /// 開幕パフェ(pco.rs が選んだ位置に置く。2026-11-09)
    Pco(PieceLocation),
}

/// 開幕に何を目指すか(2026-11-05・ユーザー。全自動の設定で選ぶ)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OpeningMode {
    /// これまでの既定: TD系テンプレか素の状態を半々で選ぶ
    #[default]
    TemplateOrPlain,
    /// TDテンプレ・開幕TSD・種3C4W から均等に選ぶ
    Random,
    /// TDテンプレ・開幕TSD から均等に選ぶ(種3C4W を除く。2026-11-07・ユーザー)
    RandomNoC4w,
    /// TDテンプレ(組めるものが無ければ素の状態)
    Td,
    /// 開幕TSD
    Tsd,
    /// 種3C4W(序盤の作戦を使わず、最初から中あけの評価で種3を作る)
    Seed3,
    /// 開幕パフェ(見込みが低ければ TDテンプレか開幕TSD。2026-11-09)
    Pco,
}

impl OpeningMode {
    /// bot の `--opening` の値から。知らない値は None。
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "default" => Some(Self::TemplateOrPlain),
            "random" => Some(Self::Random),
            "no_c4w" => Some(Self::RandomNoC4w),
            "td" => Some(Self::Td),
            "tsd" => Some(Self::Tsd),
            "seed3" => Some(Self::Seed3),
            "pco" => Some(Self::Pco),
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
    /// 開幕パフェ。選んでから置いたミノの数と、次に置く位置
    Pco(u32, PieceLocation),
    /// テンプレでパフェまで撃った後、最初に撃つまで火力の種(TSD・テトリス)を作る(開幕TSD と同じ評価)。置いたミノの数
    Seed(u32),
}

/// 開幕パフェを始める見込みの下限。
/// 【2026-11-09】0・0.15・0.3・0.5 で比べて差は誤差の範囲(0 と 0.3 は no_c4w に 58.9%・58.7%、C4W に 36.5%・36.8%)
const PCO_START: f32 = 0.3;
/// 開幕パフェの1手目に考える時間と、2手目から(ミリ秒)。
/// 【2026-11-09】200局で 1手目400・以降60 は 57.5%、1手目60・以降30 も 57.5% で、時間を延ばしても成功率は変わらない
/// (見えていないミノの来方を試す数は足りている)。止まらないように短くした
const PCO_FIRST_BUDGET_MS: u64 = 60;
const PCO_BUDGET_MS: u64 = 30;

/// 開幕パフェでソフトドロップ1回(空中で止めない)の置き方も使うか(2026-11-09)。
/// パフェは増えるが勝率は上がらなかったので(pco.rs の冒頭)、標準では使わない。環境変数 PCO_SOFT があれば使う(比べる時用)
fn pco_soft() -> bool {
    static SOFT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SOFT.get_or_init(|| std::env::var("PCO_SOFT").is_ok())
}

/// 開幕パフェの次の位置(見込みが無ければ None)。
fn pco_next(root: &Root, placed: u32, budget_ms: u64, rules: &Rules) -> Option<(PieceLocation, f32)> {
    let board = root.state.board;
    let soft = pco_soft();
    let rows = crate::pco::rows_left(&board, placed, soft)?;
    if root.incoming.iter().any(|g| g.lines > 0) {
        return None;
    }
    crate::pco::best_action(
        &board,
        rows,
        root.hold,
        &root.queue,
        &root.bag,
        std::time::Duration::from_millis(budget_ms),
        soft.then(|| rules.spawn()),
    )
    .map(|(a, p)| (a.loc, p))
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
    /// 直前に見た NEXT の数(開幕パフェの間、新しいミノが見えたら置く位置を考え直す)
    last_queue_len: usize,
    rng: u64,
    mode: OpeningMode,
    /// テンプレか開幕パフェでパフェまで撃った後、その局の終わりまで「C4W で対抗」を使わない(2026-11-09)
    no_rival: bool,
    /// パフェまで撃った後の扱い(`Weights::after_pc_seed`・`after_pc_no_rival`。`with_after_pc` で決める)
    after_pc_seed: bool,
    after_pc_no_rival: bool,
    /// テンプレの途中で攻撃された(予告が届いたか、せり上がりで図をずらした)。新しい作戦を選ぶと戻す
    attacked: bool,
    /// 攻撃された後は C4W の形も作り、TSD まで撃ったらテンプレを終える(`Weights::td_attacked_well`)
    attacked_well: bool,
    /// テンプレで何回消すまでに攻撃されたら速攻とみなすか(`Weights::td_attacked_clears`)
    attacked_clears: u32,
}

impl OpeningTracker {
    pub fn new(seed: u64) -> Self {
        Self::with_mode(seed, OpeningMode::default())
    }

    pub fn with_mode(seed: u64, mode: OpeningMode) -> Self {
        OpeningTracker {
            state: State::Done,
            last_board: None,
            last_queue_len: 0,
            rng: seed | 1,
            mode,
            no_rival: false,
            after_pc_seed: false,
            after_pc_no_rival: false,
            attacked: false,
            attacked_well: false,
            attacked_clears: 1,
        }
    }

    /// 【2026-11-10・ユーザー「相手が開幕TSDなどの速攻ではTST→TSDは打ててもパフェができないので、TSDまで打った後は
    /// C4Wに移行する。TSTやTSDを打ってからC4WにするのではなくTミノを待ちながらC4Wを組んでいく」】
    /// テンプレの途中で攻撃されたら、テンプレの手を続けながら中あけ(C4W)の評価も使い、TSD まで撃ったらテンプレを終える。
    /// `clears`: テンプレで何回消すまでに攻撃されたら速攻とみなすか。対戦シミュレーター(1000局)で 0回(撃つ前だけ)・1回・2回は
    /// 開幕TSD に 33.7%・35.5%・35.8%、TDテンプレ(run16)に 58.8%(引き分けが多い)・58.9%・58.4%(切り替えない run16 は 32.7%)。
    pub fn with_attacked_well(mut self, on: bool, clears: u32) -> Self {
        self.attacked_well = on;
        self.attacked_clears = clears;
        self
    }

    /// テンプレの途中で攻撃された局は、その局の終わりまで作戦層を「C4W で対抗」にする(相手が C4W でなくても。2026-11-10)
    pub fn force_rival(&self) -> bool {
        self.attacked_well && self.attacked
    }

    /// 序盤の作戦の間でも中あけの評価を使うか(`Root::opening_well` に入れる)。
    pub fn well_now(&self) -> bool {
        self.attacked_well && self.attacked && matches!(self.state, State::Template(_))
    }

    /// パフェまで撃った後の扱い(2026-11-09・ユーザー「TDパフェを取った後にC4Wに向かっているのがよくない。
    /// 火力の種を早々に作ることの方が大事」)。`seed`: 次のテンプレが組めなければ、最初に撃つまで TSD を狙う。
    /// `no_rival`: その局の終わりまで「C4W で対抗」を使わない。
    pub fn with_after_pc(mut self, seed: bool, no_rival: bool) -> Self {
        self.after_pc_seed = seed;
        self.after_pc_no_rival = no_rival;
        self
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
        // こちらがまだ撃つ前(TST の前)に届いた攻撃だけを速攻とみなす(TD どうしなら相手の TST はこちらの TST と同じ頃に届く)
        if matches!(&self.state, State::Template(t) if t.clears <= self.attacked_clears) && root.incoming.iter().any(|g| g.lines > 0) {
            self.attacked = true;
        }
        let changed = self.last_board.is_some_and(|b| b != board);
        let fewer_cells = self.last_board.is_some_and(|b| count(&b) > count(&board));
        // この手で消した行数(ミノ1つ = 4マス置いて、10マスずつ消える)。割り切れなければ 0 にせず、図から外れたことにする
        let cleared_lines = self.last_board.map_or(0, |b| {
            let gone = (count(&b) + 4).saturating_sub(count(&board));
            if fewer_cells && gone % 10 == 0 { gone / 10 } else if fewer_cells { u32::MAX } else { 0 }
        });
        if board.is_empty() && !root.queue.is_empty() {
            let fresh = match &self.state {
                State::Done => true,
                State::Plain(n) | State::Tsd(n) | State::Pco(n, _) | State::Seed(n) => *n > 0,
                State::Template(_) => changed,
            };
            if fresh {
                // テンプレか開幕パフェでパフェまで撃った(その途中で盤面が空になった)
                let from_pc = (matches!(self.state, State::Template(_)) && changed) || matches!(self.state, State::Pco(n, _) if n > 0);
                self.attacked = false;
                self.choose(root, rules);
                let (seed, normal) = (self.after_pc_seed, self.after_pc_no_rival);
                // 次のテンプレが組めなければ、C4W に向かわず火力の種を作る
                if from_pc && seed && matches!(self.state, State::Plain(_)) {
                    self.state = State::Seed(0);
                }
                // パフェの後は、その局の終わりまで「C4W で対抗」を使わない(新しい局の始めで戻す)
                if from_pc && normal {
                    self.no_rival = true;
                } else if !from_pc {
                    self.no_rival = false;
                }
            }
        } else if changed {
            self.state = match std::mem::replace(&mut self.state, State::Done) {
                State::Plain(n) if n + 1 < PLAIN_PIECES => State::Plain(n + 1),
                State::Tsd(n) if !fewer_cells && n + 1 < PLAIN_PIECES => State::Tsd(n + 1),
                State::Seed(n) if !fewer_cells && n + 1 < PLAIN_PIECES => State::Seed(n + 1),
                State::Template(t) if cleared_lines != u32::MAX => match t.follow(&board, cleared_lines) {
                    Some(next) => {
                        if next.rise > 0 && t.clears <= self.attacked_clears {
                            self.attacked = true;
                        }
                        // 攻撃された後は TSD まで(2回消すまで)でテンプレを終え、C4W に移る(パフェはせり上がりで組めない)
                        if self.attacked_well && self.attacked && next.clears >= 2 { State::Done } else { State::Template(next.narrowed(&board)) }
                    }
                    None => State::Done,
                },
                State::Pco(n, _) => match pco_next(root, n + 1, PCO_BUDGET_MS, rules) {
                    Some((loc, _)) => State::Pco(n + 1, loc),
                    None => State::Done,
                },
                _ => State::Done,
            };
        } else if let State::Pco(n, _) = self.state {
            // 盤面はそのままで新しいミノが見えた(TBP の new_piece)。見えるミノが増えたので考え直す
            if root.queue.len() != self.last_queue_len {
                self.state = pco_next(root, n, PCO_BUDGET_MS, rules).map_or(State::Done, |(loc, _)| State::Pco(n, loc));
            }
        }
        self.last_queue_len = root.queue.len();
        self.last_board = Some(board);
        self.opening()
    }

    fn choose(&mut self, root: &Root, rules: &Rules) {
        if self.mode == OpeningMode::Pco {
            if let Some((loc, p)) = pco_next(root, 0, PCO_FIRST_BUDGET_MS, rules) {
                if p >= PCO_START {
                    self.state = State::Pco(0, loc);
                    return;
                }
            }
        }
        let mode = match self.mode {
            OpeningMode::Pco => [OpeningMode::Td, OpeningMode::Tsd][(self.next_random() % 2) as usize],
            OpeningMode::Random => [OpeningMode::Td, OpeningMode::Tsd, OpeningMode::Seed3][(self.next_random() % 3) as usize],
            OpeningMode::RandomNoC4w => [OpeningMode::Td, OpeningMode::Tsd][(self.next_random() % 2) as usize],
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
        // 【2026-11-07・ユーザー】はちみつ砲 → 迷走砲(Iミノが左側)の順に、1巡目をソフトドロップなしで組めれば狙う
        // 【2026-11-11】パフェまで届く割合の順に、くろみつ砲 → はちみつ砲 → 迷走砲
        // 【2026-11-11・ユーザー】袋の残りも足して、1巡目の7個すべてをソフトドロップなしで置けるものだけ
        let hard = |t: &&Arc<Template>| t.buildable_hard_drop_known(&root.queue, root.hold, &root.bag, rules);
        if let Some(t) = priority_templates().iter().find(hard) {
            self.state = State::Template(t.clone());
            return;
        }
        // どれも組めなければ、ほかのTDテンプレを `OTHER_ORDER` の順に、1巡目をソフトドロップなしで組めれば狙う。どれも組めなければ
        // テンプレを狙わない(素の状態)
        // 【2026-11-11・ユーザー「TDテンプレの中で1巡目にソフトドロップが不要になるものを選ぶ」】ほかのテンプレも同じ条件
        // 【2026-11-11・ユーザー「パフェができる確率が高いものを用いたい」】以前は、ほかのテンプレか素の状態を均等に選んでいて、
        // 組めるテンプレがあっても素の状態を選ぶことがあった(対戦シミュレーターで3%の局)
        if let Some(t) = OTHER_ORDER.iter().filter_map(|name| td_templates().iter().find(|t| t.name == *name)).find(hard) {
            self.state = State::Template(t.clone());
        }
    }

    /// 今の作戦。
    pub fn opening(&self) -> Opening {
        match &self.state {
            State::Done => Opening::None,
            State::Plain(_) => Opening::Plain,
            State::Template(t) => Opening::Template(t.clone()),
            State::Tsd(_) => Opening::Tsd,
            State::Pco(_, loc) => Opening::Pco(*loc),
            State::Seed(_) => Opening::Tsd,
        }
    }

    /// 「C4W で対抗」を使わないか(テンプレでパフェまで撃った後。2026-11-09)
    pub fn suppress_rival(&self) -> bool {
        self.no_rival
    }

    /// 相手の C4W が `rows` 行以上育っていたら序盤を切り上げる(`rows` が 0 なら何もしない)。切り上げたら true。
    /// 序盤の間は中あけの評価を使わないので、相手が先に育てた C4W を追いかけられない(2026-11-07)。
    /// 【2026-11-08・ユーザー】テンプレは TST→TSD まで打ち切るので、テンプレの間は切り上げない。
    /// 【2026-11-09】シミュレーターの結果(TDテンプレは開幕 C4W に 14%、切り上げると 33%)でテンプレの間も切り上げたが、
    /// 実機4局すべてで TST の直後に C4W へ移り、TSD・パフェを組まずに相手の先打ちに負けた(ユーザー「TDを組んだ後に
    /// 追いかけてC4Wでは相手の先打ちに対抗できておらず負けてしまう」)。元に戻した。シミュレーターは TST→TSD→パフェまで
    /// 届いても C4W に 20% しか勝てず、実機とユーザーの知識(潰せる)に合わないので、規則のずれを疑う(DESIGN.md 8.12)。
    /// 開幕パフェの間も、組み終わるか見込みが無くなるまで続ける。
    pub fn pivot_if_opponent_c4w(&mut self, opponent: &Board, rows: u32) -> bool {
        if rows == 0
            || matches!(self.state, State::Done | State::Template(_) | State::Pco(..) | State::Seed(_))
            || crate::shape::ren_rows(opponent) < rows
        {
            return false;
        }
        self.state = State::Done;
        true
    }

    /// 選んだテンプレの名前(素の状態・序盤でなければ None)。記録用。
    pub fn template_name(&self) -> Option<&str> {
        match &self.state {
            State::Template(t) => Some(&t.name),
            _ => None,
        }
    }
}

/// 並べ替えをすべて。
fn permutations(items: &[Piece]) -> Vec<Vec<Piece>> {
    if items.len() <= 1 {
        return vec![items.to_vec()];
    }
    let mut out = Vec::new();
    for i in 0..items.len() {
        let mut rest = items.to_vec();
        let first = rest.remove(i);
        for mut p in permutations(&rest) {
            p.insert(0, first);
            out.push(p);
        }
    }
    out
}

/// `items` から `k` 個を選ぶ組み合わせ(昇順のまま)。
fn combinations(items: &[u32], k: usize) -> Vec<Vec<u32>> {
    if k == 0 {
        return vec![Vec::new()];
    }
    let mut out = Vec::new();
    for i in 0..items.len() {
        for mut rest in combinations(&items[i + 1..], k - 1) {
            rest.insert(0, items[i]);
            out.push(rest);
        }
    }
    out
}

fn count(board: &Board) -> u32 {
    board.cols.iter().map(|c| c.count_ones()).sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 優先テンプレを名前で
    fn tpl(name: &str) -> &'static Arc<Template> {
        priority_templates().iter().find(|t| t.name == name).expect("優先テンプレに無い")
    }

    #[test]
    fn priority_templates_in_order() {
        let names: Vec<&str> = priority_templates().iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["くろみつ砲", "自動探索TD-A", "はちみつ砲", "迷走砲"]);
        // 迷走砲の1巡目の図は Iミノが左端のものだけ
        let meisou = &tpl("迷走砲");
        let first: Vec<&Form> = meisou.forms.iter().filter(|f| f.is_first()).collect();
        assert!(!first.is_empty());
        for form in first {
            let (_, cells) = form.items.iter().find(|(p, _)| *p == Piece::I).unwrap();
            assert!(cells.iter().all(|&(x, _)| x == 0), "{cells:?}");
        }
    }

    #[test]
    fn 撃つtは印が無くても最後に置く() {
        // 【2026-11-08・実機 debug_log_20261108_202040 の1P 局3】はちみつ砲の2巡目の図には「最後に置く」印が無いものがあり、
        // TST の溝へ T を先に入れる手(1列消し)を図の手として受け付けていた
        let honey = &tpl("はちみつ砲");
        // 実機で崩した図(1巡目の図0の後の2巡目)と同じ並び: TST の行に Z などのマスがある
        let second: Vec<&Form> = honey.forms.iter().filter(|f| f.section == "理想形 > 2巡目" && !f.last.is_empty()).collect();
        assert_eq!(second.len(), 12, "理想形の2巡目で T が1つの図はすべて");
        for form in &second {
            let t = form.items.iter().position(|(p, _)| *p == Piece::T).unwrap();
            assert_eq!(form.last, vec![t], "{}", form.section);
            // 既に置いてあるブロックだけの盤面では、T を溝へ入れる手を受け付けない(ほかのミノを先に置く)
            let mut board = Board::new();
            board.cols = form.existing;
            let (_, cells) = form.items[t];
            let loc = (0..4)
                .flat_map(|r| (-2..12).flat_map(move |x| (-2..24).map(move |y| (r, x, y))))
                .map(|(r, x, y)| {
                    use tetrisai_core::Rotation::*;
                    PieceLocation { piece: Piece::T, rotation: [North, East, South, West][r], x, y }
                })
                .find(|l| l.sorted_cells() == cells)
                .unwrap();
            assert!(form.items.len() == 1 || !form.accepts(&board, &loc), "{}", form.section);
        }
        // 1巡目の T(土台)は印を付けない
        assert!(honey.forms.iter().filter(|f| f.is_first()).all(|f| f.last.is_empty()));
    }

    #[test]
    fn priority_template_is_chosen_in_order_when_buildable_by_hard_drop() {
        use Piece::*;
        let rules = Rules::ppt2();
        // 7種の並びのいくつかで、1巡目をソフトドロップなしで組める優先テンプレのうち先のものを選ぶ
        for order in [[I, O, Z, T, L, S, J], [S, T, I, Z, L, O, J], [J, T, I, S, O, Z, L], [L, Z, S, J, I, O, T]] {
            let expected = priority_templates()
                .iter()
                .find(|t| t.buildable_hard_drop_known(&order[..6], None, &order[6..], &rules))
                .map(|t| t.name.as_str());
            let root = Root { queue: order[..6].to_vec(), bag: order[6..].to_vec(), ..Root::default() };
            let mut tracker = OpeningTracker::with_mode(1, OpeningMode::Td);
            tracker.choose(&root, &rules);
            if expected.is_some() {
                assert_eq!(tracker.template_name(), expected, "{order:?}");
            }
        }
        // S T I Z L O J ではくろみつ砲を組めない(I を HOLD に残したまま最後に入れられない。実機 debug_log_20261111_210217 の9局目)
        let kuro = &tpl("くろみつ砲");
        assert_eq!(kuro.name, "くろみつ砲");
        assert!(!kuro.buildable_hard_drop_known(&[S, T, I, Z, L, O], None, &[J], &rules));
    }

    #[test]
    fn 先のミノまで図どおりに置き続けられる割合() {
        use Piece::*;
        let rules = Rules::ppt2();
        let honey = &tpl("はちみつ砲");
        // はちみつ砲を組める並び: 先のミノを足しても置き続けられる見込みがある
        let ok = honey.continue_rate(&Board::default(), &[I, O, Z, T, L, S, J], None, &[], &rules, 8, false, false);
        assert!(ok > 0.0, "{ok}");
        // 同じ盤面・同じミノなら同じ値(並べ方は盤面から決める)
        assert_eq!(ok, honey.continue_rate(&Board::default(), &[I, O, Z, T, L, S, J], None, &[], &rules, 8, false, false));
        // O を続けて置く図は無い
        let none = honey.continue_rate(&Board::default(), &[O, O, O, O, O], None, &[], &rules, 8, false, false);
        assert_eq!(none, 0.0);
    }

    #[test]
    fn tstの後はtsdを飛ばすパフェを組めたとみなさない() {
        use Piece::*;
        let rules = Rules::ppt2();
        // はちみつ砲で TST を撃った後の盤面(対戦シミュレーターの種 2)。図には、TSD の溝を T 以外で埋めてパフェにする図もある
        let board = Board::from_rows(&["......####", "..##..####", "####...###", "#####.####"]);
        let t = tpl("はちみつ砲").follow(&board, 3).expect("TST の後も図に従う");
        // I O S J Z L T の順では、T を最後まで使わずにパフェにする道筋しか無い(T は TSD の溝に入らない)
        let queue = [I, O, S, J, Z, L, T];
        assert!(walk(&t, board, &queue, None, &rules, false, false, &mut WalkMemo::default()));
        assert!(!walk(&t, board, &queue, None, &rules, false, true, &mut WalkMemo::default()), "TSD を撃つ前のパフェは数えない");
    }

    #[test]
    fn no_c4w_mode_never_chooses_seed3() {
        // 種3C4W を選ぶと State::Done になる。開幕C4Wを除く設定では必ず TD 系か開幕TSD か素の状態になる
        assert_eq!(OpeningMode::parse("no_c4w"), Some(OpeningMode::RandomNoC4w));
        let rules = Rules::ppt2();
        let queue = [Piece::I, Piece::O, Piece::T, Piece::S, Piece::Z, Piece::J];
        let root = Root { hold: None, queue: queue.to_vec(), ..Root::default() };
        for seed in 1..200 {
            let mut tracker = OpeningTracker::with_mode(seed, OpeningMode::RandomNoC4w);
            tracker.choose(&root, &rules);
            assert_ne!(tracker.state, State::Done, "種 {seed}");
        }
    }

    #[test]
    fn ほかのtd系テンプレ4種を読める() {
        let names: Vec<&str> = td_templates().iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["ホットケーキ積み", "PC-Spin", "皐月積み", "ベーカリーTD"]);
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
                Opening::Pco(_) => panic!("既定では開幕パフェを選ばない"),
            }
        }
        // 組めるテンプレがあるミノ順なので、おおよそ半々
        assert!((70..=130).contains(&template), "テンプレ {template} / 素 {plain}");
    }

    #[test]
    fn 開幕パフェを選んで置き続ける() {
        let rules = Rules::ppt2();
        // よく知られた PCO が組める1巡目
        let mut root = Root {
            queue: vec![Piece::I, Piece::L, Piece::O, Piece::J, Piece::S, Piece::Z],
            bag: vec![Piece::T],
            ..Default::default()
        };
        let mut tracker = OpeningTracker::with_mode(1, OpeningMode::Pco);
        let Opening::Pco(loc) = tracker.observe(&root, &rules) else { panic!("開幕パフェを選ばない") };
        // 1手置くと、次の位置を考え直す(開幕パフェのまま)
        let mv = crate::pco::to_move(&root.state.board, &crate::pco::Action { loc }, &rules).expect("入る手がある");
        root.play(&mv.placement, &rules);
        root.push_piece(Piece::T);
        assert!(matches!(tracker.observe(&root, &rules), Opening::Pco(_)));
        // おじゃまが来たら開幕パフェをやめる
        let mut hit = root.clone();
        hit.state.board.add_garbage(2, 0);
        assert_eq!(tracker.observe(&hit, &rules), Opening::None);
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

    /// 図の一番下にあるミノを図の位置に置いた盤面と、そのテンプレ(はちみつ砲)。
    fn honey_with_one_piece() -> (Arc<Template>, Board) {
        let honey = tpl("はちみつ砲").clone();
        let first = honey.forms.iter().find(|f| f.is_first()).unwrap();
        let (_, cells) = *first.items.iter().find(|(_, c)| c.iter().any(|&(_, y)| y == 0)).unwrap();
        let mut board = Board::default();
        for (x, y) in cells {
            board.cols[x as usize] |= 1 << y;
        }
        (honey, board)
    }

    #[test]
    fn せり上がってもテンプレを続ける() {
        // 【2026-11-08・ユーザー「相手の攻撃が早いと TST をあきらめる」】下におじゃまが2行増えても、図を2段ずらして続ける
        let (honey, board) = honey_with_one_piece();
        assert!(honey.consistent(&board));
        let mut risen = Board::default();
        for x in 0..WIDTH {
            risen.cols[x] = board.cols[x] << 2 | if x == 4 { 0 } else { 0b11 };
        }
        assert!(!honey.consistent(&risen), "ずらさないと図と合わない");
        let followed = honey.follow(&risen, 0).expect("せり上がりで図をずらして続ける");
        assert_eq!(followed.rise, 2);
        assert!(followed.consistent(&risen));
        // おじゃまの形(穴1つ)でない行が下に増えたのは、図から外れた
        let mut broken = risen;
        broken.cols[4] |= 1;
        assert!(honey.follow(&broken, 0).is_none());
    }

    #[test]
    fn 消した回数の上限までは続ける() {
        // 3巡目の図の始まりの盤面(TST の後)
        let honey = tpl("はちみつ砲").clone();
        let form = honey.forms.iter().find(|f| f.section.contains("3巡目")).unwrap();
        let mut board = Board::default();
        for x in 0..WIDTH {
            board.cols[x] = form.existing[x];
        }
        let mut t = honey.clone();
        for n in 1..TEMPLATE_CLEARS {
            t = t.follow(&board, 1).expect("上限までは消した後も続ける");
            assert_eq!(t.clears, n);
        }
        assert!(t.follow(&board, 1).is_none(), "上限で終わる");
    }

    #[test]
    fn 袋の残りも足して1巡目をソフトドロップなしで組めるか見る() {
        // 2026-11-11: J T I S O Z の後に L が来ると、はちみつ砲の1巡目は L でソフトドロップが要る(見えている6個だけなら組める)
        use Piece::*;
        let rules = Rules::ppt2();
        let honey = &tpl("はちみつ砲");
        assert_eq!(honey.name, "はちみつ砲");
        let queue = [J, T, I, S, O, Z];
        assert!(!honey.buildable_hard_drop_known(&queue, None, &[L], &rules));
        // 7個目が L 以外(I O Z T L S J の並び)なら組める
        assert!(honey.buildable_hard_drop_known(&[I, O, Z, T, L, S], None, &[J], &rules));
        // 開幕で選ぶ時も、はちみつ砲にはしない
        let mut tracker = OpeningTracker::with_mode(1, OpeningMode::Td);
        let root = Root { queue: queue.to_vec(), bag: vec![L], ..Root::default() };
        tracker.observe(&root, &rules);
        assert_ne!(tracker.template_name(), Some("はちみつ砲"));
    }

    #[test]
    fn 行が消えた後に置くミノは消えてからつながる() {
        // 2026-11-10: 1行目が消えた後に置く I(縦)。消える前は 0行目と 2〜4行目に分かれて描かれている
        let gap_i = [(0i8, 0i8), (0, 2), (0, 3), (0, 4)];
        let form = Form {
            section: "例".to_string(),
            existing: bits((1..WIDTH as i8).map(|x| (x, 0)).chain((0..WIDTH as i8).map(|x| (x, 1)))),
            cells: bits(gap_i),
            items: vec![(Piece::I, gap_i)],
            last: Vec::new(),
            gaps: vec![1 << 1],
            floor: 0,
        };
        let board = Board::from_rows(&["##########", ".#########"]);
        assert!(form.consistent(&board));
        // 消える前は置かない
        let standing = PieceLocation::new(Piece::I, tetrisai_core::Rotation::East, 0, 2);
        assert!(!form.accepts(&board, &standing));
        // 1行目が消えた後は、0〜3行目の縦の I として置ける
        let after = form.collapsed(&[1]);
        assert_eq!(after.gaps, vec![0]);
        let board = Board::from_rows(&[".#########"]);
        let loc = after.items[0].1;
        assert_eq!(loc, [(0, 0), (0, 1), (0, 2), (0, 3)]);
        let i = PieceLocation::new(Piece::I, tetrisai_core::Rotation::East, 0, 2);
        assert_eq!(i.sorted_cells(), loc, "{i:?}");
        assert!(after.accepts(&board, &i));
    }

    #[test]
    fn テンプレの間は相手のc4wで切り上げない() {
        let rules = Rules::ppt2();
        let mut tracker = OpeningTracker::with_mode(1, OpeningMode::Td);
        let queue = vec![Piece::S, Piece::L, Piece::T, Piece::Z, Piece::O, Piece::I, Piece::J];
        let root = Root { queue, ..Root::default() };
        assert!(matches!(tracker.observe(&root, &rules), Opening::Template(_)));
        let well = Board::from_rows(&["###....###"; 10]);
        assert!(!tracker.pivot_if_opponent_c4w(&well, 2));
        assert!(matches!(tracker.opening(), Opening::Template(_)));
        // 素の状態なら切り上げる
        let mut plain = OpeningTracker::with_mode(1, OpeningMode::TemplateOrPlain);
        plain.state = State::Plain(0);
        assert!(plain.pivot_if_opponent_c4w(&well, 2));
    }

    #[test]
    fn tsdで消えた後もパフェまで図に従う() {
        // 【2026-11-08・ユーザー「TST→TSD→パフェで21段せり上げ」】はちみつ砲の3巡目(パフェ狙い)の図: T を入れて TSD で
        // 下の2行を消した後、消えた行を図から取り除いた位置に残りのミノを置くと、盤面が空になる(パフェ)
        let honey = tpl("はちみつ砲").clone();
        let form = honey
            .forms
            .iter()
            .find(|f| f.section.contains("3巡目") && f.items.iter().any(|i| i.0 == Piece::T) && f.last.is_empty())
            .unwrap()
            .clone();
        let mut board = Board::default();
        for x in 0..WIDTH {
            board.cols[x] = form.existing[x];
        }
        let t = Arc::new(Template { name: honey.name.clone(), forms: vec![form.clone()], clears: 1, rise: 0, pc_after_tst: false });
        let (_, tee) = *form.items.iter().find(|i| i.0 == Piece::T).unwrap();
        for (x, y) in tee {
            board.cols[x as usize] |= 1 << y;
        }
        let full = board.full_lines();
        assert_eq!(full.count_ones(), 2, "TSD");
        board.remove_lines(full);
        let after = t.follow(&board, 2).expect("TSD の後も図に従う");
        assert_eq!(after.clears, 2);
        let rest = after.forms.iter().find(|f| f.items.len() == form.items.len() - 1).expect("T を除いた残りの図");
        for &(_, cells) in &rest.items {
            for (x, y) in cells {
                assert_eq!(board.cols[x as usize] >> y & 1, 0, "置くマスが空いている");
                board.cols[x as usize] |= 1 << y;
            }
        }
        let full = board.full_lines();
        board.remove_lines(full);
        assert!(board.is_empty(), "パフェ: {board:?}");
    }
}
