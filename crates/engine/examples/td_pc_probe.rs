//! TDテンプレごとに、TST→TSD→パフェまで組める割合を数える(2026-11-11)。
//! ミノ順をすべて知っている前提(上限の見積もり)で、図どおりの手だけを深さ優先で試す。
//! 1巡目をソフトドロップなしで組める局だけを数え、T 以外のソフトドロップを禁じる範囲を変えて比べる。
//!
//! 使い方: cargo run --release -p tetrisai-engine --example td_pc_probe -- [局数]

use std::collections::HashMap;

use tetrisai_core::{generate, Piece, PlayerState, Rules, Spin};
use tetrisai_engine::opening::{priority_templates, td_templates, Template};
use std::sync::Arc;

fn bag_sequence(seed: u64, n: usize) -> Vec<Piece> {
    let mut rng = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut out = Vec::new();
    while out.len() < n {
        let mut bag = Piece::ALL.to_vec();
        for i in (1..bag.len()).rev() {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            bag.swap(i, (rng % (i as u64 + 1)) as usize);
        }
        out.extend(bag);
    }
    out
}

/// 到達した段階: 0 なし・1 TST・2 TST→TSD・3 TST→TSD→パフェ
struct Ctx<'a> {
    seq: &'a [Piece],
    rules: Rules,
    /// この手数(置いた数)より前の手は、T 以外のソフトドロップを禁じる
    hard_until: u32,
    memo: HashMap<([u64; 10], Option<Piece>, usize, u32, u8), u8>,
}

fn dfs(c: &mut Ctx, state: PlayerState, t: &Arc<Template>, hold: Option<Piece>, idx: usize, placed: u32, stage: u8) -> u8 {
    if stage == 3 {
        return 3;
    }
    let key = (state.board.cols, hold, idx, t.clears, stage);
    if let Some(&v) = c.memo.get(&key) {
        return v;
    }
    let mut best = stage;
    let Some(&current) = c.seq.get(idx) else { return best };
    // (置くミノ, 置いた後の HOLD, 次の位置)
    let mut options = vec![(current, hold, idx + 1)];
    match hold {
        Some(h) if h != current => options.push((h, Some(current), idx + 1)),
        None => {
            if let Some(&after) = c.seq.get(idx + 1) {
                if after != current {
                    options.push((after, Some(current), idx + 2));
                }
            }
        }
        _ => {}
    }
    'outer: for (piece, hold, next) in options {
        for mv in generate(&state.board, piece, c.rules.spawn()) {
            if piece != Piece::T && placed < c.hard_until && mv.soft_drop_sections > 0 {
                continue;
            }
            let mut s = state;
            let r = s.apply(&mv.placement, &c.rules);
            // 図に無くても Tスピンダブル以上とパフェは撃てる(search.rs と同じ)
            let shot = r.perfect_clear || (r.spin == Spin::Full && r.lines >= 2);
            if !t.accepts(&state.board, &mv.placement.location) && !shot {
                continue;
            }
            let new_stage = if r.lines == 0 {
                stage
            } else if stage == 0 && r.spin == Spin::Full && r.lines == 3 {
                1
            } else if stage == 1 && r.spin == Spin::Full && r.lines >= 2 {
                2
            } else if stage == 2 && r.perfect_clear {
                3
            } else if stage >= 1 && !r.perfect_clear {
                // TSD の前後の別の消し方(パフェを2回に分けるなど)は段階を進めずに続ける
                stage
            } else {
                continue;
            };
            if new_stage == 3 {
                best = 3;
                break 'outer;
            }
            if r.lines > 0 && s.board.is_empty() {
                continue;
            }
            let Some(nt) = t.follow(&s.board, r.lines) else {
                best = best.max(new_stage);
                continue;
            };
            let v = dfs(c, s, &nt, hold, next, placed + 1, new_stage);
            best = best.max(v);
            if best == 3 {
                break 'outer;
            }
        }
    }
    c.memo.insert(key, best);
    best
}

fn main() {
    let games: u64 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(500);
    if let Some(samples) = std::env::var("TD_POLICY").ok().and_then(|s| s.parse().ok()) {
        for hard_until in [14, 7] {
            policy(games, samples, hard_until);
        }
        return;
    }
    let rules = Rules::ppt2();
    let templates: Vec<Arc<Template>> = priority_templates().iter().chain(td_templates()).cloned().collect();
    // PC_SEQ=ミノ順: そのミノ順で、テンプレごとの全部知っている時の段階(1巡目だけ縛る)を出す
    if let Ok(q) = std::env::var("PC_SEQ") {
        let seq: Vec<Piece> = q.chars().filter_map(Piece::from_char).collect();
        for t in &templates {
            if !t.buildable_hard_drop_known(&seq[..6], None, &seq[6..7], &rules) {
                continue;
            }
            let mut c = Ctx { seq: &seq, rules: rules.clone(), hard_until: 7, memo: HashMap::new() };
            println!("{} 段階{}", t.name, dfs(&mut c, PlayerState::default(), t, None, 0, 0, 0));
        }
        return;
    }
    let limits = [(7u32, "1巡目だけ"), (14, "2巡目まで"), (99, "最後まで")];
    println!("局数 {games}(ミノ順をすべて知っている前提。1巡目をソフトドロップなしで組める局だけ)");
    for t in &templates {
        let mut eligible = 0;
        // [縛る範囲][段階] の局数
        let mut counts = [[0u32; 4]; 3];
        for g in 0..games {
            let seq = bag_sequence(g + 1, 35);
            if !t.buildable_hard_drop_known(&seq[..6], None, &seq[6..7], &rules) {
                continue;
            }
            eligible += 1;
            for (k, &(limit, _)) in limits.iter().enumerate() {
                let mut c = Ctx { seq: &seq, rules: rules.clone(), hard_until: limit, memo: HashMap::new() };
                let stage = dfs(&mut c, PlayerState::default(), t, None, 0, 0, 0);
                for s in 1..=stage as usize {
                    counts[k][s] += 1;
                }
            }
        }
        let pct = |n: u32| if eligible == 0 { 0.0 } else { 100.0 * n as f64 / eligible as f64 };
        println!("== {}  1巡目を組める {eligible}/{games} ({:.1}%)", t.name, 100.0 * eligible as f64 / games as f64);
        for (k, &(_, name)) in limits.iter().enumerate() {
            println!(
                "  T以外のソフトドロップ禁止 {name:<6}: TST {:5.1}%  TSD {:5.1}%  パフェ {:5.1}%",
                pct(counts[k][1]),
                pct(counts[k][2]),
                pct(counts[k][3])
            );
        }
    }
}

/// 1巡目の7個(開幕に分かる分)から、テンプレの選び方ごとにパフェまで届く割合を比べる(環境変数 TD_POLICY=<標本数>)。
/// - 今の選び方: はちみつ砲 → 迷走砲 → ほかから均等(均等の部分は期待値で数える)
/// - 固定の順: 2巡目まで縛った時のパフェの割合の順
/// - 1巡目を見て選ぶ: 2巡目以降をランダムに `samples` 通り作って見積もり、一番高いテンプレ
/// - 全部知っている上限: 実際のミノ順で一番よいテンプレ
fn policy(games: u64, samples: u64, hard_until: u32) {
    let rules = Rules::ppt2();
    let templates: Vec<Arc<Template>> = priority_templates().iter().chain(td_templates()).cloned().collect();
    let static_order = ["くろみつ砲", "PC-Spin", "はちみつ砲", "迷走砲", "皐月積み", "ベーカリーTD", "ホットケーキ積み"];
    let stage_of = |t: &Arc<Template>, seq: &[Piece]| {
        let mut c = Ctx { seq, rules: rules.clone(), hard_until, memo: HashMap::new() };
        dfs(&mut c, PlayerState::default(), t, None, 0, 0, 0)
    };
    let results: Vec<[f64; 4]> = std::thread::scope(|s| {
        let handles: Vec<_> = (0..games)
            .map(|g| {
                let templates = &templates;
                let rules = &rules;
                let stage_of = &stage_of;
                s.spawn(move || {
                    let seq = bag_sequence(g + 1_000_001, 35);
                    let ok: Vec<&Arc<Template>> = templates
                        .iter()
                        .filter(|t| t.buildable_hard_drop_known(&seq[..6], None, &seq[6..7], rules))
                        .collect();
                    if ok.is_empty() {
                        return [0.0; 4];
                    }
                    let pc = |t: &Arc<Template>| (stage_of(t, &seq) == 3) as u8 as f64;
                    // 今の選び方
                    let cur = if let Some(t) = ok.iter().find(|t| t.name == "はちみつ砲") {
                        pc(t)
                    } else if let Some(t) = ok.iter().find(|t| t.name == "迷走砲") {
                        pc(t)
                    } else {
                        // ほかのテンプレか素の状態を均等に(素の状態はパフェ 0 とみなす)
                        ok.iter().map(|t| pc(t)).sum::<f64>() / (ok.len() + 1) as f64
                    };
                    let st = static_order.iter().find_map(|n| ok.iter().find(|t| t.name == *n)).map_or(0.0, |t| pc(t));
                    // 1巡目を見て選ぶ
                    let mut best = (-1.0, 0usize);
                    for (i, t) in ok.iter().enumerate() {
                        let mut hit = 0.0;
                        for k in 0..samples {
                            let mut alt = seq[..7].to_vec();
                            alt.extend(bag_sequence(g * 7919 + k * 104_729 + 17, 28));
                            hit += (stage_of(t, &alt) == 3) as u8 as f64;
                        }
                        if hit > best.0 {
                            best = (hit, i);
                        }
                    }
                    let cond = pc(ok[best.1]);
                    let max = ok.iter().map(|t| pc(t)).fold(0.0, f64::max);
                    [cur, st, cond, max]
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });
    let sum = |k: usize| 100.0 * results.iter().map(|r| r[k]).sum::<f64>() / games as f64;
    println!(
        "局数 {games}・標本 {samples}・T以外のソフトドロップ禁止 {hard_until}手まで: 今の選び方 {:.1}%  固定の順 {:.1}%  1巡目を見て選ぶ {:.1}%  全部知っている上限 {:.1}%",
        sum(0),
        sum(1),
        sum(2),
        sum(3)
    );
}
