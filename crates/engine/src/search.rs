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
//!
//! 対戦の駆け引き(M4): 消した手の攻撃のうち相殺に使わなかった分を「相手に届く攻撃」として積み上げ、
//! 相手の危なさ(相手の高さ + 相手への予告)に応じて評価する(`eval::battle_value`)。相手が高いと
//! 届く攻撃の価値が上がるので、予告が来ていてもすぐ消して相殺せず、受けてから撃つ(相殺を外して
//! カウンターする)手が選ばれやすくなる。相手が次に撃てる攻撃の見積もりも、自分の高さの余裕として見る。

use std::collections::HashMap;

use crate::opening::Opening;
use crate::strategy::Strategy;
use crate::eval::{
    battle_value, evaluate, opponent_threat, ren_potential, reward, rival_fire_value, ren_cut_value, tsd_fire_value, BattleContext, EvalContext, Garbage, Weights,
};
use tetrisai_core::{generate_into, Board, ClearResult, Move, Piece, Placement, PlayerState, Rules, Spawn};

/// 予告の1件(相手の1回の攻撃)。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IncomingGarbage {
    pub lines: u32,
    /// あと何手でせり上がれるようになるか(0 なら次にラインを消さない手を置いたときにせり上がる)。
    pub delay: u32,
}

/// 相手の情報(分かる範囲で)。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Opponent {
    pub board: Board,
    /// 相手への予告(こちらが送って、相手がまだ受けていない攻撃)の行数
    pub incoming: u32,
    /// 相手の攻撃ゲージ(相手が REN 中にためている攻撃)
    pub gauge: u32,
    pub back_to_back: bool,
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
    /// 自分の攻撃ゲージ(REN 中にためている攻撃)
    pub gauge: u32,
    /// 相手の情報。無ければ相手を見ずに読む
    pub opponent: Option<Opponent>,
    /// 序盤の作戦(テンプレか素の状態。bot・対戦シミュレーターが `OpeningTracker` で選ぶ)
    pub opening: Opening,
    /// 作戦層の作戦(相手の情報があるとき、bot が `StrategyTracker` で選ぶ)
    pub strategy: Strategy,
    /// 序盤の作戦の間でも中あけ(C4W)の評価を使う(テンプレの途中で攻撃された時。`OpeningTracker::well_now`。2026-11-10)
    pub opening_well: bool,
    /// C4W を使わない(中あけの評価・REN を切った手の減点を使わない。`OpeningTracker::bans_c4w`。2026-11-13)
    pub no_c4w: bool,
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

/// 同じ局面をまとめるための鍵(盤面・HOLD・次のミノの位置・B2B・REN・予告の片付いた量・せり上がった量・届く攻撃)。
type NodeKey = (Board, Option<Piece>, usize, bool, u32, u32, u32, u32, u32, u8);

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
    /// 相手に届く攻撃(相殺に使った分を除く)の合計。
    outgoing: u32,
    /// 今の REN でためている、まとめて届く攻撃(相殺に使った分を除く。ラインを消さない手で 0 に戻る。2026-11-09)
    burst: u32,
    /// この局面までに1回にまとめて届く攻撃の最大
    chunk: u32,
    /// 序盤のテンプレの図どおりに置き続けているか。0 なら外れた。1 以上なら 1 + これまでに撃った回数(テンプレを選んでからの合計)。
    /// 【2026-11-08・ユーザー】撃った回数が `TEMPLATE_CLEARS` に届くまで(TST→TSD)続ける。
    on_template: u8,
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
    if result.lines > 0 && rules.garbage.cancel_incoming_first {
        consumed += result.attack.min(total - consumed);
    }
    if result.lines == 0 || rules.garbage.rise_on_clear {
        // 待ち時間の過ぎた予告を古い順にせり上げる(1回の固定で max_rise_per_piece 行まで。0 は上限なし)。
        // ぷよテト2 はラインを消した手(REN の途中)でも、相殺しきれなかった分がせり上がる(rise_on_clear)
        let mut budget = match rules.garbage.max_rise_per_piece {
            0 => u32::MAX,
            n => n,
        };
        let mut end = 0;
        for g in incoming {
            end += g.lines;
            if end <= consumed {
                continue;
            }
            if g.delay > depth || budget == 0 {
                break;
            }
            let n = (end - consumed).min(budget);
            consumed += n;
            risen += n;
            budget -= n;
        }
    }
    (consumed, risen)
}

/// 出現位置を `margin` 行だけ低く見て手を探す。そこから出せない(手が無い)時は、本来の出現位置で探す。
fn generate_safe(board: &Board, piece: Piece, spawn: Spawn, margin: u32, out: &mut Vec<Move>) {
    if margin > 0 {
        generate_into(board, piece, Spawn { x: spawn.x, y: spawn.y - margin as i8 }, out);
        if !out.is_empty() {
            return;
        }
    }
    generate_into(board, piece, spawn, out);
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
    // 【2026-11-09】早撃ちの間は、C4W 相手専用の重み一式(`Weights::rush`)があればそれで考える
    // 【2026-11-14】迎え撃ちの間は、迎え撃ちの重み一式(`Weights::counter`)で考える
    let weights = match (&root.strategy, &weights.rush, &weights.counter) {
        (crate::strategy::Strategy::Rush, Some(rush), _) => rush.as_ref(),
        (crate::strategy::Strategy::Counter, _, Some(counter)) => counter.as_ref(),
        _ => weights,
    };
    // 【2026-11-09】開幕パフェの間は、pco.rs が選んだ位置に置く(入らなければふつうに考える)
    if let Opening::Pco(loc) = &root.opening {
        if let Some(mv) = crate::pco::to_move(&root.state.board, &crate::pco::Action { loc: *loc }, rules) {
            return Some(SearchResult { candidates: vec![Candidate { mv, score: 0.0 }], plan: vec![mv], nodes: 0, depth: 1 });
        }
        let mut free = root.clone();
        free.opening = Opening::None;
        return search(&free, options, weights, rules, cancelled);
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
        outgoing: 0,
        burst: if root.state.consecutive_clears > 0 { root.gauge } else { 0 },
        chunk: 0,
        on_template: if let Opening::Template(t) = &root.opening { 1 + t.clears as u8 } else { 0 },
    }]];
    let incoming_total: u32 = root.incoming.iter().map(|g| g.lines).sum();
    let battle = BattleContext {
        opponent_danger: root.opponent.map(|o| {
            if weights.c4w_spawn_danger && root.strategy != crate::strategy::Strategy::Normal {
                crate::shape::spawn_danger(&o.board) + o.incoming
            } else {
                o.board.max_height() + o.incoming
            }
        }),
        opponent_spawn_danger: root.opponent.map(|o| crate::shape::spawn_danger(&o.board) + o.incoming),
        opponent_threat: root.opponent.map_or(0, |o| opponent_threat(rules, &o.board, o.gauge, o.back_to_back)),
        opponent_ren: root.opponent.map_or(0, |o| ren_potential(rules, &o.board, 0) + o.gauge),
        opponent_rows: root.opponent.map_or(0, |o| crate::shape::ren_rows(&o.board)),
        gauge: root.gauge,
        strategy: root.strategy,
        opponent_cancel: root.opponent.map_or(0, |o| crate::eval::opponent_cancel(rules, o.gauge)),
    };
    let mut nodes = 0u64;
    let mut moves = Vec::with_capacity(64);

    for depth in 0..depth_limit {
        let parents = &layers[depth];
        let mut children: Vec<Node> = Vec::new();
        let mut seen: HashMap<NodeKey, usize> = HashMap::new();

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

            // 【2026-11-08・ユーザー「はちみつ砲の2巡目を早々にあきらめた。序盤なので組み切って」】テンプレの間は、
            // 実際に置く手(根の手)に図どおりの手が1つでもあれば、図に無い手を選ばない(加点だけだと、評価の高い別の手で
            // 図から外れた: debug_log_20261108_192000 の1P 局4・2P 局6)
            // 【2026-11-11】図どおりの手が複数あれば、見えていない先のミノまで試して図どおりに置き続けられる見込みの高い手に絞り
            // (`template_allowed`)、縛るのもテンプレの終わり(TST の後の TSD・パフェ)までにした(`template_restricted`)
            let allowed: Vec<tetrisai_core::Placement> = match (&root.opening, depth) {
                (Opening::Template(t), 0) if !weights.template_bonus_only && template_restricted(parent.on_template) => {
                    template_allowed(t, root, &parent.state.board, &options_here[..count], spawn, weights.spawn_margin, rules)
                }
                _ => Vec::new(),
            };
            let template_only = !allowed.is_empty();
            for &(piece, hold, next) in &options_here[..count] {
                generate_safe(&parent.state.board, piece, spawn, weights.spawn_margin, &mut moves);
                for mv in &moves {
                    if mv.placement.location.cells().iter().all(|&(_, y)| y >= LOCK_OUT_Y) {
                        continue;
                    }
                    let off_template = template_only && !allowed.contains(&mv.placement);
                    let mut state = parent.state;
                    let result = state.apply(&mv.placement, rules);
                    // 図に無くても、Tスピンダブル以上とパフェは撃てる(消えた後の図と撃ち方がずれると、撃つ手が図に無い手に
                    // なる。縛るだけだと TST→TSD が 43% → 29%、TST→TSD→パフェが 21% → 15% に減った: 対戦シミュレーター)
                    // 【2026-11-11】TST の後は、図に無いパフェで TSD を飛ばさない(`tsd_pending`)
                    let pc_ok = result.perfect_clear && !matches!(&root.opening, Opening::Template(t) if tsd_pending(t));
                    if off_template && !(pc_ok || (result.spin == tetrisai_core::Spin::Full && result.lines >= 2)) {
                        continue;
                    }
                    // 【2026-11-14・ユーザー「くろみつ砲でIミノを入れずにTSTをしようとする挙動が見られました。悪手なので修正してください」】
                    // 図の最後に入れる T が、他のミノ(TST の壁の I など)より先に来ると、図どおりの手が無くなって縛りが外れ、T を図の溝へ
                    // 入れて消えない Tスピンを選んでいた(debug_log_20261114_135512 の1P 局2 102手目)。消えないなら溝に入れない。
                    // 図から外れて序盤が終わった後も、I を待たずに同じ溝へ T を入れたので、消えない Tスピン(spin Full で 0行)は図に
                    // かかわらず選ばない(溝の壁ができる前に T を使ってしまう手。T を HOLD して壁を先に作る手が残る)
                    if result.lines == 0
                        && mv.placement.location.piece == Piece::T
                        && (mv.placement.spin == tetrisai_core::Spin::Full
                            || matches!(&root.opening, Opening::Template(t) if t.premature_last(&parent.state.board, &mv.placement.location)))
                    {
                        continue;
                    }
                    let mut r = parent.reward
                        + reward(weights, rules, mv, &result)
                        + if root.no_c4w { 0.0 } else { ren_cut_value(weights, parent.state.consecutive_clears, result.lines, &state.board) }
                        + rival_fire_value(weights, root.strategy, parent.state.consecutive_clears, result.lines, &parent.state.board, battle.opponent_rows)
                        + if root.opening.is_active() {
                            0.0
                        } else {
                            tsd_fire_value(
                                weights,
                                &result,
                                parent.state.board.max_height() + parent.risen,
                                incoming_total - parent.consumed,
                            )
                        };
                    // 序盤のテンプレ: 図どおりの手に加点する。消した後は、消した回数が TEMPLATE_CLEARS に届くまで続ける
                    // (読みの中では消えた行を図から取り除かないので、消した後の加点は次に局面を受け取ってから)
                    let mut on_template = 0;
                    if let (1.., Opening::Template(t)) = (parent.on_template, &root.opening) {
                        if t.accepts(&parent.state.board, &mv.placement.location) && !first_stage_soft(parent.on_template, &parent.state.board, mv) {
                            r += weights.template_piece;
                            on_template = if result.lines == 0 {
                                parent.on_template
                            } else if (parent.on_template as u32) < crate::opening::TEMPLATE_CLEARS {
                                parent.on_template + 1
                            } else {
                                0
                            };
                        }
                    }
                    if matches!(root.opening, Opening::Tsd) && result.spin == tetrisai_core::Spin::Full && result.lines == 2 {
                        r += weights.opening_tsd;
                    }
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
                    // 相殺した行数 = 消費した予告のうち、せり上がらなかった分(消した手でもせり上がる規則があるので分ける。2026-11-08)
                        let cancelled_now = (consumed - parent.consumed) - (risen - parent.risen);
                    let outgoing = parent.outgoing + result.attack - cancelled_now;
                    let burst = if result.lines > 0 { parent.burst + result.attack - cancelled_now } else { 0 };
                    let chunk = parent.chunk.max(parent.burst).max(burst);
                    let our_top = state.board.max_height() + risen + pending;
                    let ctx = EvalContext {
                        t_available,
                        garbage,
                        consecutive_clears: state.consecutive_clears,
                        opponent_rows: battle.opponent_rows,
                        opening: root.opening.is_active() && !root.opening_well,
                        strategy: root.strategy,
                        no_c4w: root.no_c4w,
                    };
                    let score = r
                        + evaluate(weights, rules, &state.board, state.back_to_back, &ctx)
                        + battle_value(weights, &battle, outgoing, chunk, our_top, consumed - risen);
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
                        outgoing,
                        burst,
                        chunk,
                        on_template,
                    };
                    let key = (
                        state.board,
                        hold,
                        next,
                        state.back_to_back,
                        state.consecutive_clears,
                        consumed,
                        risen,
                        outgoing,
                        // まとめて届く攻撃は早撃ちの評価でだけ使う(ほかの作戦では今までどおり同じ局面とみなして1つにまとめる)
                        if root.strategy == crate::strategy::Strategy::Rush { chunk } else { 0 },
                        on_template,
                    );
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
                    generate_safe(&node.state.board, use_piece, spawn, weights.spawn_margin, &mut moves);
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
                        // 相殺した行数 = 消費した予告のうち、せり上がらなかった分(消した手でもせり上がる規則があるので分ける。2026-11-08)
                        let cancelled_now = (consumed - node.consumed) - (risen - node.risen);
                        let outgoing = node.outgoing + result.attack - cancelled_now;
                        let burst = if result.lines > 0 { node.burst + result.attack - cancelled_now } else { 0 };
                        let chunk = node.chunk.max(node.burst).max(burst);
                        let our_top = state.board.max_height() + risen + pending;
                        let ctx = EvalContext {
                            t_available: 1,
                            garbage,
                            consecutive_clears: state.consecutive_clears,
                        opponent_rows: battle.opponent_rows,
                            opening: root.opening.is_active() && !root.opening_well,
                            strategy: root.strategy,
                            no_c4w: root.no_c4w,
                        };
                        let s = node.reward
                            + reward(weights, rules, mv, &result)
                            + if root.no_c4w { 0.0 } else { ren_cut_value(weights, node.state.consecutive_clears, result.lines, &state.board) }
                            + rival_fire_value(weights, root.strategy, node.state.consecutive_clears, result.lines, &node.state.board, battle.opponent_rows)
                            + if root.opening.is_active() {
                                0.0
                            } else {
                                tsd_fire_value(
                                    weights,
                                    &result,
                                    node.state.board.max_height() + node.risen,
                                    incoming_total - node.consumed,
                                )
                            }
                            + evaluate(weights, rules, &state.board, state.back_to_back, &ctx)
                            + battle_value(weights, &battle, outgoing, chunk, our_top, consumed - risen);
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
    fn テンプレの間は図どおりの手があれば必ず選ぶ() {
        // 【2026-11-08・ユーザー「はちみつ砲の2巡目を早々にあきらめた。序盤なので組み切って」】
        // テンプレの加点を 0 にしても(評価がほかの手を好んでも)、根の手は図どおりの手から選ぶ
        let rules = Rules::ppt2();
        let honey = crate::opening::priority_templates()[1].clone();
        let queue = [Piece::I, Piece::O, Piece::L, Piece::S, Piece::Z, Piece::T];
        assert!(honey.buildable_hard_drop(&queue, None, &rules));
        let mut weights = Weights::default();
        weights.template_piece = 0.0;
        let root = Root {
            state: PlayerState::new(Board::new()),
            queue: queue.to_vec(),
            opening: Opening::Template(honey.clone()),
            ..Default::default()
        };
        let options = SearchOptions { width: 64, max_depth: 6, speculate: false };
        let r = search(&root, &options, &weights, &rules, &|| false).expect("手がある");
        let first = r.candidates[0].mv.placement.location;
        assert!(honey.accepts(&Board::new(), &first), "{first:?}");
        assert!(r.candidates.iter().all(|c| honey.accepts(&Board::new(), &c.mv.placement.location)));
        // テンプレでなければ縛らない
        let free = Root { opening: Opening::None, ..root };
        let r = search(&free, &options, &weights, &rules, &|| false).expect("手がある");
        assert!(r.candidates.iter().any(|c| !honey.accepts(&Board::new(), &c.mv.placement.location)));
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
    fn テンプレのtstの壁ができる前にtを溝へ入れない() {
        // 【2026-11-14・ユーザー「くろみつ砲でIミノを入れずにTSTをしようとする挙動が見られました。悪手なので修正してください」】
        // 実機(debug_log_20261114_135512 の1P 局2 102手目)では、操作ミノ T・HOLD O・次 O I O S Z で、0列目の I より先に
        // T を TST の溝へ入れ、1行も消えない Tスピンにしていた
        let board = crate::opening::tests::kuromitsu_before_wall();
        let kuro = std::sync::Arc::new(crate::opening::all_templates().iter().find(|t| t.name == "くろみつ砲").unwrap().as_ref().clone()).narrowed(&board);
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../config/weights/b2b.toml")).unwrap();
        let weights: Weights = toml::from_str(&text).unwrap();
        let root = Root {
            state: PlayerState::new(board),
            hold: Some(Piece::O),
            queue: vec![Piece::T, Piece::O, Piece::I, Piece::O, Piece::S, Piece::Z],
            opening: Opening::Template(kuro),
            ..Default::default()
        };
        for width in [8, 64, 512] {
            let options = SearchOptions { width, max_depth: 6, speculate: false };
            let r = search(&root, &options, &weights, &Rules::ppt2(), &|| false).expect("手がある");
            let mut state = root.state;
            let result = state.apply(&r.candidates[0].mv.placement, &Rules::ppt2());
            let first = r.candidates[0].mv.placement;
            assert!(
                !(first.location.piece == Piece::T && result.lines == 0 && first.spin == Spin::Full),
                "幅{width}: 消えない Tスピン {first:?}"
            );
            assert_ne!(first.location.sorted_cells(), [(6, 1), (5, 2), (6, 2), (6, 3)], "幅{width}: 壁の前に T を溝へ入れた");
            // 読み筋の中でも、消えない Tスピンは選ばない
            let mut st = root.state;
            for m in &r.plan {
                let p = m.placement;
                let res = st.apply(&p, &Rules::ppt2());
                assert!(!(p.location.piece == Piece::T && p.spin == Spin::Full && res.lines == 0), "幅{width}: 読み筋 {:?}", r.plan);
            }
        }
    }

    #[test]
    fn b2bの重みはb2bが切れていてもtsdの溝を掘りrenで崩さない() {
        // 【2026-11-14・ユーザー「C4W以外の盤面においてTSDのドネイトが出来上がっているが掘りRENで消化する癖がある」】
        // 実機(13:00 の1局目66手目)の局面。B2B が切れていると、TSD の点(1.43)が掘って穴・高さを減らす盤面の点の差より小さく、
        // T を縦に置いて単発で消し、掘り REN を選んでいた。b2b.toml の TSD の点を 8 にして TSD を選ぶようにした
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../config/weights/b2b.toml")).unwrap();
        let weights: Weights = toml::from_str(&text).unwrap();
        let board = Board::from_rows(&[
            "#.........",
            "####...###",
            "#####..###",
            "####...###",
            "#####.####",
            "#########.",
            "#######.##",
            "####.#####",
            "####.#####",
            "####.#####",
            "####.#####",
            "####.#####",
        ]);
        let mut state = PlayerState::new(board);
        state.back_to_back = false;
        let root = Root {
            state,
            hold: Some(Piece::Z),
            queue: vec![Piece::T, Piece::S, Piece::L, Piece::I, Piece::J, Piece::Z],
            ..Default::default()
        };
        // 実機は思考を待たずに浅い探索(幅 8〜64)の手を使うことが多い。幅 128・256 ではまだ単発を選ぶ(点 8 は境目に近い)
        for width in [8, 64, 1024] {
            let options = SearchOptions { width, max_depth: 6, speculate: false };
            let r = search(&root, &options, &weights, &Rules::ppt2(), &|| false).expect("手がある");
            let first = r.candidates[0].mv.placement;
            assert_eq!((first.location.piece, first.spin), (Piece::T, Spin::Full), "幅{width}: {:?}", r.candidates[0]);
        }
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
        // 攻撃4で古いほうから4行を相殺(残りの1行はまだ待ち時間の中)
        assert_eq!(garbage_after(&incoming, 5, (0, 0), 0, &clear, &rules), (4, 0));
        // 攻撃の無い消去(REN の始め)でも、待ち時間の過ぎた予告はせり上がる(ぷよテト2)
        let single = ClearResult { lines: 1, attack: 0, ..Default::default() };
        assert!(rules.garbage.rise_on_clear);
        assert_eq!(garbage_after(&incoming, 5, (0, 0), 0, &single, &rules), (3, 3));
        let mut old = rules.clone();
        old.garbage.rise_on_clear = false;
        assert_eq!(garbage_after(&incoming, 5, (0, 0), 0, &single, &old), (0, 0));
        // 消さない手: 1手目(depth 0)では待ち時間0の3行だけせり上がる
        assert_eq!(garbage_after(&incoming, 5, (0, 0), 0, &none, &rules), (3, 3));
        // 3手目(depth 2)なら残りの2行もせり上がる
        assert_eq!(garbage_after(&incoming, 5, (3, 3), 2, &none, &rules), (5, 5));
        // ぷよテト2 は1回の固定で予告の全部がせり上がる(2026-11-08・実機ログで 10行・11行が1手でせり上がった)
        assert_eq!(rules.garbage.max_rise_per_piece, 0);
        let big = [IncomingGarbage { lines: 10, delay: 0 }];
        assert_eq!(garbage_after(&big, 10, (0, 0), 0, &none, &rules), (10, 10));
        // 上限のある規則なら、その行数まで
        let mut capped = rules.clone();
        capped.garbage.max_rise_per_piece = 7;
        assert_eq!(garbage_after(&big, 10, (0, 0), 0, &none, &capped), (7, 7));
        assert_eq!(garbage_after(&big, 10, (7, 7), 1, &none, &capped), (10, 10));
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

/// 根の手を図どおりの手に絞る段階か(`on_template` は 1 + 撃った回数)。
/// 【2026-11-08】以前は最初の消去(TST)まで(消えた後も縛ると TST→TSD が 43% → 39% に減った)。それは後のミノの置き場を
/// 見ずに縛っていたためで、先のミノまで試して選ぶ(`template_allowed`)ようにしてからは、テンプレの終わりまで縛る方が
/// TSD・パフェまで届く(2026-11-11、`td_solo_probe`)。環境変数 TD_RESTRICT_CLEARS(比べる時用)で何回撃つまで縛るかを変えられる。
fn template_restricted(on_template: u8) -> bool {
    static LIMIT: std::sync::OnceLock<u8> = std::sync::OnceLock::new();
    let limit = *LIMIT.get_or_init(|| std::env::var("TD_RESTRICT_CLEARS").ok().and_then(|s| s.parse().ok()).unwrap_or(crate::opening::TEMPLATE_CLEARS as u8));
    (1..=limit).contains(&on_template)
}

/// テンプレの間に根で選んでよい手(空なら縛らない)。
/// 図どおりの手(1巡目は T 以外のソフトドロップなしで、置いた後もソフトドロップなしで1巡目を組み続けられる手)が複数あれば、
/// 見えているミノ(と袋の残り)の後に見えていない先のミノをランダムに足した `continue_samples` 通りで、図どおりに置き続けられる
/// 割合を試し、「置き続けられる割合 + `hard_bonus` × T 以外をハードドロップだけで置き続けられる割合」が一番高い手に絞る。
///
/// 【2026-11-11・ユーザー「TDテンプレについてさらに最適化を追究したい。2巡目でもソフトドロップしないのが好ましい。
/// パフェができる確率が高いものを用いたい」】以前は今のミノを図に置けるかしか見ず、HOLD の使い方を誤って後のミノの置き場が
/// 無くなりテンプレが終わっていた(ミノ順を全部知っていれば 皐月積みは TSD まで 92% 届くのに、探索では 4%)。
fn template_allowed(
    t: &std::sync::Arc<crate::opening::Template>,
    root: &Root,
    board: &Board,
    options: &[(Piece, Option<Piece>, usize)],
    spawn: tetrisai_core::Spawn,
    spawn_margin: u32,
    rules: &Rules,
) -> Vec<tetrisai_core::Placement> {
    let mut moves = Vec::with_capacity(64);
    // (手, 置いた後の HOLD, 次の位置)
    let mut accepted: Vec<(tetrisai_core::Move, Option<Piece>, usize)> = Vec::new();
    for &(piece, hold, next) in options {
        generate_safe(board, piece, spawn, spawn_margin, &mut moves);
        for mv in &moves {
            // 1巡目(盤面が28マスに満たない間)は T 以外のソフトドロップを使わない(どの段階でも)
            if t.accepts(board, &mv.placement.location) && !first_stage_soft(1 + t.clears as u8, board, mv) {
                accepted.push((*mv, hold, next));
            }
        }
    }
    let cells: u32 = board.cols.iter().map(|c| c.count_ones()).sum();
    // 1巡目は、置いた後もソフトドロップなしで1巡目を組み続けられる手だけ(無ければ縛らない)
    if t.clears == 0 && cells < 24 {
        accepted.retain(|(mv, hold, next)| first_stage_continues(t, root, board, mv, *hold, *next, rules));
        if accepted.is_empty() {
            return Vec::new();
        }
    }
    let samples = continue_samples();
    if accepted.len() <= 1 || samples == 0 {
        return accepted.iter().map(|(mv, ..)| mv.placement).collect();
    }
    // bot は同じ局面を幅を広げながら読み直すので、直前と同じ局面なら前の結果を使う
    type Key = (Board, Vec<Piece>, Option<Piece>, Vec<Piece>, String, u32, u32);
    thread_local! {
        static LAST: std::cell::RefCell<Option<(Key, Vec<tetrisai_core::Placement>)>> = const { std::cell::RefCell::new(None) };
    }
    let key: Key = (*board, root.queue.clone(), root.hold, root.bag.clone(), t.name.clone(), t.clears, t.rise);
    if let Some(hit) = LAST.with(|l| l.borrow().as_ref().filter(|(k, _)| *k == key).map(|(_, v)| v.clone())) {
        return hit;
    }
    // 候補が複数あれば、見えていない先のミノまで試した割合で点数を付け、一番高い手だけにする
    let rate = |mv: &tetrisai_core::Move, hold: Option<Piece>, next: usize, hard: bool| {
        let mut s = tetrisai_core::PlayerState::new(*board);
        let r = s.apply(&mv.placement, rules);
        let tsd = r.spin == tetrisai_core::Spin::Full && r.lines >= 2;
        // TST を撃った後(1回撃った後)は、TSD を撃つまでのパフェを組めたとみなさない
        let need_tsd = !t.pc_after_tst && ((t.clears == 0 && r.spin == tetrisai_core::Spin::Full && r.lines == 3) || (tsd_pending(t) && !tsd));
        if r.perfect_clear {
            return if tsd_pending(t) && !tsd { 0.0 } else { 1.0 };
        }
        match t.follow(&s.board, r.lines) {
            Some(nt) => {
                nt.continue_rate(&s.board, root.queue.get(next..).unwrap_or(&[]), hold, &root.bag, rules, samples, hard, need_tsd)
            }
            None => 0.0,
        }
    };
    let bonus = hard_bonus();
    let scores: Vec<f32> = accepted
        .iter()
        .map(|(mv, hold, next)| {
            let is_hard = mv.placement.location.piece == Piece::T || mv.soft_drop_sections == 0;
            let h = if is_hard && bonus > 0.0 { rate(mv, *hold, *next, true) } else { 0.0 };
            rate(mv, *hold, *next, false) + bonus * h
        })
        .collect();
    let best = scores.iter().copied().fold(f32::MIN, f32::max);
    let chosen: Vec<tetrisai_core::Placement> =
        accepted.iter().zip(&scores).filter(|&(_, &sc)| sc >= best - 1e-6).map(|((mv, ..), _)| mv.placement).collect();
    LAST.with(|l| *l.borrow_mut() = Some((key, chosen.clone())));
    chosen
}

/// テンプレで TST を撃った後、TSD をまだ撃っていないか(1回だけ撃った後とみなす)。この間は TSD を撃たずにパフェにする
/// 道筋を選ばない(`template_allowed` の点数で 0、図に無いパフェも撃たない)。TSD の道筋が見込めない時(点数がすべて 0)だけ
/// TST→パフェになりうる。
///
/// 【2026-11-11・ユーザー「止める方法を調べて」(TSD を飛ばすパフェ)】実機 debug_log_20261111_204212 の10局中3局と、1人で
/// 組ませた時の 14〜45% が、TST の後に TSD を撃たずパフェになっていた(TST 6行 + パフェ 10行。TSD の約5行が抜ける)。
/// TBA の図に、TSD の溝を T 以外のミノで埋めて行を消す「3巡目 - パフェ狙い」の図(はちみつ砲など)があり、探索はパフェを高く
/// 見るのでその図を選んでいた。
///
/// 【2026-11-13】TST→パフェを狙うテンプレ(`Template::pc_after_tst`)は除く。
fn tsd_pending(t: &crate::opening::Template) -> bool {
    t.clears == 1 && !t.pc_after_tst
}

/// 点数の、T 以外をハードドロップで置き続けられる割合の重み(環境変数 TD_HARD_BONUS で変えられる。比べる時用)。
/// 【2026-11-11】`td_solo_probe`(1人で300局)で 0・0.3・1 を比べ、パフェまで届く割合は誤差の範囲で、0.3 は 0 より2巡目に
/// ソフトドロップを使う局が減った(はちみつ砲 49%→33%、迷走砲 39%→28%)。1 は 0.3 と差なし
fn hard_bonus() -> f32 {
    static B: std::sync::OnceLock<f32> = std::sync::OnceLock::new();
    *B.get_or_init(|| std::env::var("TD_HARD_BONUS").ok().and_then(|s| s.parse().ok()).unwrap_or(0.3))
}

/// `template_allowed` で先のミノを試す数(環境変数 TD_SAMPLES で変えられる。比べる時用)。
fn continue_samples() -> u32 {
    static N: std::sync::OnceLock<u32> = std::sync::OnceLock::new();
    *N.get_or_init(|| std::env::var("TD_SAMPLES").ok().and_then(|s| s.parse().ok()).unwrap_or(8))
}

/// テンプレの1巡目(まだ1回も消しておらず、盤面が7個分の28マスに満たない)の手で、ソフトドロップを使う手か(T は除く)。
/// 図どおりとみなさない。
/// 【2026-11-11・ユーザー「TDテンプレの中で1巡目にソフトドロップが不要になるものを選ぶ」】図のマスへの置き方でも、置く順によっては
/// 張り出しの下へソフトドロップで入れることになる。ハードドロップで入る順で組む。2巡目(TST の形を作る所)は差し込みが要る図が
/// あるので縛らない(縛ると TST まで届く割合が 81% → 63% に下がった)。
fn first_stage_soft(on_template: u8, board: &Board, mv: &tetrisai_core::Move) -> bool {
    let cells: u32 = board.cols.iter().map(|c| c.count_ones()).sum();
    on_template == 1 && cells < 28 && mv.soft_drop_sections > 0 && mv.placement.location.piece != Piece::T
}

/// テンプレの1巡目(盤面が28マスに満たない)の根の手で、置いた後も分かっているミノ(NEXT と袋の残り)でソフトドロップなしに
/// 組み続けられるか(1巡目でなければ true)。選ぶ時に確かめた「ソフトドロップなしで組める順」に沿って置くため(2026-11-11)。
/// これが無いと、今のミノはハードドロップで置けても後のミノで差し込みが要る順に入り、図から外れた(TST まで 81% → 71%)。
fn first_stage_continues(
    t: &crate::opening::Template,
    root: &Root,
    board: &Board,
    mv: &tetrisai_core::Move,
    hold: Option<Piece>,
    next: usize,
    rules: &Rules,
) -> bool {
    let cells: u32 = board.cols.iter().map(|c| c.count_ones()).sum();
    if t.clears > 0 || cells >= 24 {
        return true; // 1巡目の最後の1個か、1巡目の後
    }
    let mut after = *board;
    after.place(&mv.placement.location);
    t.continues_hard_drop(&after, root.queue.get(next..).unwrap_or(&[]), hold, &root.bag, rules)
}
