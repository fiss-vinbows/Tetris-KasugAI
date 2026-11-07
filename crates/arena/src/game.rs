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
            if let Ok(outcome) = players[i].place(&placement, rules, &mut sequence) {
                sends[i] = outcome.send;
                clock[i] += outcome.move_ms;
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
    let [p0, p1] = players;
    Ok(GameResult { seed, winner, losses, stats: [p0.stats, p1.stats], adjudicated })
}
