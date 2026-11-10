//! 開幕パフェ(PCO)の探索(2026-11-09)。
//!
//! 【経緯】相手が開幕 C4W のとき、ふつうの火力(TSD・テトリス)は1秒あたり約1.9行で、C4W(約2.6行)に追いつけない。
//! C4W を倒せるのは、相手が REN を始める前(開始から約6秒)にまとめて届いた攻撃だけで、今の早撃ちは約4行しか届いていなかった。
//! パフェは10ミノ(約3.5秒)で10行を送れるので、REN の前に届けられる。
//!
//! 4段以内の盤面から、見えているミノ(と袋の残りから来うるミノ)でパフェできる手を探す。
//! 置き方はハードドロップだけで入るものに限る(速く置けて、実機での置きミスも少ない)。穴を作る置き方は使わない。
//! 【2026-11-09・ユーザー「進める」】実機16局でパフェが3局だけだったので、方針「全自動のパフェはソフトドロップ1回以下」の範囲で、
//! 下が空いたマス(後でソフトドロップで差し込む所)を残す置き方と、ソフトドロップ1回(空中で止めない)で差し込む置き方も
//! 使えるようにした(`spawn` を渡した時。ハードドロップだけで組めない時だけ使う。いつも混ぜると探索が広がりすぎて時間切れになる)。
//! パフェの成功率は 500局で 53.8%→65.0%、対戦でパフェの回数は1局 0.67→0.79 回に増えたが、勝率は上がらなかった
//! (no_c4w に 59.7%→57.3%(4000局)・59.8%→57.5%(3000局)、C4W に 35.7%→34.8%・36.3%→34.2%)。標準では使わない。

use std::collections::HashSet;
use std::hash::{BuildHasherDefault, Hasher};
use std::time::{Duration, Instant};

use tetrisai_core::board::WIDTH;
use tetrisai_core::{generate, Board, Move, Piece, PieceLocation, Rotation, Rules, Spawn};

/// 1手目の候補(置く位置。HOLD を使うかはミノの種類で分かる)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Action {
    pub loc: PieceLocation,
}

/// 形の違う向きだけ(I・S・Z は北と東、O は北だけ。位置は x を全部試すので、残りの向きは同じ形になる)
fn rotations(p: Piece) -> &'static [Rotation] {
    match p {
        Piece::O => &[Rotation::North],
        Piece::I | Piece::S | Piece::Z => &[Rotation::North, Rotation::East],
        _ => &Rotation::ALL,
    }
}

/// `rows` 段の中に置ける位置。ハードドロップで入る位置と、`spawn` があれば、下が空いたマスがある盤面で
/// ソフトドロップ1回(空中で止めない)で差し込める位置。
fn drops(b: &Board, p: Piece, rows: u32, spawn: Option<Spawn>, out: &mut Vec<PieceLocation>) {
    out.clear();
    if let Some(spawn) = spawn.filter(|_| covered(b)) {
        for m in generate(b, p, spawn) {
            let loc = m.placement.location;
            if m.soft_drop_sections <= 1 && m.mid_air_stops == 0 && loc.cells().iter().all(|&(_, cy)| cy >= 0 && (cy as u32) < rows) {
                let cells = loc.sorted_cells();
                if !out.iter().any(|l: &PieceLocation| l.sorted_cells() == cells) {
                    out.push(loc);
                }
            }
        }
        return;
    }
    for &rot in rotations(p) {
        for x in -2..WIDTH as i8 + 2 {
            let high = PieceLocation::new(p, rot, x, 10);
            if high.cells().iter().any(|&(cx, cy)| cx < 0 || cx >= WIDTH as i8 || cy < 0) {
                continue;
            }
            let d = b.drop_distance(&high);
            let landed = PieceLocation { y: high.y - d, ..high };
            if landed.cells().iter().all(|&(_, cy)| cy >= 0 && (cy as u32) < rows) {
                out.push(landed);
            }
        }
    }
}

/// 埋まったマスの下に空いたマスがある列があるか(列のビットが下から続いていない)
fn covered(b: &Board) -> bool {
    b.cols.iter().any(|&c| c & (c + 1) != 0)
}

/// 下が空いたマスの数の上限(ソフトドロップで差し込む所。多いと探索が広がるだけで、まず埋まらない)
const MAX_COVERED: u32 = 4;

/// `rows` 段でパフェを目指せる盤面か(`soft` なら下が空いたマスを `MAX_COVERED` 個まで許す)。
fn fits(b: &Board, rows: u32, soft: bool) -> bool {
    if !soft {
        return ok(b, rows);
    }
    let (mut region, mut covered) = (0, 0);
    for x in 0..WIDTH {
        let c = b.cols[x];
        let h = 64 - c.leading_zeros();
        if h > rows {
            return false;
        }
        let filled = c.count_ones();
        covered += h - filled;
        if filled == rows {
            if region % 4 != 0 {
                return false;
            }
            region = 0;
        } else {
            region += rows - filled;
        }
    }
    covered <= MAX_COVERED && region % 4 == 0 && (covered == 0 || !sealed(b))
}

/// 横から差し込めない空きマスがあるか: 下が空いたマスから同じ行を左右にたどって、上に何も無い列に出られなければ、
/// ソフトドロップで差し込めない(本当の穴)。
fn sealed(b: &Board) -> bool {
    let heights: [u32; WIDTH] = std::array::from_fn(|x| 64 - b.cols[x].leading_zeros());
    let empty = |x: usize, y: u32| b.cols[x] >> y & 1 == 0;
    for x in 0..WIDTH {
        for y in 0..heights[x] {
            if !empty(x, y) {
                continue;
            }
            let open = |range: &mut dyn Iterator<Item = usize>| {
                for xx in range {
                    if !empty(xx, y) {
                        return false;
                    }
                    if heights[xx] <= y {
                        return true;
                    }
                }
                false
            };
            if !open(&mut (x + 1..WIDTH)) && !open(&mut (0..x).rev()) {
                return true;
            }
        }
    }
    false
}


/// `rows` 段でパフェを目指せる盤面か: 穴が無く、どの列も `rows` 段以内で、`rows` 段まで埋まった列で区切られた
/// 空きの区画がどれも4の倍数のマス。
fn ok(b: &Board, rows: u32) -> bool {
    let mut region = 0;
    for x in 0..WIDTH {
        let c = b.cols[x];
        let h = 64 - c.leading_zeros();
        if h > rows || c != (1u64 << h) - 1 {
            return false;
        }
        if h == rows {
            if region % 4 != 0 {
                return false;
            }
            region = 0;
        } else {
            region += rows - h;
        }
    }
    region % 4 == 0
}

/// パフェまでに埋める段数。開幕から `placed` 個置いた盤面なら、消した行数は (4 × placed − マス数) / 10 で、
/// 残りは 4 − 消した行数(盤面から計算し直すと、途中で2段のパフェなどに目標が変わってしまう)。
pub fn rows_left(board: &Board, placed: u32, soft: bool) -> Option<u32> {
    let cells: u32 = board.cols.iter().map(|c| c.count_ones()).sum();
    let gone = (4 * placed).checked_sub(cells)?;
    if gone % 10 != 0 || gone / 10 > 4 {
        return None;
    }
    let rows = 4 - gone / 10;
    (rows > 0 && fits(board, rows, soft)).then_some(rows)
}

/// 詰めた鍵(u64)用の速いハッシュ
#[derive(Default)]
struct KeyHasher(u64);
impl Hasher for KeyHasher {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, _: &[u8]) {
        unreachable!()
    }
    fn write_u64(&mut self, v: u64) {
        self.0 = (v ^ (v >> 29)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    }
}
type KeySet = HashSet<u64, BuildHasherDefault<KeyHasher>>;

/// 盤面(4段以内)・段数・位置・HOLD を1つの u64 に詰める
fn key(b: &Board, rows: u32, idx: usize, hold: Option<Piece>) -> u64 {
    let mut k = 0u64;
    for c in &b.cols {
        k = (k << 4) | (c & 0xF);
    }
    (((k << 3 | rows as u64) << 5 | idx as u64) << 3) | hold.map_or(7, |p| p as u64)
}

struct Solver<'a> {
    queue: &'a [Piece],
    failed: KeySet,
    deadline: Instant,
    aborted: bool,
    buf: Vec<Vec<PieceLocation>>,
    nodes: u32,
    spawn: Option<Spawn>,
}

impl Solver<'_> {
    /// 置けるミノ(置くミノ, 置いた後の HOLD, 次の位置)
    fn options(&self, idx: usize, hold: Option<Piece>) -> Vec<(Piece, Option<Piece>, usize)> {
        let mut v = Vec::with_capacity(2);
        let Some(&cur) = self.queue.get(idx) else { return v };
        v.push((cur, hold, idx + 1));
        match hold {
            Some(h) if h != cur => v.push((h, Some(cur), idx + 1)),
            None => {
                if let Some(&n) = self.queue.get(idx + 1) {
                    if n != cur {
                        v.push((n, Some(cur), idx + 2));
                    }
                }
            }
            _ => {}
        }
        v
    }

    fn solvable(&mut self, b: Board, rows: u32, idx: usize, hold: Option<Piece>, depth: usize) -> bool {
        if rows == 0 {
            return true;
        }
        if self.aborted {
            return false;
        }
        self.nodes += 1;
        if self.nodes % 256 == 0 && Instant::now() > self.deadline {
            self.aborted = true;
            return false;
        }
        // 残りのミノが足りなければ取れない
        let cells: u32 = b.cols.iter().map(|c| c.count_ones()).sum();
        let need = ((rows * 10 - cells) / 4) as usize;
        let rest = &self.queue[idx.min(self.queue.len())..];
        // 1回置くたびに操作ミノを1つ使う(HOLD のミノを置くときも、操作ミノが HOLD に入る)
        if rest.len() < need {
            return false;
        }
        let key = key(&b, rows, idx, hold);
        if self.failed.contains(&key) {
            return false;
        }
        if self.buf.len() <= depth {
            self.buf.push(Vec::new());
        }
        for (piece, new_hold, next) in self.options(idx, hold) {
            let mut locs = std::mem::take(&mut self.buf[depth]);
            drops(&b, piece, rows, self.spawn, &mut locs);
            let mut found = false;
            for loc in &locs {
                let mut nb = b;
                nb.place(loc);
                let cleared = nb.clear_full_lines();
                let nrows = rows - cleared;
                if !fits(&nb, nrows, self.spawn.is_some()) {
                    continue;
                }
                if self.solvable(nb, nrows, next, new_hold, depth + 1) {
                    found = true;
                    break;
                }
            }
            self.buf[depth] = locs;
            if found {
                return true;
            }
        }
        if !self.aborted {
            self.failed.insert(key);
        }
        false
    }
}

/// 見えているミノの後に来うるミノの列(`len` 個)。袋の残り `bag`(空なら新しい袋)の並べ替えに、新しい袋を続ける。
/// 組み合わせが `limit` を超えたら、決まった乱数で `limit` 個を選ぶ。
fn continuations(bag: &[Piece], len: usize, limit: usize, seed: u64) -> Vec<Vec<Piece>> {
    fn rec(bag: Vec<Piece>, len: usize, prefix: &mut Vec<Piece>, out: &mut Vec<Vec<Piece>>, cap: usize) {
        if out.len() >= cap {
            return;
        }
        if prefix.len() == len {
            out.push(prefix.clone());
            return;
        }
        let bag = if bag.is_empty() { Piece::ALL.to_vec() } else { bag };
        for i in 0..bag.len() {
            let mut rest = bag.clone();
            let p = rest.remove(i);
            prefix.push(p);
            rec(rest, len, prefix, out, cap);
            prefix.pop();
        }
    }
    let mut all = Vec::new();
    rec(bag.to_vec(), len, &mut Vec::new(), &mut all, 5040);
    if all.len() <= limit {
        return all;
    }
    // 決まった乱数で間引く(xorshift)
    let mut rng = seed | 1;
    let mut picked = Vec::with_capacity(limit);
    for _ in 0..limit {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        let i = (rng % all.len() as u64) as usize;
        picked.push(all.swap_remove(i));
    }
    picked
}

/// 見えていないミノの来方を試す数。300局で 16通り 56.7%・48通り 58.3%・150通り 61.3%(1手の時間が2〜3倍)。2026-11-09
const SAMPLES: usize = 48;

/// パフェできる見込みが一番高い1手目と、その見込み(0〜1)。見込みが無ければ None。
/// `rows` はパフェまでに埋める段数(`rows_left`)、`queue[0]` が操作ミノ、`bag` は NEXT の後で今の袋に残っているミノ
/// (空なら新しい袋)。`spawn` を渡すとソフトドロップ1回の置き方も使う(None ならハードドロップだけ)。
pub fn best_action(
    board: &Board,
    rows: u32,
    hold: Option<Piece>,
    queue: &[Piece],
    bag: &[Piece],
    budget: Duration,
    spawn: Option<Spawn>,
) -> Option<(Action, f32)> {
    let deadline = Instant::now() + budget;
    // まずハードドロップだけで探す(速い)。見込みが無い時か、盤面にもう下の空いたマスがある時だけ、ソフトドロップの置き方も使う
    if !covered(board) {
        if let Some(r) = search_action(board, rows, hold, queue, bag, deadline, None) {
            return Some(r);
        }
    }
    spawn?;
    search_action(board, rows, hold, queue, bag, deadline, spawn)
}

fn search_action(
    board: &Board,
    rows: u32,
    hold: Option<Piece>,
    queue: &[Piece],
    bag: &[Piece],
    deadline: Instant,
    spawn: Option<Spawn>,
) -> Option<(Action, f32)> {
    let soft = spawn.is_some();
    if rows == 0 || rows > 4 || !fits(board, rows, soft) {
        return None;
    }
    let cells: u32 = board.cols.iter().map(|c| c.count_ones()).sum();
    let need = ((rows * 10 - cells) / 4) as usize;
    if need == 0 || need > 10 {
        return None;
    }
    // HOLD を使う分を1つ足す
    let extra = (need + 1).saturating_sub(queue.len());
    let seed = board.cols.iter().fold(queue.len() as u64, |a, &c| a.wrapping_mul(31).wrapping_add(c));
    let conts = if extra == 0 { vec![Vec::new()] } else { continuations(bag, extra, SAMPLES, seed) };

    // 1手目の候補
    let first_solver = Solver { queue, failed: KeySet::default(), deadline, aborted: false, buf: Vec::new(), nodes: 0, spawn };
    let mut firsts: Vec<(Action, Board, u32, usize, Option<Piece>)> = Vec::new();
    let mut locs = Vec::new();
    for (piece, new_hold, next) in first_solver.options(0, hold) {
        drops(board, piece, rows, spawn, &mut locs);
        for loc in &locs {
            let mut nb = *board;
            nb.place(loc);
            let cleared = nb.clear_full_lines();
            if fits(&nb, rows - cleared, soft) {
                firsts.push((Action { loc: *loc }, nb, rows - cleared, next, new_hold));
            }
        }
    }
    if firsts.is_empty() {
        return None;
    }
    let mut wins = vec![0u32; firsts.len()];
    let mut done = 0u32;
    let total = conts.len() as u32;
    for cont in &conts {
        let mut seq = queue.to_vec();
        seq.extend_from_slice(cont);
        let mut solver = Solver { queue: &seq, failed: KeySet::default(), deadline, aborted: false, buf: Vec::new(), nodes: 0, spawn };
        let mut results = vec![false; firsts.len()];
        let best_now = wins.iter().copied().max().unwrap_or(0);
        for (i, (_, nb, nrows, next, nh)) in firsts.iter().enumerate() {
            // 残りを全部取っても今の最善に届かない手は調べない
            if wins[i] + (total - done) < best_now {
                continue;
            }
            results[i] = solver.solvable(*nb, *nrows, *next, *nh, 0);
            if solver.aborted {
                break;
            }
        }
        if solver.aborted {
            break;
        }
        done += 1;
        for (w, r) in wins.iter_mut().zip(&results) {
            *w += *r as u32;
        }
    }
    if std::env::var("PCO_DEBUG").is_ok() {
        eprintln!("pco: 段{rows} 必要{need} 見えている{} 候補{} 続き{}/{} 最多{:?}", queue.len(), firsts.len(), done, conts.len(), wins.iter().max());
    }
    if done == 0 {
        return None;
    }
    let (i, &best) = wins.iter().enumerate().max_by_key(|&(i, &w)| (w, std::cmp::Reverse(i)))?;
    if best == 0 {
        return None;
    }
    Some((firsts[i].0, best as f32 / done as f32))
}

/// `action` の位置に入る手(ハードドロップだけで入る、操作の一番少ないもの)。
pub fn to_move(board: &Board, action: &Action, rules: &Rules) -> Option<Move> {
    let want = action.loc.sorted_cells();
    generate(board, action.loc.piece, rules.spawn())
        .into_iter()
        .filter(|m| m.placement.location.piece == action.loc.piece && m.placement.location.sorted_cells() == want)
        .min_by_key(|m| (m.soft_drop_sections, m.mid_air_stops, m.inputs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use Piece::*;

    #[test]
    fn 形の判定() {
        assert!(ok(&Board::new(), 4));
        // 左4列が4段埋まり、残り24マス(6ミノ)
        assert!(ok(&Board::from_rows(&["####......"; 4]), 4));
        // 穴があると取れない
        assert!(!ok(&Board::from_rows(&["#.........", ".........."]), 4));
        // 埋まった列で区切られた区画が4の倍数でない
        assert!(!ok(&Board::from_rows(&[".#########", ".#########"]), 2));
        assert!(ok(&Board::from_rows(&["..########", "..########"]), 2));
    }

    #[test]
    fn 下が空いたマスは横から差し込める時だけ許す() {
        // 左下の空きは右へたどると上に何も無い列に出られる
        let tuck = Board::from_rows(&["####......", ".........."]);
        assert!(!ok(&tuck, 4));
        assert!(fits(&tuck, 4, true));
        assert!(!sealed(&tuck));
        // 左右が埋まっていて出られない空きは本当の穴
        let hole = Board::from_rows(&["###.......", "#.#......."]);
        assert!(sealed(&hole));
        assert!(!fits(&hole, 4, true));
        // ハードドロップだけの時は今までどおり
        assert!(!fits(&tuck, 4, false));
    }

    #[test]
    fn 開幕の袋でパフェの手が見つかる() {
        // よく知られた PCO が組める1巡目(I L J O S Z T)と、2巡目の頭
        let queue = [I, L, O, J, S, Z];
        let bag = [T];
        let r = best_action(&Board::new(), 4, None, &queue, &bag, Duration::from_secs(5), None);
        let (a, p) = r.expect("見込みがあるはず");
        assert!(p > 0.3, "見込みが低すぎる: {p} {a:?}");
        let rules = Rules::ppt2();
        assert!(to_move(&Board::new(), &a, &rules).is_some());
    }

    #[test]
    fn 全部見えていれば必ず取れるか分かる() {
        // 2段のパフェ: 1段目に I を2本と残り2マス、2段目… 盤面をあらかじめ作る
        // 左8列が2段埋まり、右2列が空き(4マス)→ O で取れる
        let board = Board::from_rows(&["########..", "########.."]);
        let (a, p) = best_action(&board, 2, None, &[O, T], &[], Duration::from_secs(1), None).expect("O で取れる");
        assert_eq!(a.loc.piece, O);
        assert_eq!(p, 1.0);
        // T しか無ければ取れない
        assert!(best_action(&board, 2, None, &[T, T], &[], Duration::from_secs(1), None).is_none());
    }
}
