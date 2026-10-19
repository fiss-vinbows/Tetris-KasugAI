//! 手の生成: 出現位置から SRS で届き、固定できるすべての配置を列挙する。
//!
//! 操作は「左右移動・左右回転・1マスずつのソフトドロップ・ハードドロップ」。
//! 配置ごとに、そこへ入れるための**一番やさしい操作**の難しさも求める。
//!
//! - やさしさは (ソフトドロップを途中で止める回数, ソフトドロップの区間数, 移動・回転の回数)
//!   の辞書順で比べる。
//!   - 途中で止める: ソフトドロップを接地する前にやめて、空中で移動・回転すること。
//!     タイミングが難しいので一番重く数える。Cold Clear 2 はソフトドロップを常に底まで落とす前提で
//!     手を作るので、この回数が 0 の手が Cold Clear 2 の手の集合と一致する(tools/crosscheck-cc2)。
//!   - 区間数は TBA の難度表示(◎=0回・○=1回・△=2回以上)と同じ数え方で、
//!     「↓×16」は1回、「↓→回転→↓」は2回と数える(TBA の srs_reach.find_path_min_soft と同じ)。
//! - Tスピンかどうかは「最後の操作が回転で、そのまま固定した」場合だけ付く。
//!   同じ4マスでも、Tスピンで入れる配置とそうでない配置は別の手として返す(攻撃量が違うため)。
//!
//! 速さのための工夫(結果は単純な探索と同じ。tools/ の突き合わせで確認している):
//! - 衝突マップ: 向き・列ごとに「中心をこの高さに置くとぶつかる」をビット列で前計算する
//! - 空中の省略: 積みより2段以上上は何もない空間なので、出現位置の高さで移動・回転してから
//!   下ろしたのと同じになる。そこは1マスずつ探索せず、「出現位置の高さでの操作 + ソフトドロップ」
//!   として積みの少し上から探索を始める
//! - 費用は小さい整数なので、優先度付きキューの代わりにバケット(費用ごとの配列)を使う

use crate::board::{Board, HEIGHT, WIDTH};
use crate::piece::{Piece, PieceLocation, Rotation};
use crate::srs::{kicks, t_spin, Spin};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;

/// 固定する配置(TBP の `location` + `spin`)。`location` は代表形(canonical_form)。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Placement {
    pub location: PieceLocation,
    pub spin: Spin,
}

/// 配置と、そこへ入れる一番やさしい操作の難しさ。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Move {
    pub placement: Placement,
    /// ソフトドロップを接地前に止めて、空中で移動・回転する回数。
    pub mid_air_stops: u8,
    /// ソフトドロップの区間数(0 ならハードドロップだけで入る)。
    pub soft_drop_sections: u8,
    /// 左右移動・回転の回数(区間数が同じ中で最少のもの)。
    pub inputs: u8,
}

/// ミノの出現位置(中心)。ぷよテト2 は x=4, y=20(可視20行の1つ上)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spawn {
    pub x: i8,
    pub y: i8,
}

impl Default for Spawn {
    fn default() -> Self {
        Spawn { x: 4, y: 20 }
    }
}

// 探索の状態: 中心 (x, y) は必ず盤面内(どのミノも中心が自分のマスの1つなので)。
const XS: usize = WIDTH;
const YS: usize = HEIGHT;
const LOCS: usize = 4 * XS * YS;
const SPINS: usize = 3;
const STATES: usize = LOCS * SPINS * 2;

#[inline]
fn loc_index(r: usize, x: usize, y: usize) -> usize {
    (r * XS + x) * YS + y
}

#[inline]
fn spin_index(spin: Spin) -> usize {
    match spin {
        Spin::None => 0,
        Spin::Mini => 1,
        Spin::Full => 2,
    }
}

const SPIN_OF: [Spin; 3] = [Spin::None, Spin::Mini, Spin::Full];

/// 状態 = 位置 × Tスピン状態 × 直前の操作がソフトドロップか。
#[inline]
fn state_index(loc: usize, spin: usize, soft: bool) -> usize {
    (loc * SPINS + spin) * 2 + soft as usize
}

/// 費用は (止める回数 << 16 | 区間数 << 8 | 操作回数) で1つの数にまとめる(辞書順の比較になる)。
#[inline]
const fn cost(stops: u32, sections: u32, inputs: u32) -> u32 {
    stops << 16 | sections << 8 | inputs
}

/// 費用からバケットの番号を作る(大きすぎる値は丸める。実際の盤面ではまず起きない)。
#[inline]
fn bucket_of(c: u32) -> usize {
    let stops = (c >> 16).min(7) as usize;
    let sections = (c >> 8 & 0xff).min(15) as usize;
    let inputs = (c & 0xff).min(63) as usize;
    (stops * 16 + sections) * 64 + inputs
}
const BUCKETS: usize = 8 * 16 * 64;

/// 向き・列ごとの衝突マップ。`maps[r][x]` の y ビット目が立っていれば、中心 (x, y) に置けない。
struct CollisionMap {
    maps: [[u64; XS]; 4],
}

impl CollisionMap {
    fn new(board: &Board, piece: Piece) -> Self {
        // 40行より上は埋まり扱い
        let ceiling = !0u64 << HEIGHT;
        let mut maps = [[0u64; XS]; 4];
        for r in Rotation::ALL {
            for (dx, dy) in piece.cells().map(|c| r.rotate_cell(c)) {
                for x in 0..XS as i8 {
                    let cx = x + dx;
                    let col = if (0..XS as i8).contains(&cx) { board.cols[cx as usize] | ceiling } else { !0 };
                    let shifted = if dy >= 0 { col >> dy } else { col << -dy | ((1u64 << -dy) - 1) };
                    maps[r.index()][x as usize] |= shifted;
                }
            }
        }
        CollisionMap { maps }
    }

    #[inline]
    fn obstructed(&self, r: usize, x: i8, y: i8) -> bool {
        if !(0..XS as i8).contains(&x) || !(0..YS as i8).contains(&y) {
            return true;
        }
        self.maps[r][x as usize] >> y & 1 != 0
    }

    /// (r, x, y) から真下へ落ちられるマス数。
    #[inline]
    fn drop_distance(&self, r: usize, x: usize, y: usize) -> usize {
        let below = self.maps[r][x] & ((1u64 << y) - 1);
        if below == 0 {
            y
        } else {
            y - (64 - below.leading_zeros() as usize)
        }
    }

    /// 回転を試す。成功すれば (向き, x, y, 何番目のずらしか)。
    #[inline]
    fn rotate(&self, piece: Piece, r: usize, x: i8, y: i8, clockwise: bool) -> Option<(usize, i8, i8, u8)> {
        if piece == Piece::O {
            return None;
        }
        let from = Rotation::ALL[r];
        let to = if clockwise { from.cw() } else { from.ccw() };
        for (i, (dx, dy)) in kicks(piece, from, to).into_iter().enumerate() {
            if !self.obstructed(to.index(), x + dx, y + dy) {
                return Some((to.index(), x + dx, y + dy, i as u8));
            }
        }
        None
    }
}

/// 呼び出しごとに確保し直さないための作業領域(スレッドごとに1つ)。
struct Scratch {
    best: Vec<u32>,
    touched: Vec<u32>,
    locks: Vec<u32>,
    touched_locks: Vec<u32>,
    buckets: Vec<Vec<u32>>,
}

impl Scratch {
    fn new() -> Self {
        Scratch {
            best: vec![u32::MAX; STATES],
            touched: Vec::new(),
            locks: vec![u32::MAX; LOCS * SPINS],
            touched_locks: Vec::new(),
            buckets: vec![Vec::new(); BUCKETS],
        }
    }
}

thread_local! {
    static SCRATCH: RefCell<Scratch> = RefCell::new(Scratch::new());
}

/// `board` に `piece` を置ける配置をすべて返す。出現位置がふさがっていれば空(負け)。
pub fn generate(board: &Board, piece: Piece, spawn: Spawn) -> Vec<Move> {
    let mut out = Vec::new();
    generate_into(board, piece, spawn, &mut out);
    out
}

/// `generate` と同じ。結果を `out` に入れる(呼び出し側の配列を使い回せる)。
pub fn generate_into(board: &Board, piece: Piece, spawn: Spawn, out: &mut Vec<Move>) {
    out.clear();
    SCRATCH.with(|s| search(board, piece, spawn, &mut s.borrow_mut(), out));
}

fn search(board: &Board, piece: Piece, spawn: Spawn, s: &mut Scratch, out: &mut Vec<Move>) {
    let map = CollisionMap::new(board, piece);
    if map.obstructed(0, spawn.x, spawn.y) {
        return;
    }
    let Scratch { best, touched, locks, touched_locks, buckets } = s;
    let max_bucket = std::cell::Cell::new(0usize);

    let mut push = |best: &mut Vec<u32>, buckets: &mut Vec<Vec<u32>>, state: usize, c: u32| {
        if c < best[state] {
            if best[state] == u32::MAX {
                touched.push(state as u32);
            }
            best[state] = c;
            let b = bucket_of(c);
            buckets[b].push(state as u32);
            max_bucket.set(max_bucket.get().max(b));
        }
    };
    let mut record_lock = |locks: &mut Vec<u32>, r: usize, x: usize, y: usize, spin: Spin, c: u32| {
        let landed = PieceLocation::new(piece, Rotation::ALL[r], x as i8, y as i8).canonical_form();
        let key = loc_index(landed.rotation.index(), landed.x as usize, landed.y as usize) * SPINS + spin_index(spin);
        if c < locks[key] {
            if locks[key] == u32::MAX {
                touched_locks.push(key as u32);
            }
            locks[key] = c;
        }
    };

    // 積みより2段以上上(どの向きのマスも積みに届かない高さ)は何もない空間
    let entry_y = board.max_height() as i8 + 2;
    if entry_y < spawn.y {
        // 出現位置の高さで、移動・回転だけで行ける位置と操作回数(幅優先)
        let mut top = [u32::MAX; 4 * XS];
        let mut queue = Vec::with_capacity(4 * XS);
        queue.push((0usize, spawn.x));
        top[spawn.x as usize] = 0;
        let mut i = 0;
        while i < queue.len() {
            let (r, x) = queue[i];
            i += 1;
            let c = top[r * XS + x as usize];
            let mut next = [(0usize, 0i8); 4];
            let mut n = 0;
            for dx in [-1, 1] {
                if !map.obstructed(r, x + dx, spawn.y) {
                    next[n] = (r, x + dx);
                    n += 1;
                }
            }
            for clockwise in [false, true] {
                // 空中では縦にずれない(横のずらしで必ず入る)。縦にずれたら使わない
                if let Some((r2, x2, y2, _)) = map.rotate(piece, r, x, spawn.y, clockwise) {
                    if y2 == spawn.y {
                        next[n] = (r2, x2);
                        n += 1;
                    }
                }
            }
            for &(r2, x2) in &next[..n] {
                let k = r2 * XS + x2 as usize;
                if top[k] == u32::MAX {
                    top[k] = c + 1;
                    queue.push((r2, x2));
                }
            }
        }
        for r in 0..4 {
            for x in 0..XS {
                let inputs = top[r * XS + x];
                if inputs == u32::MAX {
                    continue;
                }
                // そのままハードドロップ
                let dist = map.drop_distance(r, x, spawn.y as usize);
                record_lock(locks, r, x, spawn.y as usize - dist, Spin::None, cost(0, 0, inputs));
                // ソフトドロップで積みの少し上まで下ろした状態から探索を続ける
                let y = entry_y as usize;
                debug_assert!(!map.obstructed(r, x as i8, entry_y));
                push(best, buckets, state_index(loc_index(r, x, y), 0, true), cost(0, 1, inputs));
            }
        }
    } else {
        push(best, buckets, state_index(loc_index(0, spawn.x as usize, spawn.y as usize), 0, false), 0);
    }

    let mut b = 0;
    while b <= max_bucket.get() {
        let mut i = 0;
        while i < buckets[b].len() {
            let state = buckets[b][i] as usize;
            i += 1;
            let c = best[state];
            if bucket_of(c) != b {
                continue; // もっとやさしい操作で到達済み
            }
            let soft = state % 2 == 1;
            let spin = SPIN_OF[state / 2 % SPINS];
            let li = state / 2 / SPINS;
            let (y, x, r) = (li % YS, li / YS % XS, li / YS / XS);
            let (stops, sections, inputs) = (c >> 16, c >> 8 & 0xff, c & 0xff);

            // ハードドロップで固定(費用はかからない)
            let dist = map.drop_distance(r, x, y);
            record_lock(locks, r, x, y - dist, if dist == 0 { spin } else { Spin::None }, c);

            // ソフトドロップの途中(接地前)で移動・回転するなら、止める回数が1つ増える
            let stops_after = if soft && dist > 0 { stops + 1 } else { stops };
            let (xi, yi) = (x as i8, y as i8);

            // 左右移動
            for dx in [-1i8, 1] {
                if !map.obstructed(r, xi + dx, yi) {
                    let next = state_index(loc_index(r, (xi + dx) as usize, y), 0, false);
                    push(best, buckets, next, cost(stops_after, sections, inputs + 1));
                }
            }
            // 左右回転
            for clockwise in [false, true] {
                if let Some((r2, x2, y2, kick)) = map.rotate(piece, r, xi, yi, clockwise) {
                    // 空中ではTスピン状態を持たない(落ちれば消えるため。状態数を減らす)
                    let grounded = map.drop_distance(r2, x2 as usize, y2 as usize) == 0;
                    let sp = if piece == Piece::T && grounded {
                        let loc = PieceLocation::new(piece, Rotation::ALL[r2], x2, y2);
                        spin_index(t_spin(board, &loc, kick))
                    } else {
                        0
                    };
                    let next = state_index(loc_index(r2, x2 as usize, y2 as usize), sp, false);
                    push(best, buckets, next, cost(stops_after, sections, inputs + 1));
                }
            }
            // 1マスのソフトドロップ
            if dist > 0 {
                let next = state_index(loc_index(r, x, y - 1), 0, true);
                let sec = if soft { sections } else { sections + 1 };
                push(best, buckets, next, cost(stops, sec, inputs));
            }
        }
        buckets[b].clear();
        b += 1;
    }

    // 結果をまとめ、作業領域を元に戻す
    touched_locks.sort_unstable();
    for &key in touched_locks.iter() {
        let key = key as usize;
        let c = locks[key];
        let li = key / SPINS;
        let (y, x, r) = (li % YS, li / YS % XS, li / YS / XS);
        out.push(Move {
            placement: Placement {
                location: PieceLocation::new(piece, Rotation::ALL[r], x as i8, y as i8),
                spin: SPIN_OF[key % SPINS],
            },
            mid_air_stops: (c >> 16) as u8,
            soft_drop_sections: (c >> 8 & 0xff) as u8,
            inputs: (c & 0xff) as u8,
        });
        locks[key] = u32::MAX;
    }
    touched_locks.clear();
    for &st in touched.iter() {
        best[st as usize] = u32::MAX;
    }
    touched.clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn count_cells(moves: &[Move]) -> usize {
        let mut cells: Vec<_> = moves.iter().map(|m| m.placement.location.sorted_cells()).collect();
        cells.sort();
        cells.dedup();
        cells.len()
    }

    #[test]
    fn 空の盤面での配置数() {
        // 空の盤面で置ける位置の数はよく知られている
        let expected = [(Piece::I, 17), (Piece::O, 9), (Piece::T, 34), (Piece::L, 34), (Piece::J, 34), (Piece::S, 17), (Piece::Z, 17)];
        for (piece, n) in expected {
            let moves = generate(&Board::new(), piece, Spawn::default());
            assert_eq!(count_cells(&moves), n, "{piece:?}");
            assert_eq!(moves.len(), n, "{piece:?}: 空の盤面ではTスピンは無く、同じ4マスは1手");
            assert!(moves.iter().all(|m| m.soft_drop_sections == 0), "{piece:?}: すべてハードドロップで入る");
        }
    }

    #[test]
    fn 出現位置がふさがっていれば手はない() {
        let mut board = Board::new();
        board.cols[4] = 1 << 20;
        assert!(generate(&board, Piece::T, Spawn::default()).is_empty());
    }

    #[test]
    fn 張り出しの下への潜り込みはソフトドロップ1回() {
        let board = Board::from_rows(&[
            "####......",
            "..........",
            "..........",
        ]);
        let moves = generate(&board, Piece::O, Spawn::default());
        let under = moves
            .iter()
            .find(|m| m.placement.location.sorted_cells() == [(0, 0), (1, 0), (0, 1), (1, 1)])
            .expect("張り出しの下に入る");
        assert_eq!(under.soft_drop_sections, 1);
    }

    #[test]
    fn 空中の棚へはソフトドロップを途中で止めて入れる() {
        // 左の棚(1〜2段目の左2列)の上、屋根の下へOを入れる。右側は床まで空いているので、
        // 3段目の高さでソフトドロップを止めて左へ動かす必要がある
        let board = Board::from_rows(&[
            "####......",
            "..........",
            "..........",
            "##........",
            "##........",
        ]);
        let moves = generate(&board, Piece::O, Spawn::default());
        let shelf = moves
            .iter()
            .find(|m| m.placement.location.sorted_cells() == [(0, 2), (1, 2), (0, 3), (1, 3)])
            .expect("棚に入る");
        assert_eq!((shelf.mid_air_stops, shelf.soft_drop_sections), (1, 1));
        // 床に置くだけなら止める必要はない
        let floor = moves
            .iter()
            .find(|m| m.placement.location.sorted_cells() == [(2, 0), (3, 0), (2, 1), (3, 1)])
            .expect("床に入る");
        assert_eq!(floor.mid_air_stops, 0);
    }

    /// TSD の入れ方: ソフトドロップで下ろしてから回転で溝に入れる。
    #[test]
    fn tsdは回転で入れるとtスピン() {
        let board = Board::from_rows(&[
            "##........",
            "#...######",
            "##.#######",
        ]);
        let moves = generate(&board, Piece::T, Spawn::default());
        let target = [(2, 0), (1, 1), (2, 1), (3, 1)];
        let tsd: Vec<_> = moves.iter().filter(|m| m.placement.location.sorted_cells() == target).collect();
        assert!(tsd.iter().any(|m| m.placement.spin == Spin::Full), "{tsd:?}");
    }
}
