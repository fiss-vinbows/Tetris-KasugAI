//! TDテンプレごとに、探索(見えるのは操作ミノ + NEXT 5 と HOLD)が1人で TST→TSD→パフェまで組み切る割合と、
//! T 以外のソフトドロップの手を数える(2026-11-11)。相手はいない(攻撃を受けない)。
//! 1巡目をソフトドロップなしで組める局だけを数える。テンプレが終わったらその局は終わり。
//!
//! 使い方: cargo run --release -p tetrisai-engine --example td_solo_probe -- [局数] [重みの TOML] [テンプレ名(省略で全部)]
//! 環境変数: SOLO_THREADS(並列の数。既定 8)・SOLO_SEED(その局だけ)・SOLO_DEBUG(終わった所を表示)

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use tetrisai_core::{Piece, Rules, Spin};
use tetrisai_engine::opening::{priority_templates, td_templates, Template};
use tetrisai_engine::{search, Opening, Root, SearchOptions, Weights};

static SEARCH_US: AtomicU64 = AtomicU64::new(0);
static SEARCH_MAX_US: AtomicU64 = AtomicU64::new(0);
static SEARCHES: AtomicU64 = AtomicU64::new(0);

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

/// 1局の結果
#[derive(Clone, Copy, Default)]
struct Outcome {
    /// 到達した段階: 0 なし・1 TST・2 TST→TSD・3 TST→TSD→パフェ
    stage: u8,
    /// TST の後、TSD を撃たずにパフェ
    tst_pc: bool,
    /// T 以外のソフトドロップの手数(1巡目・2巡目・それより後)
    soft: [u32; 3],
    /// パフェ(か終わり)までに置いた数
    pieces: u32,
}

/// 探索の幅(環境変数 SOLO_WIDTH。既定 128。bot の最初の答えは幅 8)
fn width() -> usize {
    std::env::var("SOLO_WIDTH").ok().and_then(|s| s.parse().ok()).unwrap_or(128)
}

fn debug() -> bool {
    std::env::var("SOLO_DEBUG").is_ok()
}

fn play(t: &Arc<Template>, seq: &[Piece], weights: &Weights, rules: &Rules) -> Outcome {
    let mut root = Root::default();
    let mut it = seq.iter().copied();
    for _ in 0..6 {
        root.push_piece(it.next().unwrap());
    }
    let mut tmpl = t.clone();
    let mut out = Outcome::default();
    for n in 0..32u32 {
        root.opening = Opening::Template(tmpl.clone());
        let options = SearchOptions { width: width(), max_depth: 6, speculate: false };
        let started = std::time::Instant::now();
        let found = search(&root, &options, weights, rules, &|| false);
        let us = started.elapsed().as_micros() as u64;
        SEARCH_US.fetch_add(us, Ordering::Relaxed);
        SEARCH_MAX_US.fetch_max(us, Ordering::Relaxed);
        SEARCHES.fetch_add(1, Ordering::Relaxed);
        // SOLO_SLOW_MS: これより遅かった探索の局面を出す(遅い所を探す用)
        if std::env::var("SOLO_SLOW_MS").ok().and_then(|v| v.parse::<u64>().ok()).is_some_and(|ms| us > ms * 1000) {
            eprintln!(
                "遅い探索 {}ms {n}手目 撃った{} 図{} 次{:?} HOLD{:?}
  {}",
                us / 1000,
                tmpl.clears,
                tmpl.forms.len(),
                root.queue,
                root.hold,
                root.state.board.to_rows().join("
  ")
            );
        }
        let Some(c) = found.as_ref().and_then(|r| r.candidates.first()) else { break };
        if c.mv.soft_drop_sections > 0 && c.mv.placement.location.piece != Piece::T {
            out.soft[(n / 7).min(2) as usize] += 1;
        }
        let mut st = root.state;
        let on_form = tmpl.accepts(&root.state.board, &c.mv.placement.location);
        let res = st.apply(&c.mv.placement, rules);
        if std::env::var("SOLO_TRACE").is_ok() {
            let live: Vec<String> = tmpl
                .forms
                .iter()
                .filter(|f| f.consistent(&root.state.board))
                .map(|f| format!("{}(T{}個)", f.section, f.items.iter().filter(|(p, _)| *p == Piece::T).count()))
                .collect();
            eprintln!("   図 {}個(盤面と合う {}): {:?}", tmpl.forms.len(), live.len(), live.iter().take(6).collect::<Vec<_>>());
            if std::env::var("SOLO_FORMS").ok().and_then(|s| s.parse::<u32>().ok()) == Some(n + 1) {
                for f in tmpl.forms.iter().filter(|f| f.consistent(&root.state.board)) {
                    eprintln!("    {} 印{:?} floor{:b}", f.section, f.last, f.floor);
                    for (i, (p, c)) in f.items.iter().enumerate() {
                        eprintln!("      {p:?} {c:?} 消える行{:b}", f.gaps.get(i).copied().unwrap_or(0));
                    }
                }
            }
            eprintln!(
                "{:>2}手目 {:?} 図どおり{} 消した{} spin{:?} SD{} 候補{}  次{:?} HOLD{:?}
  {}",
                n + 1,
                c.mv.placement.location.piece,
                on_form,
                res.lines,
                res.spin,
                c.mv.soft_drop_sections,
                found.as_ref().map_or(0, |r| r.candidates.len()),
                root.queue,
                root.hold,
                st.board.to_rows().join("
  ")
            );
        }
        root.play(&c.mv.placement, rules);
        if let Some(p) = it.next() {
            root.push_piece(p);
        }
        out.pieces = n + 1;
        if res.lines > 0 {
            let stage = out.stage;
            if stage == 0 && res.spin == Spin::Full && res.lines == 3 {
                out.stage = 1;
            } else if stage == 1 && res.spin == Spin::Full && res.lines >= 2 {
                out.stage = 2;
            } else if stage == 2 && res.perfect_clear {
                out.stage = 3;
            } else if stage == 1 && res.perfect_clear {
                out.tst_pc = true;
            } else if stage == 0 || res.perfect_clear {
                if debug() {
                    eprintln!("想定外の消去 {n}手目 段階{stage} 消した{} spin{:?} パフェ{}", res.lines, res.spin, res.perfect_clear);
                }
                break;
            }
            if res.perfect_clear {
                break;
            }
        }
        match tmpl.follow(&root.state.board, res.lines) {
            Some(next) => tmpl = next.narrowed(&root.state.board), // 実機(OpeningTracker)と同じく図を絞る
            None => {
                if debug() {
                    eprintln!(
                        "終わり {n}手目 段階{} 消した{} spin{:?} 次{:?} HOLD{:?}\n  {}",
                        out.stage,
                        res.lines,
                        res.spin,
                        root.queue,
                        root.hold,
                        root.state.board.to_rows().join("\n  ")
                    );
                }
                break;
            }
        }
    }
    if debug() {
        eprintln!("結果 段階{} TST→パフェ {}", out.stage, out.tst_pc);
    }
    out
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let games: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(300);
    let path = args.get(2).cloned().unwrap_or_else(|| "config/weights/run16.toml".to_string());
    let only = args.get(3).cloned();
    let threads: u64 = std::env::var("SOLO_THREADS").ok().and_then(|s| s.parse().ok()).unwrap_or(8);
    let only_seed: Option<u64> = std::env::var("SOLO_SEED").ok().and_then(|s| s.parse().ok());
    let weights: Weights = toml::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    let rules = Rules::ppt2();
    let templates: Vec<Arc<Template>> = priority_templates()
        .iter()
        .chain(td_templates())
        .filter(|t| only.as_ref().is_none_or(|o| *o == t.name))
        .cloned()
        .collect();
    println!("局数 {games}・重み {path}(1巡目をソフトドロップなしで組める局だけ)");
    for t in &templates {
        let tagged: Vec<(u64, Outcome)> = std::thread::scope(|s| {
            let handles: Vec<_> = (0..threads)
                .map(|tid| {
                    let (weights, rules) = (&weights, &rules);
                    s.spawn(move || {
                        let mut v = Vec::new();
                        for g in (tid..games).step_by(threads as usize) {
                            if only_seed.is_some_and(|s| s != g + 1) {
                                continue;
                            }
                            let mut seq = bag_sequence(g + 1, 42);
                            // SOLO_SEQ=ミノ順(実機のミノ順を再現する。足りない分は種の並び)
                            if let Ok(q) = std::env::var("SOLO_SEQ") {
                                let given: Vec<Piece> = q.chars().filter_map(Piece::from_char).collect();
                                seq.splice(..given.len(), given);
                            }
                            if only_seed.is_some() {
                                eprintln!("ミノ順 {:?}", &seq[..28]);
                            }
                            if t.buildable_hard_drop_known(&seq[..6], None, &seq[6..7], rules) {
                                v.push((g + 1, play(t, &seq, weights, rules)));
                            }
                        }
                        v
                    })
                })
                .collect();
            handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
        });
        // 局ごとの結果(SOLO_OUT=ファイル): テンプレ,種,1巡目の7個,段階,TST→パフェ,2巡目のソフトドロップ,3巡目以降の
        if let Ok(path) = std::env::var("SOLO_OUT") {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap();
            for (seed, o) in &tagged {
                let first: String = bag_sequence(*seed, 7).iter().map(|p| p.to_char()).collect();
                writeln!(f, "{},{seed},{first},{},{},{},{}", t.name, o.stage, o.tst_pc as u8, o.soft[1], o.soft[2]).unwrap();
            }
        }
        let results: Vec<Outcome> = tagged.into_iter().map(|(_, o)| o).collect();
        let n = results.len().max(1) as f64;
        let at = |s: u8| 100.0 * results.iter().filter(|r| r.stage >= s).count() as f64 / n;
        let soft = |k: usize| results.iter().map(|r| r.soft[k]).sum::<u32>() as f64 / n;
        let soft_games = |k: usize| 100.0 * results.iter().filter(|r| r.soft[k] > 0).count() as f64 / n;
        let pcs: Vec<u32> = results.iter().filter(|r| r.stage == 3).map(|r| r.pieces).collect();
        println!(
            "== {} {} 局: TST {:.1}%  TSD {:.1}%  パフェ {:.1}%(平均 {:.1}個目)  TST→パフェ {:.1}%  / T以外のソフトドロップ 2巡目 {:.2}手({:.1}%の局)・3巡目以降 {:.2}手({:.1}%の局)・1巡目 {:.2}手",
            t.name,
            results.len(),
            at(1),
            at(2),
            at(3),
            pcs.iter().sum::<u32>() as f64 / pcs.len().max(1) as f64,
            100.0 * results.iter().filter(|r| r.tst_pc).count() as f64 / n,
            soft(1),
            soft_games(1),
            soft(2),
            soft_games(2),
            soft(0),
        );
    }
    let n = SEARCHES.load(Ordering::Relaxed).max(1);
    println!(
        "探索1回の平均 {:.1}ms・最大 {:.1}ms(並列 {threads})",
        SEARCH_US.load(Ordering::Relaxed) as f64 / n as f64 / 1000.0,
        SEARCH_MAX_US.load(Ordering::Relaxed) as f64 / 1000.0
    );
    println!("先のミノまで試す1通りで調べた局面の最大 {}", tetrisai_engine::opening::WALK_NODES_MAX.load(Ordering::Relaxed));
}
