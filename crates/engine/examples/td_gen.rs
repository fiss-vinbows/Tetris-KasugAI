//! TDテンプレの3巡目(TST の後)の図を自動で作る(2026-11-12、DESIGN.md 8.14 の段階1)。
//!
//! テンプレの図のうち、TST の後の盤面(既に置いてあるブロックが26マスの図)ごとに、3巡目の7種の並び5040通りすべてで
//! 「5段の中で TSD を撃ち、残り3段でパフェ」できる置き方を深さ優先で探す。見つかった置き方を2つの図にする:
//! TSD まで(既に置いてあるブロック = TST の後の盤面、最後に置く T 付き)と、パフェまで(既に置いてあるブロック = TSD の後の盤面)。
//! 図の中で行が消えるのは TSD の T と最後のミノだけにする(消える前の座標のまま描けるように)。
//!
//! 使い方: cargo run --release -p tetrisai-engine --example td_gen -- <テンプレ名> [出力 JSON]
//! 環境変数: GEN_SOFT(T 以外のソフトドロップの区間数の上限。既定 1)
//!
//! ほかのモード(環境変数で選ぶ。DESIGN.md 8.14):
//! - GEN_BAG2=1: 段階2。テンプレの1巡目の土台ごとに、2巡目の並び5040通りで届く TST の後の盤面とパフェの届きやすさを比べる
//! - GEN_BASE=1: 段階2b。1巡目の土台をすべて並べ、並び5040通りで組める割合を GEN_BASE_OUT に書く
//!   (GEN_BASE_EXCLUDE=<土台|土台…> を組める並びは数えない。GEN_BASE_ORDERS で並びを間引く)
//! - GEN_SLOT=<1巡目の割合の下限>: 土台ごとに TST の置き場所を挙げ、2巡目の並び GEN_SLOT_ORDERS 通りで届くかを GEN_SLOT_OUT に書く
//! - GEN_EVAL2=<上位の数>: (土台, 置き場所) を2巡目 GEN_EVAL_BAG2 通り・3巡目 GEN_EVAL_BAG3 通りで評価する
//! - GEN_UNION=<土台|土台…>: 土台どうしの1巡目の重なり
//! - GEN_MAKE=<名前>: GEN_MAKE_BASE・GEN_MAKE_SLOT からテンプレの図(1〜3巡目、左右反転を含む)を作って GEN_MAKE_OUT に書く
//!   (GEN_MAKE_BOARDS・GEN_MAKE_MIN_GAIN・GEN_MAKE_COVER で TST の後の盤面の選び方、GEN_MAKE_SKIP_TSD で TSD を飛ばすパフェの図も、
//!   GEN_MAKE_PC_AFTER_TST で TST→パフェ型(物差しをどのパフェでもにし、pc_after_tst を付ける)、GEN_MAKE_CACHE で2巡目の結果を使い回す)
//!
//! 自動探索TD-A(`data/td_generated.json`)の作り方:
//! GEN_MAKE_SKIP_TSD=1 GEN_MAKE=自動探索TD-A GEN_MAKE_COVER=30 GEN_MAKE_BOARDS=12 GEN_MAKE_MIN_GAIN=0.0005
//! GEN_MAKE_BASE=".##......#/###..#.###/###.######/####.#####" GEN_MAKE_SLOT="3,1;3,2;3,3;4,2" GEN_MAKE_OUT=… td_gen

use std::collections::{BTreeSet, HashSet};

use tetrisai_core::board::WIDTH;
use tetrisai_core::{generate, Board, Piece, PlayerState, Rules, Spin};
use tetrisai_engine::opening::{priority_templates, td_templates};

/// 1つの置き方(ミノとマス。マスは (x, y) の昇順)
type Item = (Piece, [(i8, i8); 4]);

/// 見つかった解の1区切り: 区切りの始めの盤面と、その間に置くミノ(行を消した手で区切る。消した手は最後)。
/// `tsd` はその区切りが T の TSD で終わるか(図で T を最後に置く印を付ける)
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
struct Segment {
    board: Vec<u64>,
    items: Vec<Item>,
    tsd: bool,
}

/// 盤面の空いたマスのうち `rows` 段の中のものが、つながった区画ごとに4の倍数か(パフェの前提)
fn parity_ok(b: &Board, rows: u32) -> bool {
    let mut seen = [0u64; WIDTH];
    for x0 in 0..WIDTH {
        for y0 in 0..rows {
            if b.cols[x0] >> y0 & 1 == 1 || seen[x0] >> y0 & 1 == 1 {
                continue;
            }
            let mut stack = vec![(x0, y0)];
            seen[x0] |= 1 << y0;
            let mut n = 0;
            while let Some((x, y)) = stack.pop() {
                n += 1;
                let mut push = |nx: usize, ny: u32, stack: &mut Vec<(usize, u32)>| {
                    if b.cols[nx] >> ny & 1 == 0 && seen[nx] >> ny & 1 == 0 {
                        seen[nx] |= 1 << ny;
                        stack.push((nx, ny));
                    }
                };
                if x > 0 {
                    push(x - 1, y, &mut stack);
                }
                if x + 1 < WIDTH {
                    push(x + 1, y, &mut stack);
                }
                if y > 0 {
                    push(x, y - 1, &mut stack);
                }
                if y + 1 < rows {
                    push(x, y + 1, &mut stack);
                }
            }
            if n % 4 != 0 {
                return false;
            }
        }
    }
    true
}

fn sorted_cells(loc: &tetrisai_core::PieceLocation) -> [(i8, i8); 4] {
    let mut c = loc.cells();
    c.sort();
    c
}

struct Gen {
    rules: Rules,
    soft_max: u8,
    /// 置き続けられないと分かった局面(盤面・HOLD・残りのミノ・段階)
    dead: HashSet<(Board, Option<Piece>, Vec<Piece>, bool)>,
}

/// TSD の前(`tsd_done` が false)は、行を消してよい(消すたびに区切る)が、T 以外で2行以上は消さない。T の TSD(2行)で `tsd_done`。
/// 消した行の分だけ、置ける段(`rows`。始めは5段)が減る。TSD の後にパフェで盤面が空になれば解。
/// 見つかれば `segs` に区切りの列が入る(最後の区切りはパフェで終わる)。
#[allow(clippy::too_many_arguments)]
fn dfs(
    g: &mut Gen,
    board: Board,
    queue: &[Piece],
    hold: Option<Piece>,
    rows: u32,
    tsd_done: bool,
    cur: &mut Vec<Item>,
    cur_board: Board,
    segs: &mut Vec<Segment>,
) -> bool {
    let Some((&current, rest)) = queue.split_first() else { return false };
    let key = (board, hold, queue.to_vec(), tsd_done);
    if g.dead.contains(&key) {
        return false;
    }
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
    for (piece, rest, hold) in options {
        for m in generate(&board, piece, g.rules.spawn()) {
            if piece != Piece::T && m.soft_drop_sections > g.soft_max {
                continue;
            }
            let loc = m.placement.location;
            if loc.cells().iter().any(|&(_, y)| y < 0 || y as u32 >= rows) {
                continue;
            }
            let mut s = PlayerState::new(board);
            let r = s.apply(&m.placement, &g.rules);
            let item = (piece, sorted_cells(&loc));
            let is_tsd = piece == Piece::T && r.spin == Spin::Full && r.lines == 2;
            if r.perfect_clear {
                if !tsd_done && !is_tsd {
                    continue;
                }
                cur.push(item);
                segs.push(Segment { board: cur_board.cols.to_vec(), items: cur.clone(), tsd: is_tsd });
                return true;
            }
            if r.lines > 0 {
                // TSD は1回だけ。TSD の前でも後でも、T 以外の消去は1行ずつ
                if (is_tsd && tsd_done) || (!is_tsd && r.lines > 1) || (r.spin != Spin::None && !is_tsd) {
                    continue;
                }
                let rows = rows - r.lines;
                if !parity_ok(&s.board, rows) {
                    continue;
                }
                cur.push(item);
                segs.push(Segment { board: cur_board.cols.to_vec(), items: std::mem::take(cur), tsd: is_tsd });
                if dfs(g, s.board, rest, hold, rows, tsd_done || is_tsd, cur, s.board, segs) {
                    return true;
                }
                let seg = segs.pop().unwrap();
                *cur = seg.items;
                cur.pop();
                continue;
            }
            if !parity_ok(&s.board, rows) {
                continue;
            }
            cur.push(item);
            if dfs(g, s.board, rest, hold, rows, tsd_done, cur, cur_board, segs) {
                return true;
            }
            cur.pop();
        }
    }
    g.dead.insert(key);
    false
}

/// 今の図(テンプレのデータ)だけで、TST の後の盤面から TSD→パフェまで届くか(ミノ順を全部知っている前提。td_pc_probe と同じ考え方)
fn data_dfs(
    t: &std::sync::Arc<tetrisai_engine::opening::Template>,
    board: Board,
    queue: &[Piece],
    hold: Option<Piece>,
    tsd_done: bool,
    rules: &Rules,
    dead: &mut HashSet<(Board, Option<Piece>, Vec<Piece>, bool, u32)>,
) -> bool {
    let Some((&current, rest)) = queue.split_first() else { return false };
    let key = (board, hold, queue.to_vec(), tsd_done, t.clears);
    if dead.contains(&key) {
        return false;
    }
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
    for (piece, rest, hold) in options {
        for m in generate(&board, piece, rules.spawn()) {
            let mut s = PlayerState::new(board);
            let r = s.apply(&m.placement, rules);
            let is_tsd = r.spin == Spin::Full && r.lines >= 2;
            let shot = r.perfect_clear || is_tsd;
            if !shot && !t.accepts(&board, &m.placement.location) {
                continue;
            }
            if r.perfect_clear {
                if tsd_done || is_tsd {
                    return true;
                }
                continue;
            }
            if let Some(nt) = t.follow(&s.board, r.lines) {
                if data_dfs(&nt, s.board, rest, hold, tsd_done || is_tsd, rules, dead) {
                    return true;
                }
            }
        }
    }
    dead.insert(key);
    false
}

/// TST の後の盤面 `board` から、3巡目の7種の並び5040通りそれぞれで TSD+パフェの解を探す(16 並列)。
fn solve_pc(board: &Board, orders: &[[Piece; 7]], rules: &Rules, soft_max: u8) -> Vec<Option<Vec<Segment>>> {
    solve_pc_with(board, orders, rules, soft_max, false)
}

/// `solve_pc` で、`skip_tsd` なら TSD を撃たずにパフェ(TST→パフェ)の解を探す
fn solve_pc_with(board: &Board, orders: &[[Piece; 7]], rules: &Rules, soft_max: u8, skip_tsd: bool) -> Vec<Option<Vec<Segment>>> {
    std::thread::scope(|s| {
        let chunks: Vec<&[[Piece; 7]]> = orders.chunks(orders.len().div_ceil(16)).collect();
        let handles: Vec<_> = chunks
            .into_iter()
            .map(|chunk| {
                let rules = rules.clone();
                s.spawn(move || {
                    let mut g = Gen { rules, soft_max, dead: HashSet::new() };
                    chunk
                        .iter()
                        .map(|order| {
                            let (mut cur, mut segs) = (Vec::new(), Vec::new());
                            dfs(&mut g, *board, order, None, 5, skip_tsd, &mut cur, *board, &mut segs).then_some(segs)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    })
}

/// 2巡目の探索の作業用
struct Gen2 {
    rules: Rules,
    soft_max: u8,
    /// TST の溝のマス(T が入る所。ここだけは上がふさがっていてよい)
    slot: [u64; WIDTH],
    /// TST の T が入るマス(ほかのミノで埋めない)
    tst: [u64; WIDTH],
    /// 置ける段の上限(TST の後に5段に収まるように)
    rows: u32,
    /// TST で消える3段(0 なら使わない)。この3段の T 以外の空きマスは、残りのミノで上から埋められないといけない
    tst_rows: u64,
    /// 調べる局面の数の上限
    budget: usize,
    dead: HashSet<(Board, Option<Option<Piece>>, Vec<Piece>)>,
    /// 届いた TST の後の盤面と、そこまでの置き方(T 以外の6個と T。最初に見つけたもの)と、そのうちソフトドロップの手の数
    found: Vec<(Board, Vec<Item>, u32)>,
}

/// 盤面の、TST の溝(と土台に元からある穴)以外で上がふさがった空きマスが、`HOLE_MAX` より多いか。
/// 2巡目の積み方で3巡目の TSD の溝(屋根の下の空きマス)を作るので、少しは許す(くろみつ砲の理想形は1マス)
fn covered_hole(b: &Board, slot: &[u64; WIDTH], rows: u32) -> bool {
    let n: u32 = (0..WIDTH)
        .map(|x| {
            let col = b.cols[x];
            let top = 64 - col.leading_zeros();
            let below = if top >= 64 { u64::MAX } else { (1u64 << top) - 1 };
            (!col & below & ((1u64 << rows) - 1) & !slot[x]).count_ones()
        })
        .sum();
    n > HOLE_MAX
}

/// 2巡目に作ってよい、上がふさがった空きマスの数
const HOLE_MAX: u32 = 2;

/// 2巡目の7個(`queue`)を置き、最後に T で TST を撃つ置き方をすべて探す。`hold` の `Some(None)` は HOLD に3巡目の(まだ見えない)
/// ミノがある状態。2巡目のミノはすべて置き切る(HOLD に2巡目のミノを残さない)。
fn dfs2(g: &mut Gen2, board: Board, queue: &[Piece], hold: Option<Option<Piece>>, items: &mut Vec<Item>, soft: u32) {
    if g.dead.len() >= g.budget {
        return;
    }
    if g.tst_rows != 0 {
        // 消える3段の、T 以外の空きマス: 上がふさがっていたら埋められない。数が残りのミノで埋められる分を超えても無理
        let mut empty = 0;
        for x in 0..WIDTH {
            let col = board.cols[x];
            let top = 64 - col.leading_zeros();
            let below = if top >= 64 { u64::MAX } else { (1u64 << top) - 1 };
            let e = !col & g.tst_rows & !g.tst[x];
            if e & below != 0 {
                return;
            }
            empty += e.count_ones();
        }
        let left = queue.iter().filter(|&&p| p != Piece::T).count() + matches!(hold, Some(Some(p)) if p != Piece::T) as usize;
        if empty as usize > 4 * left {
            return;
        }
    }
    let key = (board, hold, queue.to_vec());
    if !g.dead.insert(key) {
        return;
    }
    // (置くミノ, 残り, 置いた後の HOLD)。queue が空なら、次は3巡目の見えないミノ(置けないが HOLD とは入れ替えられる)
    let mut options: Vec<(Piece, &[Piece], Option<Option<Piece>>)> = Vec::new();
    match queue.split_first() {
        Some((&current, rest)) => {
            options.push((current, rest, hold));
            match hold {
                Some(Some(h)) if h != current => options.push((h, rest, Some(Some(current)))),
                None => {
                    if let Some((&next, rest2)) = rest.split_first() {
                        if next != current {
                            options.push((next, rest2, Some(Some(current))));
                        }
                    }
                }
                _ => {}
            }
        }
        None => {
            // 2巡目のミノが HOLD に残っていれば、3巡目の見えないミノと入れ替えて置く
            if let Some(Some(h)) = hold {
                options.push((h, queue, Some(None)));
            }
        }
    }
    for (piece, rest, hold) in options {
        let last_real = rest.is_empty() && !matches!(hold, Some(Some(_)));
        for m in generate(&board, piece, g.rules.spawn()) {
            let loc = m.placement.location;
            if loc.cells().iter().any(|&(_, y)| y < 0 || y as u32 >= g.rows) {
                continue;
            }
            if piece != Piece::T && m.soft_drop_sections > g.soft_max {
                continue;
            }
            let mut s = PlayerState::new(board);
            let r = s.apply(&m.placement, &g.rules);
            if piece == Piece::T {
                // T は最後に TST で撃つ
                if last_real && r.spin == Spin::Full && r.lines == 3 && !g.found.iter().any(|(b, ..)| *b == s.board) {
                    let mut all = items.clone();
                    all.push((piece, sorted_cells(&loc)));
                    g.found.push((s.board, all, soft));
                }
                continue;
            }
            if r.lines > 0 || loc.cells().iter().any(|&(x, y)| g.tst[x as usize] >> y & 1 == 1) {
                continue;
            }
            if covered_hole(&s.board, &g.slot, g.rows) {
                continue;
            }
            items.push((piece, sorted_cells(&loc)));
            dfs2(g, s.board, rest, hold, items, soft + (m.soft_drop_sections > 0) as u32);
            items.pop();
        }
    }
}

/// 7種の並びすべて(Heap のアルゴリズム)
fn all_orders() -> Vec<[Piece; 7]> {
    let mut order = Piece::ALL;
    let mut out = vec![order];
    let mut c = [0usize; 7];
    let mut i = 0;
    while i < 7 {
        if c[i] < i {
            if i % 2 == 0 {
                order.swap(0, i);
            } else {
                order.swap(c[i], i);
            }
            out.push(order);
            c[i] += 1;
            i = 0;
        } else {
            c[i] = 0;
            i += 1;
        }
    }
    out
}

fn board_rows(b: &Board, rows: u32) -> String {
    (0..rows).rev().map(|y| (0..WIDTH).map(|x| if b.cols[x] >> y & 1 == 1 { '#' } else { '.' }).collect::<String>()).collect::<Vec<_>>().join("/")
}

fn cells_json(cells: impl IntoIterator<Item = (i8, i8)>) -> String {
    cells.into_iter().map(|(x, y)| format!("[{x},{y}]")).collect::<Vec<_>>().join(",")
}

fn board_cells(b: &Board) -> Vec<(i8, i8)> {
    let mut v = Vec::new();
    for x in 0..WIDTH {
        for y in 0..20 {
            if b.cols[x] >> y & 1 == 1 {
                v.push((x as i8, y as i8));
            }
        }
    }
    v
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let name = args.get(1).cloned().unwrap_or_else(|| "くろみつ砲".to_string());
    let out_path = args.get(2).cloned();
    let soft_max: u8 = std::env::var("GEN_SOFT").ok().and_then(|s| s.parse().ok()).unwrap_or(1);
    let rules = Rules::ppt2();
    let template = priority_templates().iter().chain(td_templates()).find(|t| t.name == name).expect("テンプレが無い").clone();
    if let Ok(name) = std::env::var("GEN_MAKE") {
        make_template(&rules, &name);
        return;
    }
    if let Ok(list) = std::env::var("GEN_UNION") {
        // 土台どうしの1巡目の重なり(並び5040通り)。'|' 区切りで土台を並べる
        let targets: Vec<Board> = list.split('|').map(|r| { let b = parse_rows(r); let m = mirror(&b); if m.cols < b.cols { m } else { b } }).collect();
        let orders = all_orders();
        let masks: Vec<u32> = std::thread::scope(|s| {
            let chunks: Vec<&[[Piece; 7]]> = orders.chunks(orders.len().div_ceil(16)).collect();
            let hs: Vec<_> = chunks.into_iter().map(|c| { let (rules, targets) = (rules.clone(), &targets); s.spawn(move || c.iter().map(|o| { let r = reach_bases(o, &rules); targets.iter().enumerate().fold(0u32, |m, (i, t)| m | ((r.contains(t) as u32) << i)) }).collect::<Vec<_>>()) }).collect();
            hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
        });
        let mut hist = std::collections::BTreeMap::new();
        for m in &masks { *hist.entry(*m).or_insert(0) += 1; }
        for (m, n) in hist { println!("  組める土台 {m:0w$b}: {n} 並び ({:.1}%)", 100.0 * n as f64 / 5040.0, w = targets.len()); }
        return;
    }
    if let Ok(n) = std::env::var("GEN_SLOT") {
        slot_search(&rules, n.parse().unwrap_or(0.6));
        return;
    }
    if let Ok(n) = std::env::var("GEN_EVAL2") {
        eval_slots(&rules, n.parse().unwrap_or(100));
        return;
    }
    if std::env::var("GEN_BASE").is_ok() {
        base_search(&rules);
        return;
    }
    if std::env::var("GEN_BAG2").is_ok() {
        bag2(&template, &rules, soft_max);
        return;
    }
    // TST の後の盤面(既に置いてあるブロックが26マスの図)。重複を除く
    let mut boards: Vec<Board> = Vec::new();
    for f in &template.forms {
        let n: u32 = f.existing.iter().map(|c| c.count_ones()).sum();
        if n == 26 && !f.is_first() {
            let b = Board { cols: f.existing };
            if !boards.contains(&b) {
                boards.push(b);
            }
        }
    }
    println!("{name}: TST の後の盤面 {} 通り(ソフトドロップの区間 {soft_max} まで)", boards.len());
    let orders = all_orders();
    let mut forms_json: Vec<String> = Vec::new();
    for (bi, board) in boards.iter().enumerate() {
        let results = solve_pc(board, &orders, &rules, soft_max);
        let solved = results.iter().filter(|r| r.is_some()).count();
        // 今の図だけで届く並び(TST を撃った後のテンプレ: この盤面と合う図)
        let after_tst = std::sync::Arc::new(tetrisai_engine::opening::Template {
            name: template.name.clone(),
            forms: template.forms.iter().filter(|f| !f.is_first() && f.consistent(board)).cloned().collect(),
            clears: 1,
            rise: 0,
            pc_after_tst: false,
        });
        let data_ok: Vec<bool> = std::thread::scope(|s| {
            let chunks: Vec<&[[Piece; 7]]> = orders.chunks(orders.len().div_ceil(16)).collect();
            let handles: Vec<_> = chunks
                .into_iter()
                .map(|chunk| {
                    let (rules, t) = (rules.clone(), after_tst.clone());
                    s.spawn(move || {
                        let mut dead = HashSet::new();
                        chunk.iter().map(|o| data_dfs(&t, *board, &o[..], None, false, &rules, &mut dead)).collect::<Vec<_>>()
                    })
                })
                .collect();
            handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
        });
        let data_solved = data_ok.iter().filter(|&&b| b).count();
        let union = data_ok.iter().zip(&results).filter(|(d, r)| **d || r.is_some()).count();
        println!(
            "  盤面{} 今の図だけで届く並び {data_solved}/5040 ({:.1}%)・作った図と合わせると {union}/5040 ({:.1}%)",
            bi + 1,
            100.0 * data_solved as f64 / 5040.0,
            100.0 * union as f64 / 5040.0
        );
        let distinct: BTreeSet<&Vec<Segment>> = results.iter().flatten().collect();
        // 区切りごとの図(重複を除く。TSD で終わる区切りは T を最後に)
        let mut forms: BTreeSet<Segment> = BTreeSet::new();
        for segs in &distinct {
            for seg in segs.iter() {
                let mut items = seg.items.clone();
                let last = items.pop().unwrap();
                items.sort();
                items.push(last);
                forms.insert(Segment { board: seg.board.clone(), items, tsd: seg.tsd });
            }
        }
        println!(
            "  盤面{} {}: 解けた並び {solved}/5040 ({:.1}%)・違う解 {}・図 {}",
            bi + 1,
            board_rows(board, 5),
            100.0 * solved as f64 / 5040.0,
            distinct.len(),
            forms.len()
        );
        for f in &forms {
            let b = Board { cols: f.board.clone().try_into().unwrap() };
            let n = f.items.len();
            let items_json: Vec<String> = f
                .items
                .iter()
                .enumerate()
                .map(|(i, (p, c))| {
                    format!(r#"{{"piece":"{}","cells":[{}],"last":{}}}"#, p.to_char(), cells_json(c.iter().copied()), f.tsd && i + 1 == n)
                })
                .collect();
            let section = if f.tsd { "自動 > 3巡目 TSD" } else { "自動 > 3巡目" };
            forms_json.push(format!(r#"{{"section":"{section}","existing":[{}],"items":[{}]}}"#, cells_json(board_cells(&b)), items_json.join(",")));
        }
    }
    if let Some(path) = out_path {
        let json = format!("[{{\"name\":\"{name}\",\"forms\":[\n{}\n]}}]\n", forms_json.join(",\n"));
        std::fs::write(&path, json).expect("書き出せない");
        println!("図 {} 個を {path} に書き出した", forms_json.len());
    }
}

/// 段階2: 1巡目の土台ごとに、2巡目の7種の並び5040通りで届く TST の後の盤面を探し、パフェの届きやすさで比べる。
fn bag2(template: &std::sync::Arc<tetrisai_engine::opening::Template>, rules: &Rules, soft_max: u8) {
    use std::collections::HashMap;
    let orders = all_orders();
    // 1巡目の土台(1巡目の図の完成形)と、そこから始まる2巡目の図の撃つ T のマス(TST の溝)
    let mut bases: Vec<(Board, [u64; WIDTH], [u64; WIDTH])> = Vec::new();
    for f in template.forms.iter().filter(|f| f.is_first()) {
        let base = Board { cols: f.cells };
        let mut slot = [0u64; WIDTH];
        let mut tst = [0u64; WIDTH];
        for g in template.forms.iter().filter(|g| g.existing == base.cols) {
            for (p, c) in &g.items {
                if *p == Piece::T {
                    for &(x, y) in c {
                        slot[x as usize] |= 1 << y;
                        tst[x as usize] |= 1 << y;
                    }
                }
            }
        }
        // 土台に元からある穴(上がふさがった空きマス。3巡目の TSD の溝など)も、穴として数えない
        for x in 0..WIDTH {
            let col = base.cols[x];
            let top = 64 - col.leading_zeros();
            slot[x] |= !col & ((1u64 << top) - 1);
        }
        if !bases.iter().any(|(b, ..)| *b == base) && tst.iter().any(|&c| c != 0) {
            bases.push((base, slot, tst));
        }
    }
    println!("{}: 1巡目の土台 {} 通り(ソフトドロップの区間 {soft_max} まで)", template.name, bases.len());
    if std::env::var("GEN_TRY").is_ok() {
        // 図にある2巡目の置き方を1個ずつ置いて、どこで弾かれるかを見る(調べる用)
        let (base, slot, tst) = &bases[0];
        let f = template.forms.iter().find(|g| g.existing == base.cols).unwrap();
        println!("{} {:?}", f.section, f.items.iter().map(|(p, _)| *p).collect::<Vec<_>>());
        let mut b = *base;
        let mut items = f.items.clone();
        items.sort_by_key(|(p, c)| (*p == Piece::T, c.iter().map(|&(_, y)| y).min()));
        let order: Vec<Piece> = items.iter().map(|(p, _)| *p).collect();
        let mut g = Gen2 { rules: rules.clone(), soft_max, slot: *slot, tst: *tst, rows: 8, tst_rows: 0, budget: usize::MAX, dead: HashSet::new(), found: Vec::new() };
        dfs2(&mut g, *base, &order, None, &mut Vec::new(), 0);
        println!("並び {order:?}: 届いた盤面 {}・調べた局面 {}", g.found.len(), g.dead.len());
        for (p, cells) in &items {
            let ms = generate(&b, *p, rules.spawn());
            let m = ms.iter().find(|m| sorted_cells(&m.placement.location) == { let mut c = *cells; c.sort(); c });
            match m {
                None => println!("  {p:?} {cells:?}: 入る手が無い"),
                Some(m) => {
                    let mut st = PlayerState::new(b);
                    let r = st.apply(&m.placement, rules);
                    b = st.board;
                    println!("{}", board_rows(&b, 8).replace('/', "
"));
                    println!(
                        "  {p:?} SD{} 消した{} spin{:?} 穴{} TSTのマス{}",
                        m.soft_drop_sections,
                        r.lines,
                        r.spin,
                        covered_hole(&b, slot, 8),
                        cells.iter().any(|&(x, y)| tst[x as usize] >> y & 1 == 1)
                    );
                }
            }
        }
        return;
    }
    // TST の後の盤面ごとの、3巡目で TSD+パフェまで届く並びの割合(作った図で数える)
    let mut cover: HashMap<Board, f64> = HashMap::new();
    for (bi, (base, slot, tst)) in bases.iter().enumerate() {
        println!("== 土台{} {}", bi + 1, board_rows(base, 5));
        let per_order: Vec<Vec<(Board, Vec<Item>, u32)>> = std::thread::scope(|s| {
            let chunks: Vec<&[[Piece; 7]]> = orders.chunks(orders.len().div_ceil(16)).collect();
            let handles: Vec<_> = chunks
                .into_iter()
                .map(|chunk| {
                    let rules = rules.clone();
                    s.spawn(move || {
                        chunk
                            .iter()
                            .map(|order| {
                                let mut g = Gen2 { rules: rules.clone(), soft_max, slot: *slot, tst: *tst, rows: 8, tst_rows: 0, budget: usize::MAX, dead: HashSet::new(), found: Vec::new() };
                                dfs2(&mut g, *base, order, None, &mut Vec::new(), 0);
                                g.found
                            })
                            .collect::<Vec<_>>()
                    })
                })
                .collect();
            handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
        });
        let boards: BTreeSet<Vec<u64>> = per_order.iter().flatten().map(|(b, ..)| b.cols.to_vec()).collect();
        let to_board = |v: &Vec<u64>| Board { cols: v.clone().try_into().unwrap() };
        for b in &boards {
            let board = to_board(b);
            if !cover.contains_key(&board) {
                let n = solve_pc(&board, &orders, rules, soft_max).iter().filter(|r| r.is_some()).count();
                cover.insert(board, n as f64 / 5040.0);
            }
        }
        // 今の図(2巡目の図)で届く TST の後の盤面
        let data_after: Vec<Board> = template
            .forms
            .iter()
            .filter(|f| f.existing == base.cols)
            .map(|f| {
                let mut b = Board { cols: f.existing };
                for (_, c) in &f.items {
                    for &(x, y) in c {
                        b.cols[x as usize] |= 1 << y;
                    }
                }
                b.clear_full_lines();
                b
            })
            .collect();
        // 並びごとの一番よい盤面(パフェの届きやすさ。同じならソフトドロップの少ない方)と、今の図で届く盤面だけの中での一番よいもの
        let mut sum_best = 0.0;
        let mut sum_data = 0.0;
        let (mut reach, mut reach_data, mut hard) = (0, 0, 0);
        for found in &per_order {
            if let Some((c, soft)) = found.iter().map(|(b, _, soft)| (cover[b], *soft)).max_by(|a, b| a.0.partial_cmp(&b.0).unwrap().then(b.1.cmp(&a.1))) {
                sum_best += c;
                reach += 1;
                hard += (soft == 0) as u32;
            }
            if let Some(c) = found.iter().filter(|(b, ..)| data_after.contains(b)).map(|(b, ..)| cover[b]).max_by(|a, b| a.partial_cmp(b).unwrap()) {
                sum_data += c;
                reach_data += 1;
            }
        }
        println!(
            "  TST まで組める並び: 作った置き方 {reach}/5040・今の図の盤面だけ {reach_data}/5040。届く TST の後の盤面 {} 通り",
            boards.len()
        );
        println!(
            "  並びごとに一番よい盤面を選んだ時の TSD+パフェ(全部知っている前提の平均): 作った置き方 {:.1}%・今の図の盤面だけ {:.1}%。一番よい盤面に2巡目をハードドロップだけで届く並び {hard}",
            100.0 * sum_best / 5040.0,
            100.0 * sum_data / 5040.0
        );
        let mut ranked: Vec<(Board, f64, usize)> = boards
            .iter()
            .map(|b| {
                let board = to_board(b);
                let n = per_order.iter().filter(|f| f.iter().any(|(fb, ..)| *fb == board)).count();
                (board, cover[&board], n)
            })
            .collect();
        ranked.sort_by(|a, b| (b.1 * b.2 as f64).partial_cmp(&(a.1 * a.2 as f64)).unwrap());
        for (board, c, n) in ranked.iter().take(10) {
            println!("    {:5.1}% 届く並び {n:4} {}{}", 100.0 * c, board_rows(board, 5), if data_after.contains(board) { " (今の図にある)" } else { "" });
        }
    }
}

/// 左右反転した盤面
fn mirror(b: &Board) -> Board {
    let mut cols = b.cols;
    cols.reverse();
    Board { cols }
}

/// 盤面の上がふさがった空きマスの数(`rows` 段の中)
fn holes(b: &Board, rows: u32) -> u32 {
    (0..WIDTH)
        .map(|x| {
            let col = b.cols[x];
            let top = 64 - col.leading_zeros();
            let below = if top >= 64 { u64::MAX } else { (1u64 << top) - 1 };
            (!col & below & ((1u64 << rows) - 1)).count_ones()
        })
        .sum()
}

/// 段階2b の土台の条件
const BASE_ROWS: u32 = 4;
const BASE_HOLES: u32 = 1;

/// 1巡目の7種をハードドロップで置いた盤面(土台)をすべて並べる(4段まで・ラインを消さない・上がふさがったマスは1つまで)。
/// 左右反転は小さい方にそろえる。
fn enumerate_bases(rules: &Rules) -> Vec<Board> {
    fn go(b: Board, left: u8, rules: &Rules, seen: &mut HashSet<(Board, u8)>, out: &mut HashSet<Board>) {
        if !seen.insert((b, left)) {
            return;
        }
        if left == 0 {
            let m = mirror(&b);
            out.insert(if m.cols < b.cols { m } else { b });
            return;
        }
        for (i, &p) in Piece::ALL.iter().enumerate() {
            if left >> i & 1 == 0 {
                continue;
            }
            for m in generate(&b, p, rules.spawn()) {
                if m.soft_drop_sections > 0 {
                    continue;
                }
                let loc = m.placement.location;
                if loc.cells().iter().any(|&(_, y)| y as u32 >= BASE_ROWS) {
                    continue;
                }
                let mut nb = b;
                nb.place(&loc);
                if nb.full_lines() != 0 || holes(&nb, BASE_ROWS) > BASE_HOLES {
                    continue;
                }
                go(nb, left & !(1 << i), rules, seen, out);
            }
        }
    }
    let mut seen = HashSet::new();
    let mut out = HashSet::new();
    go(Board::default(), 0x7f, rules, &mut seen, &mut out);
    let mut v: Vec<Board> = out.into_iter().collect();
    v.sort_by_key(|b| b.cols);
    v
}

/// 決まった種で並びを間引く
fn sample_orders(orders: &[[Piece; 7]], n: usize, seed: u64) -> Vec<[Piece; 7]> {
    let mut rng = seed | 1;
    (0..n)
        .map(|_| {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            orders[(rng % orders.len() as u64) as usize]
        })
        .collect()
}

/// 段階2b: 土台から新しいテンプレを探す
fn base_search(rules: &Rules) {
    let started = std::time::Instant::now();
    let bases = enumerate_bases(rules);
    println!("土台 {} 通り({:.1}秒)", bases.len(), started.elapsed().as_secs_f64());
    // くろみつ砲・はちみつ砲の土台が入っているか
    for t in priority_templates() {
        for f in t.forms.iter().filter(|f| f.is_first()) {
            let b = Board { cols: f.cells };
            let m = mirror(&b);
            let c = if m.cols < b.cols { m } else { b };
            println!("  {} の土台 {}: {}", t.name, board_rows(&b, 4), if bases.contains(&c) { "入っている" } else { "入っていない" });
        }
    }
    if std::env::var("GEN_BASE_COUNT").is_ok() {
        return;
    }
    let orders = all_orders();
    // 1巡目を組める並びの割合: 並びごとに届く土台(左右反転はそろえる)をすべて集めて数える
    let n_orders: usize = std::env::var("GEN_BASE_ORDERS").ok().and_then(|s| s.parse().ok()).unwrap_or(5040);
    let sample: Vec<[Piece; 7]> = if n_orders >= orders.len() { orders.clone() } else { sample_orders(&orders, n_orders, 12345) };
    let index: std::collections::HashMap<Board, usize> = bases.iter().enumerate().map(|(i, b)| (*b, i)).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    // GEN_BASE_EXCLUDE(土台を '|' 区切り): この土台のどれかを組める並びは数えない(今のテンプレで足りる並びを除いて探す)
    let exclude: Vec<Board> = std::env::var("GEN_BASE_EXCLUDE")
        .map(|list| {
            list.split('|')
                .map(|r| {
                    let b = parse_rows(r);
                    let m = mirror(&b);
                    if m.cols < b.cols { m } else { b }
                })
                .collect()
        })
        .unwrap_or_default();
    let counted = std::sync::atomic::AtomicUsize::new(0);
    let counts: Vec<u32> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..16)
            .map(|_| {
                let (rules, sample, index, next, exclude, counted) = (rules.clone(), &sample, &index, &next, &exclude, &counted);
                s.spawn(move || {
                    let mut counts = vec![0u32; index.len()];
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(order) = sample.get(i) else { break };
                        let reach = reach_bases(order, &rules);
                        if exclude.iter().any(|e| reach.contains(e)) {
                            continue;
                        }
                        counted.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        for b in reach {
                            if let Some(&k) = index.get(&b) {
                                counts[k] += 1;
                            }
                        }
                        if i % 500 == 0 {
                            eprintln!("  並び {i} / {}({:.0}秒)", sample.len(), started.elapsed().as_secs_f64());
                        }
                    }
                    counts
                })
            })
            .collect();
        let mut total = vec![0u32; index.len()];
        for h in handles {
            for (t, c) in total.iter_mut().zip(h.join().unwrap()) {
                *t += c;
            }
        }
        total
    });
    let counted = counted.into_inner().max(1);
    println!("数えた並び {counted}/{}", sample.len());
    let rates: Vec<f64> = counts.iter().map(|&c| c as f64 / counted as f64).collect();
    let mut hist = [0usize; 11];
    for r in &rates {
        hist[(r * 10.0).floor().min(10.0) as usize] += 1;
    }
    println!("1巡目を組める並びの割合の分布: {hist:?}({:.1}秒)", started.elapsed().as_secs_f64());
    let mut ranked: Vec<(f64, &Board)> = rates.iter().copied().zip(&bases).collect();
    ranked.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    for (r, b) in ranked.iter().take(10) {
        println!("  {:5.1}% {}", 100.0 * r, board_rows(b, 4));
    }
    let path = std::env::var("GEN_BASE_OUT").unwrap_or_else(|_| "bases.txt".to_string());
    let lines: Vec<String> = ranked.iter().map(|(r, b)| format!("{r:.3} {}", board_rows(b, 4))).collect();
    std::fs::write(&path, lines.join("\n")).unwrap();
    println!("書き出した: {path}");
}

/// 並び `order`(1巡目の7個。HOLD あり)でハードドロップだけで届く土台(7個すべて置いた盤面。左右反転は小さい方にそろえる)
fn reach_bases(order: &[Piece; 7], rules: &Rules) -> HashSet<Board> {
    fn place(b: &Board, piece: Piece, rules: &Rules) -> Vec<Board> {
        generate(b, piece, rules.spawn())
            .iter()
            .filter(|m| m.soft_drop_sections == 0 && m.placement.location.cells().iter().all(|&(_, y)| (y as u32) < BASE_ROWS))
            .filter_map(|m| {
                let mut nb = *b;
                nb.place(&m.placement.location);
                (nb.full_lines() == 0 && holes(&nb, BASE_ROWS) <= BASE_HOLES).then_some(nb)
            })
            .collect()
    }
    fn go(b: Board, idx: usize, hold: Option<Piece>, order: &[Piece; 7], rules: &Rules, seen: &mut HashSet<(Board, usize, Option<Piece>)>, out: &mut HashSet<Board>) {
        if !seen.insert((b, idx, hold)) {
            return;
        }
        if idx == 7 {
            match hold {
                None => {
                    let m = mirror(&b);
                    out.insert(if m.cols < b.cols { m } else { b });
                }
                Some(h) => {
                    for nb in place(&b, h, rules) {
                        go(nb, 7, None, order, rules, seen, out);
                    }
                }
            }
            return;
        }
        let current = order[idx];
        for nb in place(&b, current, rules) {
            go(nb, idx + 1, hold, order, rules, seen, out);
        }
        match hold {
            Some(h) if h != current => {
                for nb in place(&b, h, rules) {
                    go(nb, idx + 1, Some(current), order, rules, seen, out);
                }
            }
            None if idx + 1 < 7 => {
                for nb in place(&b, order[idx + 1], rules) {
                    go(nb, idx + 2, Some(current), order, rules, seen, out);
                }
            }
            _ => {}
        }
    }
    let mut seen = HashSet::new();
    let mut out = HashSet::new();
    go(Board::default(), 0, None, order, rules, &mut seen, &mut out);
    out
}

/// 一覧の土台を読む("割合 図" の行)
fn read_bases(path: &str) -> Vec<(f64, Board)> {
    let text = std::fs::read_to_string(path).expect("土台の一覧が無い");
    text.lines()
        .filter_map(|l| {
            let (r, _) = l.split_once(' ')?;
            let (_, rows) = l.rsplit_once(' ')?;
            let lines: Vec<&str> = rows.split('/').collect();
            let mut b = Board::default();
            for (i, line) in lines.iter().enumerate() {
                let y = lines.len() - 1 - i;
                for (x, ch) in line.chars().enumerate() {
                    if ch == '#' {
                        b.cols[x] |= 1 << y;
                    }
                }
            }
            Some((r.parse().ok()?, b))
        })
        .collect()
}

/// 土台に元からある、上がふさがった空きマス
fn base_holes_of(base: &Board) -> [u64; WIDTH] {
    let mut h = [0u64; WIDTH];
    for x in 0..WIDTH {
        let col = base.cols[x];
        let top = 64 - col.leading_zeros();
        h[x] = !col & ((1u64 << top) - 1);
    }
    h
}

/// 土台の上で TST を撃てそうな T の置き場所(マス)をすべて挙げる。
/// T のマスを除いて消える3段を埋め、それより上は土台だけの盤面(理想形)で、T がその場所に TST で入り、消した後が5段に収まるもの。
fn tst_slots(base: &Board, rules: &Rules) -> Vec<[(i8, i8); 4]> {
    let holes = base_holes_of(base);
    let mut out = Vec::new();
    for r in 0..=3i8 {
        for x in 0..WIDTH as i8 {
            for dir in [-1i8, 1] {
                let sx = x + dir;
                if !(0..WIDTH as i8).contains(&sx) {
                    continue;
                }
                let mut cells = [(x, r), (x, r + 1), (x, r + 2), (sx, r + 1)];
                cells.sort();
                if cells.iter().any(|&(cx, cy)| base.cols[cx as usize] >> cy & 1 == 1) {
                    continue;
                }
                // 消える3段に、T 以外の土台の穴があれば埋められない
                let rows_mask: u64 = 0b111 << r;
                let t_mask = |cx: usize| cells.iter().filter(|c| c.0 as usize == cx).fold(0u64, |m, c| m | 1 << c.1);
                if (0..WIDTH).any(|cx| holes[cx] & rows_mask & !t_mask(cx) != 0) {
                    continue;
                }
                let mut ideal = *base;
                for cx in 0..WIDTH {
                    ideal.cols[cx] |= rows_mask;
                }
                for &(cx, cy) in &cells {
                    ideal.cols[cx as usize] &= !(1u64 << cy);
                }
                // 屋根(2巡目のミノが作る、溝の上のブロック)を1マスだけ足した形も試す
                let mut roofs: Vec<Option<(i8, i8)>> = vec![None];
                for cx in (x - 2).max(0)..=(x + 2).min(WIDTH as i8 - 1) {
                    for cy in r + 3..=r + 4 {
                        if ideal.cols[cx as usize] >> cy & 1 == 0 {
                            roofs.push(Some((cx, cy)));
                        }
                    }
                }
                let ok = roofs.iter().any(|roof| {
                    let mut b = ideal;
                    if let Some((cx, cy)) = roof {
                        b.cols[*cx as usize] |= 1 << cy;
                    }
                    generate(&b, Piece::T, rules.spawn()).iter().any(|m| {
                        if sorted_cells(&m.placement.location) != cells {
                            return false;
                        }
                        let mut st = PlayerState::new(b);
                        let res = st.apply(&m.placement, rules);
                        res.spin == Spin::Full && res.lines == 3 && st.board.max_height() <= 5
                    })
                });
                if ok {
                    out.push(cells);
                }
            }
        }
    }
    out
}

/// 土台と TST の置き場所から dfs2 の作業用を作る
fn gen2_for(base: &Board, cells: &[(i8, i8); 4], rules: &Rules) -> Gen2 {
    let mut slot = base_holes_of(base);
    let mut tst = [0u64; WIDTH];
    for &(x, y) in cells {
        slot[x as usize] |= 1 << y;
        tst[x as usize] |= 1 << y;
    }
    let r = cells.iter().map(|c| c.1).min().unwrap();
    let budget = std::env::var("GEN_SLOT_BUDGET").ok().and_then(|s| s.parse().ok()).unwrap_or(usize::MAX);
    Gen2 { rules: rules.clone(), soft_max: 1, slot, tst, rows: 8, tst_rows: 0b111 << r, budget, dead: HashSet::new(), found: Vec::new() }
}

fn cells_str(cells: &[(i8, i8); 4]) -> String {
    cells.iter().map(|(x, y)| format!("{x},{y}")).collect::<Vec<_>>().join(";")
}

fn parse_cells(s: &str) -> [(i8, i8); 4] {
    let v: Vec<(i8, i8)> = s
        .split(';')
        .map(|c| {
            let (x, y) = c.split_once(',').unwrap();
            (x.parse().unwrap(), y.parse().unwrap())
        })
        .collect();
    v.try_into().unwrap()
}

/// "図" の文字列(上の段から '/' 区切り)を盤面にする
fn parse_rows(rows: &str) -> Board {
    let lines: Vec<&str> = rows.split('/').collect();
    let mut b = Board::default();
    for (i, line) in lines.iter().enumerate() {
        let y = lines.len() - 1 - i;
        for (x, ch) in line.chars().enumerate() {
            if ch == '#' {
                b.cols[x] |= 1 << y;
            }
        }
    }
    b
}

/// 段階2b: 1巡目の割合が `min_rate` 以上の土台ごとに TST の置き場所を挙げ、2巡目の並びを少し試して TST まで届く割合を数える
fn slot_search(rules: &Rules, min_rate: f64) {
    use std::sync::Mutex;
    let path = std::env::var("GEN_BASE_OUT").unwrap_or_else(|_| "bases.txt".to_string());
    let bases: Vec<(f64, Board)> = read_bases(&path).into_iter().filter(|(r, _)| *r >= min_rate).collect();
    let n_orders: usize = std::env::var("GEN_SLOT_ORDERS").ok().and_then(|s| s.parse().ok()).unwrap_or(8);
    let orders = sample_orders(&all_orders(), n_orders, 4242);
    let started = std::time::Instant::now();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let n_slots = std::sync::atomic::AtomicUsize::new(0);
    let results: Mutex<Vec<(f64, usize, [(i8, i8); 4], Board)>> = Mutex::new(Vec::new());
    let only_count = std::env::var("GEN_SLOT_COUNT").is_ok();
    std::thread::scope(|s| {
        for _ in 0..16 {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(&(rate, base)) = bases.get(i) else { break };
                let slots = tst_slots(&base, rules);
                n_slots.fetch_add(slots.len(), std::sync::atomic::Ordering::Relaxed);
                let mut mine = Vec::new();
                for cells in slots {
                    if only_count {
                        continue;
                    }
                    let mut reach = 0;
                    for order in &orders {
                        let mut g = gen2_for(&base, &cells, rules);
                        dfs2(&mut g, base, order, None, &mut Vec::new(), 0);
                        reach += !g.found.is_empty() as usize;
                    }
                    if reach > 0 {
                        mine.push((rate, reach, cells, base));
                    }
                }
                let mut r = results.lock().unwrap();
                r.extend(mine);
                if i % 1000 == 0 {
                    eprintln!(
                        "  土台 {i} / {}・置き場所 {}・届いたもの {}({:.0}秒)",
                        bases.len(),
                        n_slots.load(std::sync::atomic::Ordering::Relaxed),
                        r.len(),
                        started.elapsed().as_secs_f64()
                    );
                }
            });
        }
    });
    let mut r = results.into_inner().unwrap();
    r.sort_by(|a, b| (b.0 * b.1 as f64).partial_cmp(&(a.0 * a.1 as f64)).unwrap());
    let mut hist = vec![0usize; n_orders + 1];
    for x in &r {
        hist[x.1] += 1;
    }
    println!(
        "土台 {} 個・TST の置き場所 {} 個。2巡目の並び {n_orders} 通りで TST まで届いた数ごとの(土台, 置き場所)の数: {hist:?}({:.0}秒)",
        bases.len(),
        n_slots.into_inner(),
        started.elapsed().as_secs_f64()
    );
    for (rate, reach, cells, b) in r.iter().take(15) {
        println!("  1巡目 {:5.1}%  TST {reach}/{n_orders}  T {}  {}", 100.0 * rate, cells_str(cells), board_rows(b, 4));
    }
    let out = std::env::var("GEN_SLOT_OUT").unwrap_or_else(|_| "slots.txt".to_string());
    let lines: Vec<String> = r.iter().map(|(rate, reach, cells, b)| format!("{rate:.3} {reach} {} {}", cells_str(cells), board_rows(b, 4))).collect();
    std::fs::write(&out, lines.join("\n")).unwrap();
    println!("書き出した: {out}({} 個)", lines.len());
}

/// 段階2b: (土台, TST の置き場所) の上位 `top` 個を、2巡目(TST まで)と3巡目(TSD+パフェ)で評価する(並びを間引く)。
/// 土台ごとに、並びごとに一番よい置き場所・盤面を選んだものでも比べる。
fn eval_slots(rules: &Rules, top: usize) {
    use std::collections::HashMap;
    use std::sync::Mutex;
    let path = std::env::var("GEN_SLOT_OUT").unwrap_or_else(|_| "slots.txt".to_string());
    let text = std::fs::read_to_string(&path).expect("置き場所の一覧が無い");
    let mut items: Vec<(f64, [(i8, i8); 4], Board)> = text
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split(' ').collect();
            Some((f[0].parse().ok()?, parse_cells(f[2]), parse_rows(f[3])))
        })
        .take(top)
        .collect();
    // くろみつ砲の土台と置き場所も比べるために入れる(一覧に無ければ)
    let kuro = Board { cols: priority_templates()[0].forms.iter().find(|f| f.is_first()).unwrap().cells };
    let kuro_c = {
        let m = mirror(&kuro);
        if m.cols < kuro.cols { m } else { kuro }
    };
    if !items.iter().any(|(_, _, b)| *b == kuro_c) {
        for cells in tst_slots(&kuro_c, rules) {
            items.push((0.783, cells, kuro_c));
        }
    }
    let orders = all_orders();
    let n2: usize = std::env::var("GEN_EVAL_BAG2").ok().and_then(|s| s.parse().ok()).unwrap_or(48);
    let n3: usize = std::env::var("GEN_EVAL_BAG3").ok().and_then(|s| s.parse().ok()).unwrap_or(120);
    let bag2 = sample_orders(&orders, n2, 777);
    let bag3 = sample_orders(&orders, n3, 999);
    let cover: Mutex<HashMap<Board, f64>> = Mutex::new(HashMap::new());
    let started = std::time::Instant::now();
    let next = std::sync::atomic::AtomicUsize::new(0);
    // 置き場所ごとの、並びごとの一番よい値(TST に届かなければ None)
    let per_item: Mutex<Vec<(usize, Vec<Option<f64>>)>> = Mutex::new(Vec::new());
    std::thread::scope(|s| {
        for _ in 0..16 {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(&(_, cells, base)) = items.get(i) else { break };
                let mut vals = Vec::new();
                for order in &bag2 {
                    let mut g = gen2_for(&base, &cells, rules);
                    dfs2(&mut g, base, order, None, &mut Vec::new(), 0);
                    if g.found.is_empty() {
                        vals.push(None);
                        continue;
                    }
                    let mut best = 0.0f64;
                    if std::env::var("GEN_EVAL_DEBUG").is_ok() {
                        for (b, ..) in g.found.iter().take(5) {
                            eprintln!("  TST の後 {}", board_rows(b, 6));
                        }
                    }
                    for (b, ..) in &g.found {
                        let known = cover.lock().unwrap().get(b).copied();
                        let c = known.unwrap_or_else(|| {
                            let mut gen = Gen { rules: rules.clone(), soft_max: 1, dead: HashSet::new() };
                            // GEN_EVAL_ANYPC: TSD+パフェが無理な並びでは、TSD を飛ばすパフェ(TST→パフェ)でもよいとして数える
                            let any_pc = std::env::var("GEN_EVAL_ANYPC").is_ok();
                            let mut skip = Gen { rules: rules.clone(), soft_max: 1, dead: HashSet::new() };
                            let n = bag3
                                .iter()
                                .filter(|o| {
                                    let (mut cur, mut segs) = (Vec::new(), Vec::new());
                                    dfs(&mut gen, *b, &o[..], None, 5, false, &mut cur, *b, &mut segs)
                                        || (any_pc && {
                                            let (mut cur, mut segs) = (Vec::new(), Vec::new());
                                            dfs(&mut skip, *b, &o[..], None, 5, true, &mut cur, *b, &mut segs)
                                        })
                                })
                                .count();
                            let c = n as f64 / bag3.len() as f64;
                            cover.lock().unwrap().insert(*b, c);
                            c
                        });
                        best = best.max(c);
                    }
                    vals.push(Some(best));
                }
                let mut p = per_item.lock().unwrap();
                p.push((i, vals));
                if p.len() % 20 == 0 {
                    eprintln!("  {} / {} 個({:.0}秒)", p.len(), items.len(), started.elapsed().as_secs_f64());
                }
            });
        }
    });
    let per_item = per_item.into_inner().unwrap();
    let summary = |vals: &[Option<f64>]| {
        let tst = vals.iter().filter(|v| v.is_some()).count() as f64 / vals.len() as f64;
        let s23 = vals.iter().map(|v| v.unwrap_or(0.0)).sum::<f64>() / vals.len() as f64;
        (tst, s23)
    };
    // 置き場所ごと
    let mut rows: Vec<(f64, f64, f64, f64, String, Board)> = per_item
        .iter()
        .map(|(i, vals)| {
            let (rate, cells, b) = items[*i];
            let (tst, s23) = summary(vals);
            (rate * s23, rate, tst, s23, cells_str(&cells), b)
        })
        .collect();
    rows.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    println!(
        "(土台, TST の置き場所) {} 個を評価({:.0}秒)。点数 = 1巡目の割合 × 2巡目の並びごとに一番よい TST の後の盤面の TSD+パフェ",
        rows.len(),
        started.elapsed().as_secs_f64()
    );
    for (score, rate, tst, s23, cells, b) in rows.iter().take(20) {
        println!(
            "  点数 {:5.1}%  1巡目 {:5.1}%  TST まで {:5.1}%  TSD+パフェ {:5.1}%  T {cells}  {}{}",
            100.0 * score,
            100.0 * rate,
            100.0 * tst,
            100.0 * s23,
            board_rows(b, 4),
            if *b == kuro_c { "  ← くろみつ砲" } else { "" }
        );
    }
    // 土台ごと(並びごとに一番よい置き場所を選ぶ)
    let mut by_base: HashMap<Board, (f64, Vec<Option<f64>>)> = HashMap::new();
    for (i, vals) in &per_item {
        let (rate, _, b) = items[*i];
        let e = by_base.entry(b).or_insert((rate, vec![None; vals.len()]));
        for (a, v) in e.1.iter_mut().zip(vals) {
            if let Some(v) = v {
                *a = Some(a.unwrap_or(0.0).max(*v));
            }
        }
    }
    let mut bases: Vec<(f64, f64, f64, f64, Board)> = by_base
        .iter()
        .map(|(b, (rate, vals))| {
            let (tst, s23) = summary(vals);
            (rate * s23, *rate, tst, s23, *b)
        })
        .collect();
    bases.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
    println!("土台ごと(並びごとに一番よい置き場所):");
    for (k, (score, rate, tst, s23, b)) in bases.iter().enumerate() {
        if k < 20 || *b == kuro_c {
            println!(
                "  {:3} 位 点数 {:5.1}%  1巡目 {:5.1}%  TST まで {:5.1}%  TSD+パフェ {:5.1}%  {}{}",
                k + 1,
                100.0 * score,
                100.0 * rate,
                100.0 * tst,
                100.0 * s23,
                board_rows(b, 4),
                if *b == kuro_c { "  ← くろみつ砲" } else { "" }
            );
        }
    }
    let out = std::env::var("GEN_EVAL_OUT").unwrap_or_else(|_| "eval.txt".to_string());
    let lines: Vec<String> = rows
        .iter()
        .map(|(score, rate, tst, s23, cells, b)| format!("{score:.3} {rate:.3} {tst:.3} {s23:.3} {cells} {}", board_rows(b, 4)))
        .collect();
    std::fs::write(&out, lines.join("\n")).unwrap();
    println!("書き出した: {out}");
}

/// 図1つを JSON にする(`last` は最後のミノを撃つ T とするか)
fn form_json(section: &str, existing: &Board, items: &[Item], last: bool) -> String {
    let n = items.len();
    let items_json: Vec<String> = items
        .iter()
        .enumerate()
        .map(|(i, (p, c))| format!(r#"{{"piece":"{}","cells":[{}],"last":{}}}"#, p.to_char(), cells_json(c.iter().copied()), last && i + 1 == n))
        .collect();
    format!(r#"{{"section":"{section}","existing":[{}],"items":[{}]}}"#, cells_json(board_cells(existing)), items_json.join(","))
}

/// make_template の2巡目の結果を文字にする(1行に1並び。`;` で区切る。盤面の列/置き方/数)
fn save_per_order(v: &[Vec<(Board, Vec<Item>, u32)>]) -> String {
    let mut out = String::new();
    for f in v {
        let es: Vec<String> = f
            .iter()
            .map(|(b, items, n)| {
                let cols: Vec<String> = b.cols.iter().map(|c| format!("{c:x}")).collect();
                let its: Vec<String> = items
                    .iter()
                    .map(|(p, c)| format!("{}{}", p.to_char(), c.iter().map(|(x, y)| format!(":{x},{y}")).collect::<String>()))
                    .collect();
                format!("{}/{}/{n}", cols.join(","), its.join(" "))
            })
            .collect();
        out.push_str(&es.join(";"));
        out.push(char::from(10u8));
    }
    out
}

/// `save_per_order` の逆
fn load_per_order(t: &str) -> Vec<Vec<(Board, Vec<Item>, u32)>> {
    t.lines()
        .map(|line| {
            line.split(';')
                .filter(|e| !e.is_empty())
                .map(|e| {
                    let mut parts = e.split('/');
                    let cols: Vec<u64> = parts.next().unwrap().split(',').map(|c| u64::from_str_radix(c, 16).unwrap()).collect();
                    let items: Vec<Item> = parts
                        .next()
                        .unwrap()
                        .split(' ')
                        .map(|it| {
                            let mut ps = it.split(':');
                            let p = Piece::from_char(ps.next().unwrap().chars().next().unwrap()).unwrap();
                            let c: Vec<(i8, i8)> = ps
                                .map(|xy| {
                                    let (x, y) = xy.split_once(',').unwrap();
                                    (x.parse().unwrap(), y.parse().unwrap())
                                })
                                .collect();
                            (p, c.try_into().unwrap())
                        })
                        .collect();
                    let n = parts.next().unwrap().parse().unwrap();
                    (Board { cols: cols.try_into().unwrap() }, items, n)
                })
                .collect()
        })
        .collect()
}

/// 左右反転したミノ
fn mirror_piece(p: Piece) -> Piece {
    match p {
        Piece::L => Piece::J,
        Piece::J => Piece::L,
        Piece::S => Piece::Z,
        Piece::Z => Piece::S,
        p => p,
    }
}

/// 左右反転した置き方(マスは並べ直す)
fn mirror_item(item: &Item) -> Item {
    let mut c = item.1.map(|(x, y)| (WIDTH as i8 - 1 - x, y));
    c.sort();
    (mirror_piece(item.0), c)
}

/// 土台を7種1個ずつに分ける置き方(どれかの順でハードドロップだけで置けるもの)をすべて挙げる
fn base_decompositions(target: &Board, rules: &Rules) -> Vec<Vec<Item>> {
    fn go(b: Board, used: u8, items: &mut Vec<Item>, target: &Board, rules: &Rules, seen: &mut HashSet<Vec<Item>>, out: &mut Vec<Vec<Item>>) {
        let mut key = items.clone();
        key.sort();
        if !seen.insert(key.clone()) {
            return;
        }
        if used == 0x7f {
            out.push(key);
            return;
        }
        for (i, &p) in Piece::ALL.iter().enumerate() {
            if used >> i & 1 == 1 {
                continue;
            }
            for m in generate(&b, p, rules.spawn()) {
                let loc = m.placement.location;
                if m.soft_drop_sections != 0 || !loc.cells().iter().all(|&(x, y)| y >= 0 && target.cols[x as usize] >> y & 1 == 1) {
                    continue;
                }
                let mut nb = b;
                nb.place(&loc);
                items.push((p, sorted_cells(&loc)));
                go(nb, used | 1 << i, items, target, rules, seen, out);
                items.pop();
            }
        }
    }
    let (mut seen, mut out) = (HashSet::new(), Vec::new());
    go(Board::default(), 0, &mut Vec::new(), target, rules, &mut seen, &mut out);
    out
}

/// TST の後の盤面から、3巡目の TSD+パフェの図を作る(並び `orders` で解を探し、区切りごとの図にする)。届いた並びの数も返す
fn pc_forms(board: &Board, orders: &[[Piece; 7]], rules: &Rules, skip_tsd: bool) -> (usize, Vec<Segment>) {
    let results = solve_pc_with(board, orders, rules, 1, skip_tsd);
    let solved = results.iter().filter(|r| r.is_some()).count();
    let distinct: BTreeSet<&Vec<Segment>> = results.iter().flatten().collect();
    let mut forms: BTreeSet<Segment> = BTreeSet::new();
    for segs in &distinct {
        for seg in segs.iter() {
            let mut items = seg.items.clone();
            let last = items.pop().unwrap();
            items.sort();
            items.push(last);
            forms.insert(Segment { board: seg.board.clone(), items, tsd: seg.tsd });
        }
    }
    (solved, forms.into_iter().collect())
}

/// 段階2b: 見つけた土台と TST の置き場所から、テンプレの図(1〜3巡目、左右反転を含む)を作って JSON に書き出す。
/// GEN_MAKE=<名前> GEN_MAKE_BASE=<土台の図> GEN_MAKE_SLOT=<T のマス> GEN_MAKE_OUT=<出力> GEN_MAKE_BOARDS=<TST の後の盤面の数の上限>
fn make_template(rules: &Rules, name: &str) {
    use std::collections::HashMap;
    let base = parse_rows(&std::env::var("GEN_MAKE_BASE").expect("GEN_MAKE_BASE"));
    let cells = parse_cells(&std::env::var("GEN_MAKE_SLOT").expect("GEN_MAKE_SLOT"));
    let out = std::env::var("GEN_MAKE_OUT").unwrap_or_else(|_| "made.json".to_string());
    let max_boards: usize = std::env::var("GEN_MAKE_BOARDS").ok().and_then(|s| s.parse().ok()).unwrap_or(12);
    let n3: usize = std::env::var("GEN_EVAL_BAG3").ok().and_then(|s| s.parse().ok()).unwrap_or(240);
    let started = std::time::Instant::now();
    let orders = all_orders();
    // 1巡目
    let decomp = base_decompositions(&base, rules);
    println!("{name}: 土台 {}・1巡目の分け方 {} 通り", board_rows(&base, 4), decomp.len());
    // 2巡目: 並び5040通りで届く TST の後の盤面と置き方
    let cache = std::env::var("GEN_MAKE_CACHE").ok();
    let cached = cache.as_ref().and_then(|c| std::fs::read_to_string(c).ok()).map(|t| load_per_order(&t));
    let per_order: Vec<Vec<(Board, Vec<Item>, u32)>> = if let Some(v) = cached {
        v
    } else {
        std::thread::scope(|s| {
        let chunks: Vec<&[[Piece; 7]]> = orders.chunks(orders.len().div_ceil(16)).collect();
        let hs: Vec<_> = chunks
            .into_iter()
            .map(|c| {
                let base = &base;
                s.spawn(move || {
                    c.iter()
                        .map(|o| {
                            let mut g = gen2_for(base, &cells, rules);
                            dfs2(&mut g, *base, o, None, &mut Vec::new(), 0);
                            g.found
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        hs.into_iter().flat_map(|h| h.join().unwrap()).collect()
        })
    };
    if let Some(c) = &cache {
        if !std::path::Path::new(c).exists() {
            std::fs::write(c, save_per_order(&per_order)).expect("書き出せない");
        }
    }
    let reach = per_order.iter().filter(|f| !f.is_empty()).count();
    let boards: BTreeSet<Vec<u64>> = per_order.iter().flatten().map(|(b, ..)| b.cols.to_vec()).collect();
    println!("  2巡目: TST まで届く並び {reach}/5040・TST の後の盤面 {} 通り({:.0}秒)", boards.len(), started.elapsed().as_secs_f64());
    // TST の後の盤面ごとの、3巡目の TSD+パフェの割合(間引いた並びで)
    let bag3 = sample_orders(&orders, n3, 999);
    let board_list: Vec<Board> = boards.iter().map(|v| Board { cols: v.clone().try_into().unwrap() }).collect();
    let next = std::sync::atomic::AtomicUsize::new(0);
    let res = std::sync::Mutex::new(vec![0.0; board_list.len()]);
    // GEN_MAKE_PC_AFTER_TST: TST の後は TSD を飛ばしてパフェでもよい(TST→パフェ型)。物差しはどのパフェでも
    let pc_after = std::env::var("GEN_MAKE_PC_AFTER_TST").is_ok();
    std::thread::scope(|s| {
        for _ in 0..16 {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let Some(b) = board_list.get(i) else { break };
                let mut gen = Gen { rules: rules.clone(), soft_max: 1, dead: HashSet::new() };
                let mut skip = Gen { rules: rules.clone(), soft_max: 1, dead: HashSet::new() };
                let n = bag3
                    .iter()
                    .filter(|o| {
                        let (mut cur, mut segs) = (Vec::new(), Vec::new());
                        dfs(&mut gen, *b, &o[..], None, 5, false, &mut cur, *b, &mut segs)
                            || (pc_after && {
                                let (mut cur, mut segs) = (Vec::new(), Vec::new());
                                dfs(&mut skip, *b, &o[..], None, 5, true, &mut cur, *b, &mut segs)
                            })
                    })
                    .count();
                res.lock().unwrap()[i] = n as f64 / bag3.len() as f64;
            });
        }
    });
    let covers = res.into_inner().unwrap();
    let cover: HashMap<Board, f64> = board_list.iter().copied().zip(covers.iter().copied()).collect();
    let best_all: f64 = per_order.iter().map(|f| f.iter().map(|(b, ..)| cover[b]).fold(0.0, f64::max)).sum::<f64>() / 5040.0;
    // TST の後の盤面を、並びごとの一番よい値の合計が大きくなるように1つずつ選ぶ
    let mut chosen: Vec<Board> = Vec::new();
    let mut cur = vec![0.0f64; per_order.len()];
    while chosen.len() < max_boards {
        let mut best: Option<(f64, Board)> = None;
        for b in &board_list {
            if chosen.contains(b) || cover[b] == 0.0 {
                continue;
            }
            let gain: f64 = per_order
                .iter()
                .zip(&cur)
                .map(|(f, &c)| if f.iter().any(|(fb, ..)| fb == b) { (cover[b] - c).max(0.0) } else { 0.0 })
                .sum();
            if best.is_none_or(|(g, _)| gain > g) {
                best = Some((gain, *b));
            }
        }
        let Some((gain, b)) = best else { break };
        let min_gain: f64 = std::env::var("GEN_MAKE_MIN_GAIN").ok().and_then(|s| s.parse().ok()).unwrap_or(0.002);
        if gain / 5040.0 < min_gain {
            break;
        }
        for (f, c) in per_order.iter().zip(cur.iter_mut()) {
            if f.iter().any(|(fb, ..)| *fb == b) {
                *c = c.max(cover[&b]);
            }
        }
        chosen.push(b);
        println!(
            "  盤面 {} を選ぶ: TSD+パフェ {:.1}%・届く並び {}・合計 {:.1}%(すべての盤面なら {:.1}%)",
            board_rows(&b, 5),
            100.0 * cover[&b],
            per_order.iter().filter(|f| f.iter().any(|(fb, ..)| *fb == b)).count(),
            100.0 * cur.iter().sum::<f64>() / 5040.0,
            100.0 * best_all
        );
    }
    // TST まで届くのに選んだ盤面に届かない並びが無くなるように、届く並びの多い盤面を足す(TST だけでも撃てるように)
    let max_cover: usize = std::env::var("GEN_MAKE_COVER").ok().and_then(|s| s.parse().ok()).unwrap_or(0);
    for _ in 0..max_cover {
        let uncovered: Vec<&Vec<(Board, Vec<Item>, u32)>> =
            per_order.iter().filter(|f| !f.is_empty() && !f.iter().any(|(b, ..)| chosen.contains(b))).collect();
        if uncovered.is_empty() {
            break;
        }
        let mut count: HashMap<Board, (usize, f64)> = HashMap::new();
        for f in &uncovered {
            let mut seen = HashSet::new();
            for (b, ..) in f.iter() {
                if seen.insert(*b) {
                    let e = count.entry(*b).or_insert((0, cover[b]));
                    e.0 += 1;
                }
            }
        }
        let (b, (n, c)) = count.into_iter().max_by(|x, y| (x.1 .0 as f64 * (0.5 + x.1 .1)).partial_cmp(&(y.1 .0 as f64 * (0.5 + y.1 .1))).unwrap()).unwrap();
        println!("  届かない並び {} のうち {n} に届く盤面 {} を足す(TSD+パフェ {:.1}%)", uncovered.len(), board_rows(&b, 5), 100.0 * c);
        chosen.push(b);
    }
    let covered = per_order.iter().filter(|f| f.iter().any(|(b, ..)| chosen.contains(b))).count();
    println!("  選んだ盤面 {} 個に届く並び {covered}/5040", chosen.len());
    // 2巡目の図: 選んだ盤面に届く置き方(重複を除く。T は最後)
    let mut second: BTreeSet<Vec<Item>> = BTreeSet::new();
    for (b, items, _) in per_order.iter().flatten() {
        if chosen.contains(b) {
            let mut v = items.clone();
            let t = v.pop().unwrap();
            v.sort();
            v.push(t);
            second.insert(v);
        }
    }
    println!("  2巡目の図 {} 個", second.len());
    // 3巡目の図
    let mut third: Vec<Segment> = Vec::new();
    for b in &chosen {
        let (solved, forms) = pc_forms(b, &orders, rules, false);
        println!("  盤面 {}: TSD+パフェ {}/5040 ({:.1}%)・図 {}", board_rows(b, 5), solved, 100.0 * solved as f64 / 5040.0, forms.len());
        third.extend(forms);
    }
    // TSD を撃てない時の逃げ道: TSD を撃たずにパフェ(TST→パフェ)の図も入れる(くろみつ砲などの TBA の図の「パフェ狙い」と同じ役目)
    let mut skip: Vec<Segment> = Vec::new();
    if pc_after || std::env::var("GEN_MAKE_SKIP_TSD").is_ok() {
        for b in &chosen {
            let (solved, forms) = pc_forms(b, &orders, rules, true);
            println!("  盤面 {}: TSD なしのパフェ {}/5040 ({:.1}%)・図 {}", board_rows(b, 5), solved, 100.0 * solved as f64 / 5040.0, forms.len());
            skip.extend(forms);
        }
    }
    // JSON(左右反転も入れる)
    let mut forms_json: Vec<String> = Vec::new();
    for flip in [false, true] {
        let mi = |v: &[Item]| -> Vec<Item> {
            if flip {
                v.iter().map(mirror_item).collect()
            } else {
                v.to_vec()
            }
        };
        let mb = |b: &Board| if flip { mirror(b) } else { *b };
        for d in &decomp {
            forms_json.push(form_json("自動 > 1巡目", &Board::default(), &mi(d), false));
        }
        for v in &second {
            forms_json.push(form_json("自動 > 2巡目", &mb(&base), &mi(v), true));
        }
        for seg in &third {
            let b = Board { cols: seg.board.clone().try_into().unwrap() };
            let section = if seg.tsd { "自動 > 3巡目 TSD" } else { "自動 > 3巡目" };
            forms_json.push(form_json(section, &mb(&b), &mi(&seg.items), seg.tsd));
        }
        for seg in &skip {
            let b = Board { cols: seg.board.clone().try_into().unwrap() };
            forms_json.push(form_json("自動 > 3巡目 パフェ狙い", &mb(&b), &mi(&seg.items), false));
        }
    }
    let flag = if pc_after { ",\"pc_after_tst\":true" } else { "" };
    let json = format!("[{{\"name\":\"{name}\"{flag},\"forms\":[\n{}\n]}}]\n", forms_json.join(",\n"));
    std::fs::write(&out, json).expect("書き出せない");
    println!("図 {} 個を {out} に書き出した({:.0}秒)", forms_json.len(), started.elapsed().as_secs_f64());
}
