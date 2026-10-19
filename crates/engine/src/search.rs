//! ビーム探索。
//!
//! 1手ごとに、今の局面から置ける手(HOLD を使う手を含む)をすべて展開し、点数
//! (報酬の合計 + 盤面の評価)の高い順に `width` 個だけ残す。見えているミノ(操作ミノ + NEXT)を
//! 使い切るまで読み、最後に一番点数の高い局面へ至る1手目を選ぶ。
//!
//! 読む手数: HOLD が空のときに HOLD を使うと、ミノを1つ HOLD に置いたまま終わる。
//! 比べる手数をそろえるため、HOLD が空なら「見えているミノの数 − 1」手、HOLD があれば
//! 「見えているミノの数」手まで読む。
//!
//! 予測の1手(`speculate`): 見えているミノを使い切った局面に、次に来うるミノ(袋の残りから絞る)を
//! それぞれ一番良い場所へ置いてみて、その平均を局面の点数にする。運の悪いミノが来ると困る形を避けられる。
//!
//! 予告(届く予定のおじゃま)も探索の中で再現する: ラインを消す手は攻撃の分だけ予告を打ち消し(相殺)、
//! 消さない手では待ち時間の過ぎた予告がせり上がったとみなす。穴の位置は分からないので盤面には足さず、
//! 「せり上がった分 + まだ残っている予告」を盤面の高さに上乗せして評価する。

use std::collections::HashMap;

use crate::eval::{evaluate, reward, Garbage, Weights};
use tetrisai_core::{generate_into, ClearResult, Move, Piece, Placement, PlayerState, Rules};

/// 予告の1件(相手の1回の攻撃)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IncomingGarbage {
    pub lines: u32,
    /// あと何手でせり上がれるようになるか(0 なら次にラインを消さない手を置いたときにせり上がる)。
    pub delay: u32,
}

/// 探索の出発点(TBP の start で受け取った局面)。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Root {
    pub state: PlayerState,
    pub hold: Option<Piece>,
    /// `queue[0]` が操作ミノ、続きが NEXT。
    pub queue: Vec<Piece>,
    /// 自分への予告(古い順)。
    pub incoming: Vec<IncomingGarbage>,
    /// NEXT の最後のミノの後で、今の袋(7種1巡)に残っているミノ。空なら次は新しい袋(7種どれでも)。
    /// 袋の情報が無いときも空にしておく(7種どれでも来うるとみなす)。
    pub bag: Vec<Piece>,
}

impl Root {
    /// 新しく見えたミノを NEXT の最後に足し、袋の残りを更新する。
    pub fn push_piece(&mut self, piece: Piece) {
        self.queue.push(piece);
        if self.bag.is_empty() {
            self.bag = Piece::ALL.to_vec();
        }
        if let Some(i) = self.bag.iter().position(|&p| p == piece) {
            self.bag.remove(i);
        }
    }

    /// 見えている NEXT の次に来うるミノ。
    fn possible_next(&self) -> Vec<Piece> {
        if self.bag.is_empty() {
            Piece::ALL.to_vec()
        } else {
            self.bag.clone()
        }
    }
}

impl Root {
    /// 指した手で局面を進める(TBP の `play`)。手のミノが操作ミノ・HOLD(HOLD が空なら NEXT の1つ目)の
    /// どれでもなければ false を返す(盤面だけは進める)。
    pub fn play(&mut self, placement: &Placement, rules: &Rules) -> bool {
        let piece = placement.location.piece;
        let current = self.queue.first().copied();
        let ok = if current == Some(piece) {
            self.queue.remove(0);
            true
        } else if self.hold == Some(piece) && current.is_some() {
            self.hold = Some(self.queue.remove(0));
            true
        } else if self.hold.is_none() && self.queue.get(1) == Some(&piece) {
            self.hold = Some(self.queue.remove(0));
            self.queue.remove(0);
            true
        } else {
            if !self.queue.is_empty() {
                self.queue.remove(0);
            }
            false
        };
        self.state.apply(placement, rules);
        ok
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SearchOptions {
    /// 各手で残す局面の数。
    pub width: usize,
    /// 読む手数の上限。
    pub max_depth: usize,
    /// 見えているミノの先を1手だけ予測して読むか。
    pub speculate: bool,
}

/// 1手目の候補。
#[derive(Clone, Copy, Debug)]
pub struct Candidate {
    pub mv: Move,
    /// この1手目から読んだ最善の局面の点数。
    pub score: f32,
}

#[derive(Clone, Debug)]
pub struct SearchResult {
    /// 1手目の候補(良い順)。
    pub candidates: Vec<Candidate>,
    /// 最善の読み筋(1手目から順。各手の座標はその手を置く直前の盤面のもの)。
    pub plan: Vec<Move>,
    /// 評価した局面の数。
    pub nodes: u64,
    /// 読んだ手数。
    pub depth: usize,
}

/// 置いたミノが全部この高さ以上なら負け(可視20行より上で固定した)。
const LOCK_OUT_Y: i8 = 20;

struct Node {
    state: PlayerState,
    hold: Option<Piece>,
    /// 次に操作ミノになるミノの、queue での位置。
    next: usize,
    reward: f32,
    score: f32,
    /// 1つ前の手番の層での位置。
    parent: u32,
    /// この局面に至った手。
    mv: Option<Move>,
    /// この局面に至る1手目の、1手目の層での位置。
    first: u32,
    /// 予告のうち、相殺かせり上がりで片付いた行数(古いほうから)。
    consumed: u32,
    /// 予告のうち、せり上がったとみなした行数。
    risen: u32,
}

/// 1手置いた後の予告の状態 (consumed, risen)。`depth` はこの手が何手目か(0 から)。
fn garbage_after(
    incoming: &[IncomingGarbage],
    total: u32,
    (mut consumed, mut risen): (u32, u32),
    depth: u32,
    result: &ClearResult,
    rules: &Rules,
) -> (u32, u32) {
    if result.lines > 0 {
        if rules.garbage.cancel_incoming_first {
            consumed += result.attack.min(total - consumed);
        }
    } else {
        // 待ち時間の過ぎた予告を古い順にせり上げる
        let mut end = 0;
        for g in incoming {
            end += g.lines;
            if end <= consumed {
                continue;
            }
            if g.delay > depth {
                break;
            }
            let n = end - consumed;
            consumed += n;
            risen += n;
        }
    }
    (consumed, risen)
}

/// 探索する。`cancelled` が true を返したら途中でやめて None を返す。
/// 置ける手が1つも無い(負け)ときも None。
pub fn search(
    root: &Root,
    options: &SearchOptions,
    weights: &Weights,
    rules: &Rules,
    cancelled: &dyn Fn() -> bool,
) -> Option<SearchResult> {
    if root.queue.is_empty() {
        return None;
    }
    let depth_limit = if root.hold.is_some() { root.queue.len() } else { root.queue.len().saturating_sub(1).max(1) };
    let depth_limit = depth_limit.min(options.max_depth).max(1);
    let spawn = rules.spawn();

    let mut layers: Vec<Vec<Node>> = vec![vec![Node {
        state: root.state,
        hold: root.hold,
        next: 0,
        reward: 0.0,
        score: 0.0,
        parent: 0,
        mv: None,
        first: 0,
        consumed: 0,
        risen: 0,
    }]];
    let incoming_total: u32 = root.incoming.iter().map(|g| g.lines).sum();
    let mut nodes = 0u64;
    let mut moves = Vec::with_capacity(64);

    for depth in 0..depth_limit {
        let parents = &layers[depth];
        let mut children: Vec<Node> = Vec::new();
        let mut seen: HashMap<(tetrisai_core::Board, Option<Piece>, usize, bool, u32, u32, u32), usize> =
            HashMap::new();

        for (pi, parent) in parents.iter().enumerate() {
            if cancelled() {
                return None;
            }
            let Some(&current) = root.queue.get(parent.next) else { continue };
            // (置くミノ, 置いた後の HOLD, 置いた後の next)
            let mut options_here: [(Piece, Option<Piece>, usize); 2] = [(current, parent.hold, parent.next + 1); 2];
            let mut count = 1;
            match parent.hold {
                Some(h) if h != current => {
                    options_here[1] = (h, Some(current), parent.next + 1);
                    count = 2;
                }
                None => {
                    if let Some(&after) = root.queue.get(parent.next + 1) {
                        if after != current {
                            options_here[1] = (after, Some(current), parent.next + 2);
                            count = 2;
                        }
                    }
                }
                _ => {}
            }

            for &(piece, hold, next) in &options_here[..count] {
                generate_into(&parent.state.board, piece, spawn, &mut moves);
                for mv in &moves {
                    if mv.placement.location.cells().iter().all(|&(_, y)| y >= LOCK_OUT_Y) {
                        continue;
                    }
                    let mut state = parent.state;
                    let result = state.apply(&mv.placement, rules);
                    let r = parent.reward + reward(weights, rules, mv, &result);
                    let t_known = hold.map_or(0, |h| (h == Piece::T) as usize)
                        + root.queue[next.min(root.queue.len())..].iter().filter(|&&p| p == Piece::T).count();
                    let t_available = t_known.clamp(1, 2);
                    let (consumed, risen) = garbage_after(
                        &root.incoming,
                        incoming_total,
                        (parent.consumed, parent.risen),
                        depth as u32,
                        &result,
                        rules,
                    );
                    let pending = incoming_total - consumed;
                    let garbage = Garbage { extra_height: risen + pending, pending };
                    let score = r + evaluate(weights, &state.board, state.back_to_back, t_available, garbage);
                    nodes += 1;

                    let first = if depth == 0 { u32::MAX } else { parent.first };
                    let child = Node {
                        state,
                        hold,
                        next,
                        reward: r,
                        score,
                        parent: pi as u32,
                        mv: Some(*mv),
                        first,
                        consumed,
                        risen,
                    };
                    let key = (state.board, hold, next, state.back_to_back, state.consecutive_clears, consumed, risen);
                    match seen.get(&key) {
                        Some(&i) if children[i].score >= score => {}
                        Some(&i) => children[i] = child,
                        None => {
                            seen.insert(key, children.len());
                            children.push(child);
                        }
                    }
                }
            }
        }

        if children.is_empty() {
            break;
        }
        if children.len() > options.width {
            children.select_nth_unstable_by(options.width - 1, |a, b| b.score.total_cmp(&a.score));
            children.truncate(options.width);
        }
        if depth == 0 {
            for (i, c) in children.iter_mut().enumerate() {
                c.first = i as u32;
            }
        }
        layers.push(children);
    }

    let last = layers.len() - 1;
    if last == 0 {
        return None; // 置ける手が無い
    }

    if options.speculate {
        let possible = root.possible_next();
        let mut leaves = std::mem::take(&mut layers[last]);
        for node in leaves.iter_mut() {
            if cancelled() {
                return None;
            }
            // 次の操作ミノ: まだ見えているミノが残っていればそれ、無ければ袋の残りのどれか
            let candidates: Vec<Piece> = match root.queue.get(node.next) {
                Some(&p) => vec![p],
                None => possible.clone(),
            };
            let mut sum = 0.0;
            for &piece in &candidates {
                let mut best = f32::NEG_INFINITY;
                // 来たミノか HOLD のミノのどちらかを置ける
                for use_piece in [Some(piece), node.hold.filter(|&h| h != piece)].into_iter().flatten() {
                    generate_into(&node.state.board, use_piece, spawn, &mut moves);
                    for mv in &moves {
                        if mv.placement.location.cells().iter().all(|&(_, y)| y >= LOCK_OUT_Y) {
                            continue;
                        }
                        let mut state = node.state;
                        let result = state.apply(&mv.placement, rules);
                        let (consumed, risen) = garbage_after(
                            &root.incoming,
                            incoming_total,
                            (node.consumed, node.risen),
                            last as u32,
                            &result,
                            rules,
                        );
                        let pending = incoming_total - consumed;
                        let garbage = Garbage { extra_height: risen + pending, pending };
                        let s = node.reward
                            + reward(weights, rules, mv, &result)
                            + evaluate(weights, &state.board, state.back_to_back, 1, garbage);
                        nodes += 1;
                        best = best.max(s);
                    }
                }
                // 置けないミノが来たら負け
                sum += if best.is_finite() { best } else { -1000.0 };
            }
            node.score = sum / candidates.len() as f32;
        }
        layers[last] = leaves;
    }

    // 1手目ごとの最善の点数
    let mut best_of_first = vec![f32::NEG_INFINITY; layers[1].len()];
    for node in &layers[last] {
        let f = node.first as usize;
        best_of_first[f] = best_of_first[f].max(node.score);
    }
    let mut candidates: Vec<Candidate> = layers[1]
        .iter()
        .zip(&best_of_first)
        .filter(|(_, &s)| s.is_finite())
        .map(|(n, &s)| Candidate { mv: n.mv.unwrap(), score: s })
        .collect();
    candidates.sort_by(|a, b| b.score.total_cmp(&a.score));

    // 最善の読み筋をたどる
    let best = layers[last].iter().enumerate().max_by(|a, b| a.1.score.total_cmp(&b.1.score)).map(|(i, _)| i)?;
    let mut plan = Vec::with_capacity(last);
    let mut i = best;
    for d in (1..=last).rev() {
        let node = &layers[d][i];
        plan.push(node.mv.unwrap());
        i = node.parent as usize;
    }
    plan.reverse();

    Some(SearchResult { candidates, plan, nodes, depth: last })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tetrisai_core::{Board, Spin};

    fn run(board: Board, hold: Option<Piece>, queue: &[Piece], width: usize) -> SearchResult {
        let root = Root { state: PlayerState::new(board), hold, queue: queue.to_vec(), ..Default::default() };
        let options = SearchOptions { width, max_depth: 6, speculate: false };
        search(&root, &options, &Weights::default(), &Rules::ppt2(), &|| false).expect("手がある")
    }

    #[test]
    fn 読み筋の1手目は候補の先頭と同じ() {
        let r = run(Board::new(), None, &[Piece::T, Piece::I, Piece::O, Piece::S, Piece::Z, Piece::L], 64);
        assert_eq!(r.plan[0].placement, r.candidates[0].mv.placement);
        assert_eq!(r.plan.len(), 5, "HOLD が空なら見えているミノ − 1 手");
        assert!(r.nodes > 0);
    }

    #[test]
    fn tsdの溝があればtsdを撃つ() {
        let board = Board::from_rows(&[
            "##........",
            "#...######",
            "##.#######",
        ]);
        let r = run(board, None, &[Piece::T, Piece::I, Piece::O], 128);
        let first = r.candidates[0].mv.placement;
        assert_eq!(first.location.piece, Piece::T);
        assert_eq!(first.spin, Spin::Full, "{:?}", r.candidates[0]);
    }

    #[test]
    fn テトリスを取れる局面ではiでテトリス() {
        let board = Board::from_rows(&["#########."; 4]);
        let r = run(board, Some(Piece::O), &[Piece::I, Piece::T, Piece::L], 128);
        let first = r.candidates[0].mv;
        let mut state = PlayerState::new(Board::from_rows(&["#########."; 4]));
        let result = state.apply(&first.placement, &Rules::ppt2());
        assert_eq!(result.lines, 4, "{first:?}");
    }

    #[test]
    fn 予告がせり上がると危ないなら相殺する() {
        // 13段の高さで、右端の井戸に I を入れればテトリス(攻撃4)。予告は8行ですぐせり上がる。
        // 消さずに積むと 13 + 8 = 21 段で負けなので、テトリスで相殺するはず
        let mut rows = vec!["#########."; 13];
        rows.insert(0, "#.........");
        let board = Board::from_rows(&rows);
        let root = Root {
            state: PlayerState::new(board),
            hold: None,
            queue: vec![Piece::I, Piece::O, Piece::O],
            incoming: vec![IncomingGarbage { lines: 8, delay: 0 }],
            ..Default::default()
        };
        let options = SearchOptions { width: 64, max_depth: 6, speculate: false };
        let r = search(&root, &options, &Weights::default(), &Rules::ppt2(), &|| false).unwrap();
        let mut state = root.state;
        let result = state.apply(&r.candidates[0].mv.placement, &Rules::ppt2());
        assert_eq!(result.lines, 4, "{:?}", r.candidates[0]);
    }

    #[test]
    fn 予告の相殺とせり上がりの計算() {
        let rules = Rules::ppt2();
        let incoming = [IncomingGarbage { lines: 3, delay: 0 }, IncomingGarbage { lines: 2, delay: 2 }];
        let clear = ClearResult { lines: 2, attack: 4, ..Default::default() };
        let none = ClearResult::default();
        // 攻撃4で古いほうから4行を相殺
        assert_eq!(garbage_after(&incoming, 5, (0, 0), 0, &clear, &rules), (4, 0));
        // 消さない手: 1手目(depth 0)では待ち時間0の3行だけせり上がる
        assert_eq!(garbage_after(&incoming, 5, (0, 0), 0, &none, &rules), (3, 3));
        // 3手目(depth 2)なら残りの2行もせり上がる
        assert_eq!(garbage_after(&incoming, 5, (3, 3), 2, &none, &rules), (5, 5));
    }

    #[test]
    fn 中断できる() {
        let root = Root { queue: vec![Piece::T, Piece::I], ..Default::default() };
        let options = SearchOptions { width: 64, max_depth: 6, speculate: false };
        assert!(search(&root, &options, &Weights::default(), &Rules::ppt2(), &|| true).is_none());
    }

    #[test]
    fn 予測の1手を読んでも1手目は候補の先頭と同じ() {
        let root = Root {
            queue: vec![Piece::T, Piece::I, Piece::O],
            bag: vec![Piece::S, Piece::Z],
            ..Default::default()
        };
        let options = SearchOptions { width: 32, max_depth: 6, speculate: true };
        let r = search(&root, &options, &Weights::default(), &Rules::ppt2(), &|| false).unwrap();
        assert_eq!(r.plan[0].placement, r.candidates[0].mv.placement);
    }

    #[test]
    fn 袋の残りの更新() {
        let mut root = Root { bag: vec![Piece::S], ..Default::default() };
        root.push_piece(Piece::S);
        assert!(root.bag.is_empty(), "袋を使い切った");
        assert_eq!(root.possible_next().len(), 7, "次は新しい袋");
        root.push_piece(Piece::T);
        assert_eq!(root.bag.len(), 6);
    }
}
