//! 1局の進行: 両者が同時に1手ずつ置く。
//!
//! 毎手、両方の bot に同じ時間だけ考えさせてから最善手を聞き、同時に置く。生まれた攻撃は
//! 置いた後で相手の予告に入る(同じ手番の攻撃は、互いに次の手番以降に効く)。
//! bot には、おじゃまがせり上がった後だけ `start` で局面を渡し直し、それ以外は `play`/`new_piece` で
//! 探索を引き継がせる(Cold Clear 2 は探索木を引き継ぐほど強くなるため、毎手 start すると不利になる)。

use std::io;
use std::thread::sleep;
use std::time::Duration;

use tetrisai_core::{Piece, Placement, Rules};

use crate::agent::{Agent, BattleView};
use crate::battle::{Loss, PieceSequence, Player, Stats};

#[derive(Clone, Debug)]
pub struct GameConfig {
    pub rules: Rules,
    /// 1手あたりの思考時間
    pub think: Duration,
    /// 1人あたりの手数の上限(超えたら引き分け。`adjudicate` なら判定)
    pub max_pieces: u32,
    /// 手数の上限に達したら、危なさ(盤面の高さ + 残っている予告の行数)の小さいほうを勝ちにする。
    /// 自動調整で引き分けを減らすために使う(引き分けからは勝敗の手がかりが得られないため)
    pub adjudicate: bool,
    /// 実機の「壁に止められる」を再現する行数(`Player::block_margin`)。0 なら再現しない
    pub block_margin: u32,
    /// 手にかかる実機の時間を数え、遅れている側だけが置く(2026-11-07。ソフトドロップ・消去の演出が遅いことを再現する)
    pub timed: bool,
    /// timed のとき、手の時間を何%にするか(A, B)。人の相手の遅さを再現する(2026-11-09)
    pub time_pct: [u32; 2],
}

#[derive(Clone, Debug)]
pub struct GameResult {
    pub seed: u64,
    /// 勝った側(0 か 1)。引き分けは None
    pub winner: Option<usize>,
    pub losses: [Option<Loss>; 2],
    pub stats: [Stats; 2],
    /// 手数の上限で判定して勝敗を決めたか
    pub adjudicated: bool,
}

/// 提案がまだ無いときに聞き直す回数と間隔
const SUGGEST_RETRIES: u32 = 20;
const SUGGEST_RETRY_WAIT: Duration = Duration::from_millis(10);

/// 調べもの用: 環境変数 ARENA_TRACE に対局の種を入れるとその局だけ、`all` なら全局の1手ずつを標準エラーに出す
fn traced(seed: u64) -> bool {
    static SEED: std::sync::OnceLock<Option<Option<u64>>> = std::sync::OnceLock::new();
    match SEED.get_or_init(|| {
        std::env::var("ARENA_TRACE").ok().map(|s| if s == "all" { None } else { s.parse().ok() })
    }) {
        Some(None) => true,
        Some(Some(s)) => *s == seed,
        None => false,
    }
}

pub fn play_game(bots: [&mut dyn Agent; 2], seed: u64, config: &GameConfig) -> io::Result<GameResult> {
    let rules = &config.rules;
    let mut sequence = PieceSequence::new(seed);
    // おじゃまの穴の位置も両者で同じ乱数の列にする
    let hole_seed = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xA5A5;
    let mut players = [Player::new(&mut sequence, hole_seed), Player::new(&mut sequence, hole_seed)];
    players[0].block_margin = config.block_margin;
    players[1].block_margin = config.block_margin;
    let mut needs_start = [true, true];
    let mut last: [Option<(Placement, Vec<Piece>)>; 2] = [None, None];

    let [a, b] = bots;
    let mut bots = [a, b];

    // 対局の開始からの時間(ms)。timed でなければ両者とも毎回置く
    let mut clock = [0u32; 2];

    for _turn in 0..config.max_pieces {
        let active = [!config.timed || clock[0] <= clock[1], !config.timed || clock[1] <= clock[0]];
        for i in 0..2 {
            if !active[i] {
                continue;
            }
            if needs_start[i] {
                let bag_state = players[i].bag_state(&mut sequence);
                bots[i].start(&players[i], &bag_state)?;
                needs_start[i] = false;
            } else if let Some((placement, new_pieces)) = &last[i] {
                bots[i].play(placement, new_pieces)?;
            }
            last[i] = None;
            let incoming: Vec<_> = players[i].incoming.iter().copied().collect();
            let view = BattleView { incoming: &incoming, gauge: players[i].gauge, opponent: &players[1 - i] };
            bots[i].update_battle(&view)?;
        }
        if bots.iter().any(|b| b.thinks_in_background()) {
            sleep(config.think);
        }

        let mut suggested = [None, None];
        for i in 0..2 {
            if !active[i] {
                continue;
            }
            for _ in 0..SUGGEST_RETRIES {
                suggested[i] = bots[i].suggest()?;
                if suggested[i].is_some() {
                    break;
                }
                sleep(SUGGEST_RETRY_WAIT);
            }
        }

        let mut sends = [0, 0];
        for i in 0..2 {
            if !active[i] {
                continue;
            }
            let Some(placement) = suggested[i] else {
                let loss = if players[i].has_any_move(rules) { Loss::BotGaveUp } else { Loss::NoMove };
                players[i].loss = Some(loss);
                continue;
            };
            let alive = bots[i].template_name();
            if players[i].stats.template.is_none() {
                players[i].stats.template = alive.clone();
                if let (true, Some(name)) = (traced(seed), &alive) {
                    eprintln!("#{seed} {} テンプレ {name}", ["A", "B"][i]);
                }
            }
            players[i].stats.template_alive.push(alive.is_some());
            let before = (players[i].state.board.max_height(), players[i].incoming_lines(), players[i].stats.received);
            if let Ok(outcome) = players[i].place(&placement, rules, &mut sequence) {
                // 調べもの用: 環境変数 ARENA_TRACE に対局の種を入れると、その局の1手ずつを標準エラーに出す(2026-11-09)
                if traced(seed) {
                    let p = &players[i];
                    eprintln!(
                        "#{seed} {} 手{:>3} 時刻{:>6} 高さ{:>2}→{:>2} 予告{:>2}→{:>2} せり上がり+{:<2} REN{:>2} ゲージ{:>2} 送る{:>2} 中あけ{:>2} 攻撃計{:>3} パフェ{} 出現{}",
                        ["A", "B"][i],
                        p.stats.pieces,
                        clock[i],
                        before.0,
                        p.state.board.max_height(),
                        before.1,
                        p.incoming_lines(),
                        p.stats.received - before.2,
                        p.state.consecutive_clears,
                        p.gauge,
                        outcome.send,
                        tetrisai_engine::shape::ren_rows(&p.state.board),
                        p.stats.attack,
                        p.stats.perfect_clears,
                        tetrisai_engine::shape::spawn_height(&p.state.board),
                    );
                }
                sends[i] = outcome.send;
                // 【調べもの用・2026-11-10】環境変数 ARENA_IRS=A|B|AB: 出現待ちの間に最初の回転を押しておく(先行入力)とみなし、
                // 回転して置く手は1キー(33ms)短くする
                let irs = std::env::var("ARENA_IRS").is_ok_and(|v| v.contains(["A", "B"][i]));
                let ms = if irs && placement.location.rotation != tetrisai_core::Rotation::North {
                    outcome.move_ms.saturating_sub(33)
                } else {
                    outcome.move_ms
                };
                clock[i] += ms * config.time_pct[i] / 100;
                needs_start[i] = outcome.garbage_rose || outcome.blocked;
                let q = &players[i].queue;
                let new_pieces = q.iter().skip(q.len() - outcome.revealed).copied().collect();
                last[i] = Some((placement, new_pieces));
            }
        }
        for i in 0..2 {
            players[1 - i].receive(sends[i], rules);
        }
        if players.iter().any(|p| p.loss.is_some()) {
            break;
        }
    }

    for bot in bots.iter_mut() {
        bot.stop()?;
    }
    let losses = [players[0].loss, players[1].loss];
    let mut adjudicated = false;
    let winner = match losses {
        [None, Some(_)] => Some(0),
        [Some(_), None] => Some(1),
        [None, None] if config.adjudicate => {
            let danger = |p: &Player| p.state.board.max_height() + p.incoming_lines();
            let (d0, d1) = (danger(&players[0]), danger(&players[1]));
            adjudicated = d0 != d1;
            match d0.cmp(&d1) {
                std::cmp::Ordering::Less => Some(0),
                std::cmp::Ordering::Greater => Some(1),
                std::cmp::Ordering::Equal => None,
            }
        }
        _ => None,
    };
    if traced(seed) {
        eprintln!("#{seed} 結果 {:?}", winner);
    }
    let [p0, p1] = players;
    Ok(GameResult { seed, winner, losses, stats: [p0.stats, p1.stats], adjudicated })
}
