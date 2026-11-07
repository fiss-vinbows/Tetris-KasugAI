//! 対戦のルール: 1人分の状態(盤面・ミノ・HOLD・攻撃ゲージ・予告)と、1手置いたときの処理。
//!
//! おじゃまの流れ(ぷよテト2。細かい値は config/rules/ppt2.toml の [garbage]):
//! 1. ラインを消すと攻撃が生まれる。まず自分への予告を古いものから打ち消す(相殺)
//! 2. 残った攻撃は攻撃ゲージにためる。REN が途切れた(ラインを消さない手を置いた)時点で相手へ送る
//! 3. 送られた攻撃は相手の予告に入り、`delay_pieces` 手待ってから、相手がラインを消さない手を
//!    置いたときにせり上がる(1手あたり `max_rise_per_piece` 行まで)
//!
//! 置く手は bot の提案をそのまま信じず、こちらの手の生成で届くか確かめる(届かなければ反則負け)。
//! Tスピンかどうかもこちらの判定を使う。

use std::collections::VecDeque;

use rand::rngs::StdRng;
use rand::seq::SliceRandom;
use rand::{Rng, SeedableRng};
use tetrisai_core::board::WIDTH;
use tetrisai_core::{generate, Piece, Placement, PlayerState, Rules, Spin};

/// NEXT の数(ぷよテト2 は5個)。
pub const NEXT_COUNT: usize = 5;
/// 置いたミノが全部この高さ以上なら負け(可視20行より上で固定した)。
const LOCK_OUT_Y: i8 = 20;

/// 7種1巡(袋)のミノ列。両者に同じ列を配る。
pub struct PieceSequence {
    rng: StdRng,
    pieces: Vec<Piece>,
}

impl PieceSequence {
    pub fn new(seed: u64) -> Self {
        PieceSequence { rng: StdRng::seed_from_u64(seed), pieces: Vec::new() }
    }

    pub fn get(&mut self, i: usize) -> Piece {
        while self.pieces.len() <= i {
            let mut bag = Piece::ALL;
            bag.shuffle(&mut self.rng);
            self.pieces.extend(bag);
        }
        self.pieces[i]
    }
}

/// 予告の1件(相手の1回の攻撃)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Incoming {
    pub lines: u32,
    pub hole: usize,
    /// あと何手でせり上がれるようになるか。
    pub delay: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Loss {
    /// 置ける手が無い(出現位置がふさがった)
    NoMove,
    /// 置ける手があるのに bot が手を返さなかった
    BotGaveUp,
    /// bot の手がこちらの手の生成で届かない
    IllegalMove,
    /// 可視範囲より上で固定した
    LockOut,
    /// おじゃまで上端からあふれた
    GarbageOverflow,
}

/// 1人分の成績。
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub pieces: u32,
    /// 生まれた攻撃の合計(相殺に使った分を含む)
    pub attack: u32,
    /// 相殺に使った行数
    pub cancelled: u32,
    /// 実際に相手へ送った行数
    pub sent: u32,
    /// せり上がったおじゃまの行数
    pub received: u32,
    pub lines: u32,
    pub tetrises: u32,
    pub tspin_singles: u32,
    pub tspin_doubles: u32,
    pub tspin_triples: u32,
    pub tspin_minis: u32,
    pub perfect_clears: u32,
    pub back_to_backs: u32,
    pub max_ren: u32,
    /// ソフトドロップが必要だった手の数
    pub soft_drop_moves: u32,
    /// TSD のうち、予告があった時の数 / 予告が無く盤面が8以下だった時の数(ため込みの確認用)
    pub tsd_with_incoming: u32,
    pub tsd_low_quiet: u32,
    /// そのうち Tスピン(TSS・TSD・TST)でない手の数(2026-11-03)
    pub soft_drop_non_spin: u32,
    /// ラインを消した手の数(実機ではライン消去の演出 約0.7秒を待つ。2026-11-01 全自動の速さの比較用)
    pub clear_moves: u32,
    /// B2B が続いていたのに、普通の消去で切った回数(2026-11-05)
    pub b2b_breaks: u32,
    /// そのうち REN の途中(前の手でも消していた)で切った回数
    pub b2b_breaks_in_ren: u32,
    /// そのうち1列消しで切った回数
    pub b2b_breaks_single: u32,
    /// 壁の上を通れず、別の場所に落ちた手の数(`Player::block_margin`。2026-11-07)
    pub blocked: u32,
}

/// 1手置いた結果。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    /// 相手へ送る攻撃(この手で REN が途切れてゲージを放った分)
    pub send: u32,
    /// おじゃまがせり上がったか(bot に局面を渡し直す必要がある)
    pub garbage_rose: bool,
    /// HOLD を使ったか
    pub used_hold: bool,
    /// この手で NEXT に新しく見えたミノの数
    pub revealed: usize,
    /// 予定の場所へ届かず、別の場所に落ちたか(bot に局面を渡し直す必要がある)
    pub blocked: bool,
    /// この手にかかる実機の時間(ms)の目安(`GameConfig::timed` で使う)
    pub move_ms: u32,
}

pub struct Player {
    pub state: PlayerState,
    pub hold: Option<Piece>,
    /// `queue[0]` が操作ミノ、続く `NEXT_COUNT` 個が NEXT。
    pub queue: VecDeque<Piece>,
    /// 次にミノ列から取り出す位置
    next_index: usize,
    /// 攻撃ゲージ(REN 中にためている攻撃)
    pub gauge: u32,
    pub incoming: VecDeque<Incoming>,
    hole_rng: StdRng,
    pub stats: Stats,
    pub loss: Option<Loss>,
    /// 実機の「壁に止められる」を再現する: 出現位置よりこの行数だけ低い位置から届かない手は、届く範囲の一番近い列に落ちる。
    /// 0 なら再現しない(2026-11-07)
    pub block_margin: u32,
}

impl Player {
    /// `hole_seed` はおじゃまの穴の位置を決める乱数の種(両者に同じ種を使うと条件がそろう)。
    pub fn new(sequence: &mut PieceSequence, hole_seed: u64) -> Self {
        let mut p = Player {
            state: PlayerState::default(),
            hold: None,
            queue: VecDeque::new(),
            next_index: 0,
            gauge: 0,
            incoming: VecDeque::new(),
            hole_rng: StdRng::seed_from_u64(hole_seed),
            stats: Stats::default(),
            loss: None,
            block_margin: 0,
        };
        p.refill(sequence);
        p
    }

    fn refill(&mut self, sequence: &mut PieceSequence) -> usize {
        let mut added = 0;
        while self.queue.len() < 1 + NEXT_COUNT {
            self.queue.push_back(sequence.get(self.next_index));
            self.next_index += 1;
            added += 1;
        }
        added
    }

    /// 今使えるミノ(操作ミノ・HOLD・HOLD が空なら NEXT の1つ目)のどれかで置ける手があるか。
    pub fn has_any_move(&self, rules: &Rules) -> bool {
        let usable = [self.queue.front().copied(), self.hold.or_else(|| self.queue.get(1).copied())];
        usable.into_iter().flatten().any(|piece| {
            generate(&self.state.board, piece, rules.spawn())
                .iter()
                .any(|m| m.placement.location.cells().iter().any(|&(_, y)| y < LOCK_OUT_Y))
        })
    }

    /// NEXT の最後のミノの後で、今の袋(7種1巡)に残っているミノ。TBP の `randomizer.bag_state`。
    pub fn bag_state(&self, sequence: &mut PieceSequence) -> Vec<Piece> {
        let start = self.next_index;
        let end = start.div_ceil(7).max(1) * 7;
        let end = if start.is_multiple_of(7) { start + 7 } else { end };
        (start..end).map(|i| sequence.get(i)).collect()
    }

    /// 予告の合計行数。
    pub fn incoming_lines(&self) -> u32 {
        self.incoming.iter().map(|g| g.lines).sum()
    }

    /// 相手の攻撃を予告に加える。
    pub fn receive(&mut self, lines: u32, rules: &Rules) {
        if lines == 0 {
            return;
        }
        let hole = self.hole_rng.gen_range(0..WIDTH);
        self.incoming.push_back(Incoming { lines, hole, delay: rules.garbage.delay_pieces });
    }

    /// bot が提案した手を置く。負けたら `self.loss` を設定して Err を返す。
    pub fn place(&mut self, suggested: &Placement, rules: &Rules, sequence: &mut PieceSequence) -> Result<Outcome, Loss> {
        let result = self.place_inner(suggested, rules, sequence);
        if let Err(loss) = result {
            self.loss = Some(loss);
        }
        result
    }

    fn place_inner(&mut self, suggested: &Placement, rules: &Rules, sequence: &mut PieceSequence) -> Result<Outcome, Loss> {
        let piece = suggested.location.piece;
        let current = *self.queue.front().ok_or(Loss::NoMove)?;

        // 使うミノ(操作ミノ / HOLD / HOLD が空なら NEXT の1つ目)
        let used_hold = piece != current;
        if used_hold {
            match self.hold {
                Some(h) if h == piece => {}
                None if self.queue.get(1) == Some(&piece) => {}
                _ => return Err(Loss::IllegalMove),
            }
        }

        // こちらの手の生成で届くか確かめ、Tスピンの判定もこちらのものを使う
        let cells = suggested.location.sorted_cells();
        let moves = generate(&self.state.board, piece, rules.spawn());
        let mut matching = moves.iter().filter(|m| m.placement.location.sorted_cells() == cells);
        let first = matching.next().copied().ok_or(Loss::IllegalMove)?;
        let mut mv = std::iter::once(first)
            .chain(matching.copied())
            .find(|m| m.placement.spin == suggested.spin)
            .unwrap_or(first);
        let mut blocked = false;
        if self.block_margin > 0 {
            let spawn = rules.spawn();
            let low = generate(&self.state.board, piece, tetrisai_core::Spawn { x: spawn.x, y: spawn.y - self.block_margin as i8 });
            if !low.is_empty() && !low.iter().any(|m| m.placement.location.sorted_cells() == cells) {
                // 同じ向きのまま、一番近い列へハードドロップしたことにする
                let target = suggested.location;
                let near = low
                    .iter()
                    .filter(|m| m.soft_drop_sections == 0)
                    .min_by_key(|m| {
                        let l = m.placement.location;
                        ((l.x - target.x).abs() as i32) * 2 + (l.rotation != target.rotation) as i32 * 100
                    })
                    .copied();
                if let Some(near) = near {
                    mv = near;
                    blocked = true;
                    self.stats.blocked += 1;
                }
            }
        }
        if mv.placement.location.cells().iter().all(|&(_, y)| y >= LOCK_OUT_Y) {
            return Err(Loss::LockOut);
        }

        // ミノ列を進める
        if used_hold {
            if self.hold.is_none() {
                self.hold = self.queue.pop_front();
                self.queue.pop_front();
            } else {
                self.hold = self.queue.pop_front();
            }
        } else {
            self.queue.pop_front();
        }
        let revealed = self.refill(sequence);

        let height_before = self.state.board.max_height();
        let incoming_before = self.incoming_lines();
        let had_b2b = self.state.back_to_back;
        let result = self.state.apply(&mv.placement, rules);
        let s = &mut self.stats;
        if had_b2b && result.lines > 0 && !self.state.back_to_back {
            s.b2b_breaks += 1;
            s.b2b_breaks_in_ren += (result.ren > 0) as u32;
            s.b2b_breaks_single += (result.lines == 1) as u32;
        }
        s.pieces += 1;
        s.attack += result.attack;
        s.lines += result.lines;
        s.max_ren = s.max_ren.max(result.ren);
        s.back_to_backs += result.back_to_back as u32;
        s.perfect_clears += result.perfect_clear as u32;
        s.soft_drop_moves += (mv.soft_drop_sections > 0) as u32;
        s.soft_drop_non_spin += (mv.soft_drop_sections > 0 && result.spin != Spin::Full) as u32;
        s.clear_moves += (result.lines > 0) as u32;
        match (result.spin, result.lines) {
            (Spin::None, 4) => s.tetrises += 1,
            (Spin::Full, 1) => s.tspin_singles += 1,
            (Spin::Full, 2) => {
                s.tspin_doubles += 1;
                s.tsd_with_incoming += (incoming_before > 0) as u32;
                s.tsd_low_quiet += (incoming_before == 0 && height_before <= 8) as u32;
            }
            (Spin::Full, 3) => s.tspin_triples += 1,
            (Spin::Mini, 1..) => s.tspin_minis += 1,
            _ => {}
        }

        // 【2026-11-07・実機 debug_log_20261107_150112(約2,000手)の中央値に当てはめた】ハードドロップの手 370ms、
        // 消すと演出で +650ms、ソフトドロップは接地の待ちなどで +300ms と1段 25ms
        let drop_rows = (rules.spawn().y - mv.placement.location.y).max(0) as u32;
        let move_ms = 370
            + if result.lines > 0 { 650 } else { 0 }
            + if mv.soft_drop_sections > 0 { 300 + 25 * drop_rows } else { 0 };
        let mut outcome = Outcome { used_hold, revealed, blocked, move_ms, ..Default::default() };
        if result.lines > 0 {
            let mut attack = result.attack;
            if rules.garbage.cancel_incoming_first {
                while attack > 0 {
                    let Some(front) = self.incoming.front_mut() else { break };
                    let c = attack.min(front.lines);
                    front.lines -= c;
                    attack -= c;
                    self.stats.cancelled += c;
                    if front.lines == 0 {
                        self.incoming.pop_front();
                    }
                }
            }
            self.gauge += attack;
            if !rules.garbage.send_after_ren_ends {
                outcome.send = std::mem::take(&mut self.gauge);
            }
        } else {
            // REN が途切れたのでゲージを放ち、届いている予告をせり上げる
            outcome.send = std::mem::take(&mut self.gauge);
            outcome.garbage_rose = self.rise(rules)?;
        }
        for g in self.incoming.iter_mut() {
            g.delay = g.delay.saturating_sub(1);
        }
        self.stats.sent += outcome.send;
        Ok(outcome)
    }

    /// 待ち時間を過ぎた予告をせり上げる。せり上がったら true。
    fn rise(&mut self, rules: &Rules) -> Result<bool, Loss> {
        let mut budget = match rules.garbage.max_rise_per_piece {
            0 => u32::MAX,
            n => n,
        };
        let mut rose = false;
        while budget > 0 {
            let Some(front) = self.incoming.front_mut() else { break };
            if front.delay > 0 {
                break;
            }
            let n = front.lines.min(budget);
            let overflow = if rules.garbage.hole_changes_within_attack {
                let mut overflow = false;
                for _ in 0..n {
                    let hole = self.hole_rng.gen_range(0..WIDTH);
                    overflow |= self.state.board.add_garbage(1, hole);
                }
                overflow
            } else {
                self.state.board.add_garbage(n, front.hole)
            };
            front.lines -= n;
            budget -= n;
            self.stats.received += n;
            rose = true;
            if front.lines == 0 {
                self.incoming.pop_front();
            }
            if overflow {
                return Err(Loss::GarbageOverflow);
            }
        }
        Ok(rose)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tetrisai_core::{Board, PieceLocation, Rotation};

    fn rules() -> Rules {
        Rules::ppt2()
    }

    /// テスト用: ミノ列を固定した Player(操作ミノ + NEXT を指定)。
    fn player(board: Board, pieces: &[Piece]) -> (Player, PieceSequence) {
        let mut seq = PieceSequence::new(1);
        seq.pieces = pieces.to_vec();
        seq.pieces.extend(std::iter::repeat_n(Piece::O, 50));
        let mut p = Player::new(&mut seq, 7);
        p.state.board = board;
        (p, seq)
    }

    fn at(piece: Piece, rotation: Rotation, x: i8, y: i8) -> Placement {
        Placement { location: PieceLocation::new(piece, rotation, x, y), spin: Spin::None }
    }

    #[test]
    fn 同じ種なら同じミノ列() {
        let mut a = PieceSequence::new(5);
        let mut b = PieceSequence::new(5);
        let xs: Vec<_> = (0..21).map(|i| a.get(i)).collect();
        let ys: Vec<_> = (0..21).map(|i| b.get(i)).collect();
        assert_eq!(xs, ys);
        // 7個ごとに7種がそろう
        let mut first: Vec<_> = xs[..7].to_vec();
        first.sort();
        assert_eq!(first, Piece::ALL.to_vec());
    }

    #[test]
    fn 袋の残り() {
        let mut seq = PieceSequence::new(3);
        let mut p = Player::new(&mut seq, 1);
        // 最初は6個(操作ミノ + NEXT5)を配ったので、1つ目の袋の残りは1個
        assert_eq!(p.bag_state(&mut seq), vec![seq.get(6)]);
        p.next_index = 7;
        assert_eq!(p.bag_state(&mut seq).len(), 7, "袋の切れ目ではまるごと残っている");
    }

    #[test]
    fn 届かない手は反則負け() {
        let (mut p, mut seq) = player(Board::new(), &[Piece::T]);
        // 空中に浮いた位置
        let r = p.place(&at(Piece::T, Rotation::North, 4, 5), &rules(), &mut seq);
        assert_eq!(r, Err(Loss::IllegalMove));
        assert_eq!(p.loss, Some(Loss::IllegalMove));
    }

    #[test]
    fn テトリスはゲージにたまりrenが途切れたら送る() {
        // 4段の右端が空いた盤面(上に1マス残るのでパフェにはならない)
        let board = Board::from_rows(&[".#........", "#########.", "#########.", "#########.", "#########."]);
        let (mut p, mut seq) = player(board, &[Piece::I, Piece::O]);
        let r = p.place(&at(Piece::I, Rotation::East, 9, 2), &rules(), &mut seq).unwrap();
        assert_eq!((r.send, p.gauge), (0, 4), "消した手ではまだ送らない");
        let r = p.place(&at(Piece::O, Rotation::North, 4, 0), &rules(), &mut seq).unwrap();
        assert_eq!((r.send, p.gauge, p.stats.sent), (4, 0, 4), "消さない手で放つ");
    }

    #[test]
    fn 攻撃はまず予告を相殺する() {
        // 4段の右端が空いた盤面(上に1マス残るのでパフェにはならない)
        let board = Board::from_rows(&[".#........", "#########.", "#########.", "#########.", "#########."]);
        let (mut p, mut seq) = player(board, &[Piece::I, Piece::O]);
        p.receive(3, &rules());
        p.place(&at(Piece::I, Rotation::East, 9, 2), &rules(), &mut seq).unwrap();
        assert_eq!(p.incoming_lines(), 0);
        assert_eq!((p.stats.cancelled, p.gauge), (3, 1));
    }

    #[test]
    fn 予告は時間差のあとラインを消さない手でせり上がる() {
        let rules = rules();
        assert_eq!(rules.garbage.delay_pieces, 1);
        let (mut p, mut seq) = player(Board::new(), &[Piece::O, Piece::O, Piece::O]);
        p.receive(2, &rules);
        let r = p.place(&at(Piece::O, Rotation::North, 0, 0), &rules, &mut seq).unwrap();
        assert!(!r.garbage_rose, "1手目はまだ待つ");
        let r = p.place(&at(Piece::O, Rotation::North, 2, 0), &rules, &mut seq).unwrap();
        assert!(r.garbage_rose);
        assert_eq!(p.stats.received, 2);
        assert_eq!(p.state.board.max_height(), 4, "おじゃま2行 + O の2段");
    }

    #[test]
    fn holdが空なら次のミノを使える() {
        let (mut p, mut seq) = player(Board::new(), &[Piece::T, Piece::I, Piece::S]);
        let r = p.place(&at(Piece::I, Rotation::North, 4, 0), &rules(), &mut seq).unwrap();
        assert!(r.used_hold);
        assert_eq!(p.hold, Some(Piece::T));
        assert_eq!(p.queue[0], Piece::S);
        assert_eq!(r.revealed, 2);
    }
}
