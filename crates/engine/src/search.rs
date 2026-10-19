//! ビーム探索。
//!
//! 1手ごとに、今の局面から置ける手(HOLD を使う手を含む)をすべて展開し、点数
//! (報酬の合計 + 盤面の評価)の高い順に `width` 個だけ残す。見えているミノ(操作ミノ + NEXT)を
//! 使い切るまで読み、最後に一番点数の高い局面へ至る1手目を選ぶ。
//!
//! 読む手数: HOLD が空のときに HOLD を使うと、ミノを1つ HOLD に置いたまま終わる。
//! 比べる手数をそろえるため、HOLD が空なら「見えているミノの数 − 1」手、HOLD があれば
//! 「見えているミノの数」手まで読む。

use std::collections::HashMap;

use crate::eval::{evaluate, reward, Weights};
use tetrisai_core::{generate_into, Move, Piece, Placement, PlayerState, Rules};

/// 探索の出発点(TBP の start で受け取った局面)。
#[derive(Clone, Debug, PartialEq)]
pub struct Root {
    pub state: PlayerState,
    pub hold: Option<Piece>,
    /// `queue[0]` が操作ミノ、続きが NEXT。
    pub queue: Vec<Piece>,
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
    }]];
    let mut nodes = 0u64;
    let mut moves = Vec::with_capacity(64);

    for depth in 0..depth_limit {
        let parents = &layers[depth];
        let mut children: Vec<Node> = Vec::new();
        let mut seen: HashMap<(tetrisai_core::Board, Option<Piece>, usize, bool, u32), usize> = HashMap::new();

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
                    let score = r + evaluate(weights, &state.board, state.back_to_back, t_available);
                    nodes += 1;

                    let first = if depth == 0 { u32::MAX } else { parent.first };
                    let child = Node { state, hold, next, reward: r, score, parent: pi as u32, mv: Some(*mv), first };
                    let key = (state.board, hold, next, state.back_to_back, state.consecutive_clears);
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
        let root = Root { state: PlayerState::new(board), hold, queue: queue.to_vec() };
        let options = SearchOptions { width, max_depth: 6 };
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
    fn 中断できる() {
        let root = Root { state: PlayerState::default(), hold: None, queue: vec![Piece::T, Piece::I] };
        let options = SearchOptions { width: 64, max_depth: 6 };
        assert!(search(&root, &options, &Weights::default(), &Rules::ppt2(), &|| true).is_none());
    }
}
