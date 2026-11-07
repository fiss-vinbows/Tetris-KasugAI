//! C4W の使い方を数える道具(2026-11-07)。
//!
//! 【ユーザー「C4W から REN を切らさずに発火してほしいが、積みなおしたり真ん中にたくさん置いて TSD に変えてしまう」】
//! 中あけの行が MIN_ROWS 行以上ある間を1回の「C4W」とみなし、貯めた行数のうち何行を REN で使えたか、
//! 消さずに中央へ置いた手が何回あったかを数える。
//!
//! 使い方: c4w_audit --weights <A の TOML> [--opponent <B の TOML>] [--games N] [--width N] [--threads N] [--samples N]

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tetrisai_arena::agent::{Agent, BattleView, EngineAgent};
use tetrisai_arena::battle::{Loss, PieceSequence, Player};
use tetrisai_core::{Board, Rules, Spin};
use tetrisai_engine::{shape, OpeningTracker, Strategy, Weights};


#[derive(Default, Clone)]
struct Episode {
    peak: u32,
    best_chain: u32,
    chain: u32,
    fires: u32,
    center_no_clear: u32,
    side_no_clear: u32,
    breaks: u32,
    tspins: u32,
    lost: bool,
    trace: Vec<String>,
}

#[derive(Default)]
struct Totals {
    games: u64,
    pieces: u64,
    a_wins: u64,
    b_wins: u64,
    episodes: u64,
    peak: u64,
    best_chain: u64,
    fires: u64,
    center: u64,
    side: u64,
    breaks: u64,
    tspins: u64,
    lost: u64,
    good: u64,
    wrecked: u64,
    chain_hist: [u64; 8],
    /// 消さずに中央4列だけへ高い位置(8段目以上)に置いた手(実機のログでも同じ物差しで数えられる)
    high_center: u64,
    blocked: u64,
    cut_with_choice: u64,
    cut_table: [[u64; 10]; 10],
    cut_samples: Vec<String>,
    cut_forced: u64,
    moves_by_strategy: [u64; 3],
    rush_with_well: u64,
    wins_len: u64,
    samples: Vec<(u32, String)>,
}

fn draw(b: &Board) -> String {
    let top = b.max_height().min(24);
    (0..top)
        .rev()
        .map(|y| (0..10i8).map(|x| if b.occupied((x, y as i8)) { '#' } else { '.' }).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

fn finish(ep: &Episode, t: &mut Totals, keep: usize, wreck_center: u32) {
    t.episodes += 1;
    t.peak += ep.peak as u64;
    t.best_chain += ep.best_chain as u64;
    t.fires += ep.fires as u64;
    t.center += ep.center_no_clear as u64;
    t.side += ep.side_no_clear as u64;
    t.breaks += ep.breaks as u64;
    t.tspins += ep.tspins as u64;
    t.lost += ep.lost as u64;
    // 貯めた行の 7割以上を1つの REN で使えたら「発火できた」
    t.good += (ep.best_chain * 10 >= ep.peak * 7) as u64;
    // 消さずに中央へ3回以上置いて、REN を貯めた行の半分も使っていない = 崩した
    let wrecked = ep.center_no_clear >= wreck_center && ep.best_chain * 2 < ep.peak;
    t.wrecked += wrecked as u64;
    t.chain_hist[(ep.best_chain as usize / 2).min(7)] += 1;
    if wrecked && t.samples.len() < keep {
        t.samples.push((ep.peak, ep.trace.join("\n")));
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let get = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let num = |name: &str, d: u64| get(name).map(|s| s.parse::<u64>().unwrap()).unwrap_or(d);
    let load = |p: String| -> Weights { toml::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap() };
    let wa = load(get("--weights").expect("--weights"));
    let wb = get("--opponent").map(load).unwrap_or_else(|| wa.clone());
    let games = num("--games", 200);
    let width = num("--width", 128) as usize;
    let threads = num("--threads", 22);
    let keep = num("--samples", 0) as usize;
    let first = num("--seed", 1000);
    let max_pieces = num("--max-pieces", 400) as u32;
    // 実機に近づける: 考え直しの割合(百分率)・考え直しで REN と B2B を忘れる・予告の時間差を知らない
    let min_rows = num("--min-rows", 4) as u32;
    let wreck_center = num("--wreck-center", 3) as u32;
    let restart_pct = num("--restart-pct", 0);
    let forget = args.iter().any(|a| a == "--forget");
    let no_delay = args.iter().any(|a| a == "--no-delay");
    let block_margin = num("--block-margin", 0) as u32;
    let rules = Rules::ppt2();

    let next = Arc::new(AtomicU64::new(0));
    let totals = Arc::new(Mutex::new(Totals::default()));
    let mut handles = Vec::new();
    for _ in 0..threads {
        let (next, totals, wa, wb, rules) = (next.clone(), totals.clone(), wa.clone(), wb.clone(), rules.clone());
        handles.push(std::thread::spawn(move || loop {
            let i = next.fetch_add(1, Ordering::SeqCst);
            if i >= games {
                break;
            }
            let seed = first + i;
            let mut a = EngineAgent::new(wa.clone(), rules.clone(), width, "A");
            let mut b = EngineAgent::new(wb.clone(), rules.clone(), width, "B");
            a.opening = Some(OpeningTracker::new(seed));
            b.opening = Some(OpeningTracker::new(seed ^ 0x55));
            a.forget_on_start = forget;
            b.forget_on_start = forget;
            a.incoming_without_delay = no_delay;
            b.incoming_without_delay = no_delay;
            let mut restart_rng = seed.wrapping_mul(0x2545_F491_4F6C_DD1D) | 1;
            let mut seq = PieceSequence::new(seed);
            let hole = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xA5A5;
            let mut players = [Player::new(&mut seq, hole), Player::new(&mut seq, hole)];
            players[0].block_margin = block_margin;
            players[1].block_margin = block_margin;
            let mut needs = [true, true];
            let mut last: [Option<(tetrisai_core::Placement, Vec<tetrisai_core::Piece>)>; 2] = [None, None];
            let mut eps: [Option<Episode>; 2] = [None, None];
            let mut local = Totals::default();
            for _ in 0..max_pieces {
                for s in 0..2 {
                    if needs[s] {
                        let bag = players[s].bag_state(&mut seq);
                        let bot: &mut dyn Agent = if s == 0 { &mut a } else { &mut b };
                        bot.start(&players[s], &bag).unwrap();
                        needs[s] = false;
                    } else if let Some((pl, new)) = &last[s] {
                        let bot: &mut dyn Agent = if s == 0 { &mut a } else { &mut b };
                        bot.play(pl, new).unwrap();
                    }
                    let incoming: Vec<_> = players[s].incoming.iter().copied().collect();
                    let view = BattleView { incoming: &incoming, gauge: players[s].gauge, opponent: &players[1 - s] };
                    let bot: &mut dyn Agent = if s == 0 { &mut a } else { &mut b };
                    bot.update_battle(&view).unwrap();
                }
                let mut sug = [None, None];
                for s in 0..2 {
                    let bot: &mut dyn Agent = if s == 0 { &mut a } else { &mut b };
                    sug[s] = bot.suggest().unwrap();
                }
                let mut sends = [0, 0];
                for s in 0..2 {
                    let Some(p) = sug[s] else {
                        players[s].loss = Some(Loss::NoMove);
                        continue;
                    };
                    let rows_before = shape::center_well_rows(&players[s].state.board);
                    let strategy = if s == 0 { a.strategy_now() } else { b.strategy_now() };
                    local.moves_by_strategy[strategy as usize] += 1;
                    if strategy == Strategy::Rush && rows_before >= 3 {
                        local.rush_with_well += 1;
                    }
                    let lines_before = players[s].stats.lines;
                    // REN の途中で消さない手を選んだ時に、消せる手があったかを調べるため、置く前の局面を覚えておく
                    let board_before = players[s].state.board;
                    let combo_before = players[s].state.consecutive_clears;
                    let usable = {
                        let q = &players[s].queue;
                        let mut v = vec![q[0]];
                        match players[s].hold {
                            Some(h) => v.push(h),
                            None => v.extend(q.get(1).copied()),
                        }
                        v
                    };
                    let Ok(out) = players[s].place(&p, &rules, &mut seq) else { continue };
                    sends[s] = out.send;
                    restart_rng ^= restart_rng << 13;
                    restart_rng ^= restart_rng >> 7;
                    restart_rng ^= restart_rng << 17;
                    needs[s] = out.garbage_rose || out.blocked || restart_rng % 100 < restart_pct;
                    let q = &players[s].queue;
                    let new = q.iter().skip(q.len() - out.revealed).copied().collect::<Vec<_>>();
                    last[s] = Some((p, new));
                    local.pieces += 1;
                    let lines = players[s].stats.lines - lines_before;
                    if lines == 0 && combo_before > 0 && rows_before >= 2 {
                        let spawn = rules.spawn();
                        let can_clear = usable.iter().any(|&piece| {
                            tetrisai_core::generate(&board_before, piece, spawn).iter().any(|m| {
                                let mut b = board_before;
                                b.place(&m.placement.location);
                                b.full_lines() != 0
                            })
                        });
                        local.cut_with_choice += can_clear as u64;
                        if can_clear {
                            let i = (rows_before.min(9) as usize, combo_before.min(9) as usize);
                            local.cut_table[i.0][i.1] += 1;
                            if local.cut_samples.len() < 2 && rows_before >= 3 {
                                local.cut_samples.push(format!(
                                    "REN{} 中あけ{}行 使えたミノ{:?} 予告{} 選んだ {:?} {:?} x{} y{} {:?}
{}
→
{}",
                                    combo_before, rows_before, usable, players[s].incoming_lines(), p.location.piece, p.location.rotation, p.location.x, p.location.y, p.spin, draw(&board_before), draw(&players[s].state.board)
                                ));
                            }
                        }
                        local.cut_forced += !can_clear as u64;
                    }
                    let cells = p.location.cells();
                    if lines == 0 && cells.iter().all(|&(x, y)| (3..=6).contains(&x) && y >= 8) {
                        local.high_center += 1;
                    }
                    if eps[s].is_none() && rows_before >= min_rows {
                        eps[s] = Some(Episode::default());
                    }
                    let rows_after = shape::center_well_rows(&players[s].state.board);
                    let mut done = false;
                    if let Some(ep) = eps[s].as_mut() {
                        ep.peak = ep.peak.max(rows_before);
                        let center = p.location.cells().iter().any(|&(x, _)| (3..=6).contains(&x));
                        if lines > 0 {
                            ep.fires += 1;
                            ep.chain += 1;
                            ep.best_chain = ep.best_chain.max(ep.chain);
                        } else {
                            if ep.chain > 0 && rows_before >= 2 {
                                ep.breaks += 1;
                            }
                            ep.chain = 0;
                            if center {
                                ep.center_no_clear += 1;
                            } else {
                                ep.side_no_clear += 1;
                            }
                        }
                        if p.spin != Spin::None && lines > 0 {
                            ep.tspins += 1;
                        }
                        if keep > 0 {
                            ep.trace.push(format!(
                                "--- {:?} {:?} x{} y{} 消{} 中あけ{}行\n{}",
                                p.location.piece,
                                p.location.rotation,
                                p.location.x,
                                p.location.y,
                                lines,
                                rows_before,
                                draw(&players[s].state.board)
                            ));
                        }
                        done = rows_after == 0 && ep.chain == 0;
                    }
                    if done {
                        let ep = eps[s].take().unwrap();
                        finish(&ep, &mut local, keep, wreck_center);
                    }
                }
                for s in 0..2 {
                    players[1 - s].receive(sends[s], &rules);
                }
                if players.iter().any(|p| p.loss.is_some()) {
                    break;
                }
            }
            for s in 0..2 {
                if let Some(mut ep) = eps[s].take() {
                    ep.lost = players[s].loss.is_some();
                    finish(&ep, &mut local, keep, wreck_center);
                }
            }
            let mut t = totals.lock().unwrap();
            t.games += 1;
            t.pieces += local.pieces;
            t.cut_with_choice += local.cut_with_choice;
            for x in 0..10 {
                for y in 0..10 {
                    t.cut_table[x][y] += local.cut_table[x][y];
                }
            }
            t.cut_samples.extend(local.cut_samples.iter().cloned());
            t.cut_forced += local.cut_forced;
            for k in 0..3 {
                t.moves_by_strategy[k] += local.moves_by_strategy[k];
            }
            t.rush_with_well += local.rush_with_well;
            t.blocked += (players[0].stats.blocked + players[1].stats.blocked) as u64;
            t.wins_len += u64::from(players[0].stats.pieces);
            t.a_wins += (players[1].loss.is_some() && players[0].loss.is_none()) as u64;
            t.b_wins += (players[0].loss.is_some() && players[1].loss.is_none()) as u64;
            t.episodes += local.episodes;
            t.peak += local.peak;
            t.best_chain += local.best_chain;
            t.fires += local.fires;
            t.center += local.center;
            t.side += local.side;
            t.breaks += local.breaks;
            t.tspins += local.tspins;
            t.lost += local.lost;
            t.good += local.good;
            t.wrecked += local.wrecked;
            t.high_center += local.high_center;
            for k in 0..8 {
                t.chain_hist[k] += local.chain_hist[k];
            }
            let room = keep.saturating_sub(t.samples.len());
            t.samples.extend(local.samples.into_iter().take(room));
        }));
    }
    for h in handles {
        h.join().unwrap();
    }
    let t = totals.lock().unwrap();
    let e = t.episodes.max(1) as f64;
    println!(
        "対局 {} 手 {} A勝 {} B勝 {} / C4W {}回({:.1}回/100手)",
        t.games,
        t.pieces,
        t.a_wins,
        t.b_wins,
        t.episodes,
        t.episodes as f64 * 100.0 / t.pieces.max(1) as f64
    );
    println!(
        "平均: 貯めた {:.1}行 / 一番長い REN {:.1} / 消した手 {:.1} / 消さず中央 {:.1} / 消さず左右 {:.1} / REN切れ {:.2} / Tスピン {:.2}",
        t.peak as f64 / e,
        t.best_chain as f64 / e,
        t.fires as f64 / e,
        t.center as f64 / e,
        t.side as f64 / e,
        t.breaks as f64 / e,
        t.tspins as f64 / e
    );
    println!(
        "発火できた(7割以上) {:.1}% / 崩した(中央に3回以上・半分未満) {:.1}% / C4W の途中で負け {:.1}%",
        t.good as f64 * 100.0 / e,
        t.wrecked as f64 * 100.0 / e,
        t.lost as f64 * 100.0 / e
    );
    println!(
        "消さず高い中央 {:.2}回/100手 / 壁に止められた {:.2}回/100手 / 1局の手数 {:.0}",
        t.high_center as f64 * 100.0 / t.pieces.max(1) as f64,
        t.blocked as f64 * 100.0 / t.pieces.max(1) as f64,
        t.wins_len as f64 / t.games.max(1) as f64
    );
    println!("作戦の手数 通常/対抗/早撃ち {:?} / 自分に中あけ3行以上あるのに早撃ち {}", t.moves_by_strategy, t.rush_with_well);
    println!("REN の途中で切った手(中あけ2行以上): 消せたのに切った {} / 消せなかった {}", t.cut_with_choice, t.cut_forced);
    println!("消せたのに切った手の表(行=中あけの行数 2〜9、列=REN 0〜9): ");
    for x in 2..10 {
        println!("  {x}行 {:?}", t.cut_table[x]);
    }
    if keep > 0 {
        for c in t.cut_samples.iter().take(keep) {
            println!("##### {c}");
        }
    }
    println!("一番長い REN の分布(2刻み): {:?}", t.chain_hist);
    for (peak, s) in &t.samples {
        println!("===== 崩した例 貯めた {peak}行\n{s}");
    }
}
