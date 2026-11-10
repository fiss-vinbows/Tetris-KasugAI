//! 対局する側(エージェント)。TBP の bot プロセスと、同じプロセス内で探索するこのAIの2種類。

use std::io;

use tetrisai_core::{Piece, Placement, PlayerState, Rules};
use tetrisai_engine::{search, IncomingGarbage, OpeningTracker, Opponent, Root, SearchOptions, StrategyTracker, Weights};

use crate::battle::{Incoming, Player};

/// 毎手、考える前に渡す対戦の状況。
pub struct BattleView<'a> {
    /// 自分への予告
    pub incoming: &'a [Incoming],
    /// 自分の攻撃ゲージ
    pub gauge: u32,
    /// 相手
    pub opponent: &'a Player,
}
use crate::tbp::TbpBot;

pub trait Agent {
    /// 局面を渡し直す(対局の最初と、おじゃまがせり上がった後)。
    fn start(&mut self, player: &Player, bag_state: &[Piece]) -> io::Result<()>;
    /// 前回の提案どおりに指したことと、新しく見えたミノを伝える。
    fn play(&mut self, placement: &Placement, new_pieces: &[Piece]) -> io::Result<()>;
    /// 対戦の状況(予告・攻撃ゲージ・相手)を伝える(毎手、考える前に呼ぶ)。
    fn update_battle(&mut self, _view: &BattleView) -> io::Result<()> {
        Ok(())
    }
    /// 今の最善手。まだ無ければ None。
    fn suggest(&mut self) -> io::Result<Option<Placement>>;
    /// 別スレッドで考え続ける(`suggest` の前に思考時間を待つ必要がある)か。
    fn thinks_in_background(&self) -> bool;
    /// 対局の終わり。
    fn stop(&mut self) -> io::Result<()> {
        Ok(())
    }
    fn name(&self) -> String;
    /// 今の序盤のテンプレの名前(記録用。2026-11-08)
    fn template_name(&self) -> Option<String> {
        None
    }
}

impl Agent for TbpBot {
    fn start(&mut self, player: &Player, bag_state: &[Piece]) -> io::Result<()> {
        TbpBot::start(self, player, bag_state)
    }
    fn play(&mut self, placement: &Placement, new_pieces: &[Piece]) -> io::Result<()> {
        TbpBot::play(self, placement, new_pieces)
    }
    fn update_battle(&mut self, view: &BattleView) -> io::Result<()> {
        TbpBot::battle(self, view)
    }
    fn suggest(&mut self) -> io::Result<Option<Placement>> {
        TbpBot::suggest(self)
    }
    fn thinks_in_background(&self) -> bool {
        true
    }
    fn stop(&mut self) -> io::Result<()> {
        TbpBot::stop(self)
    }
    fn name(&self) -> String {
        self.name.clone()
    }
}

/// このAIの探索を同じプロセス内で呼ぶ。思考時間ではなく探索の幅を固定するので、速く・再現性よく回せる
/// (重みの自動調整に使う)。
pub struct EngineAgent {
    pub weights: Weights,
    pub rules: Rules,
    pub width: usize,
    pub label: String,
    /// 予告を探索で考えるか(false なら予告を知らないものとして読む。比較実験用)
    pub use_incoming: bool,
    /// 相手の情報を探索で考えるか(false なら相手を見ずに読む。比較実験用)
    pub use_opponent: bool,
    /// 見えているミノの先を1手だけ予測して読むか
    pub speculate: bool,
    /// 序盤の作戦(TD系テンプレか素の状態をランダムで選ぶ)。None なら使わない
    pub opening: Option<OpeningTracker>,
    /// start で REN と B2B を伝えない(実機の TBA の古い動きを再現する実験用。2026-11-07)
    pub forget_on_start: bool,
    /// 予告の時間差を伝えない(実機の TBA は delay 0 で送る)
    pub incoming_without_delay: bool,
    /// 予告に気づくまでの遅れ(自分の手の数)。【2026-11-10・実機 debug_log_20261110_185033】実機の C4W 側は TST の6行を
    /// 受けてから2〜3手後に REN を始めた(対戦シミュレーターでは次の手)。メモリで読んだ予告を bot に伝えるまでの遅れを再現する
    pub incoming_lag: usize,
    incoming_history: std::collections::VecDeque<Vec<IncomingGarbage>>,
    /// 作戦層(相手の情報を使うとき、相手の C4W を見て作戦を選ぶ)
    strategy: StrategyTracker,
    root: Option<Root>,
}

impl EngineAgent {
    pub fn new(weights: Weights, rules: Rules, width: usize, label: &str) -> Self {
        EngineAgent {
            weights,
            rules,
            width,
            label: label.to_string(),
            use_incoming: true,
            use_opponent: true,
            speculate: false,
            opening: None,
            forget_on_start: false,
            incoming_without_delay: false,
            incoming_lag: 0,
            incoming_history: std::collections::VecDeque::new(),
            strategy: StrategyTracker::default(),
            root: None,
        }
    }
}

impl EngineAgent {
    /// 今の作戦(道具で数えるため)。
    pub fn strategy_now(&self) -> tetrisai_engine::Strategy {
        self.strategy.current()
    }

    /// 局面が変わったら、序盤の作戦を選ぶ・進める・終える。
    fn observe_opening(&mut self) {
        if let (Some(tracker), Some(root)) = (self.opening.as_mut(), self.root.as_mut()) {
            root.opening = tracker.observe(root, &self.rules);
            root.opening_well = tracker.well_now();
        }
    }
}

impl Agent for EngineAgent {
    fn template_name(&self) -> Option<String> {
        self.opening.as_ref()?.template_name().map(String::from)
    }

    fn start(&mut self, player: &Player, bag_state: &[Piece]) -> io::Result<()> {
        // 袋の切れ目(7個まるごと残っている)は「次は新しい袋」なので空にする
        let bag = if bag_state.len() == 7 { Vec::new() } else { bag_state.to_vec() };
        self.root = Some(Root {
            state: if self.forget_on_start {
                PlayerState { board: player.state.board, back_to_back: false, consecutive_clears: 0 }
            } else {
                PlayerState { ..player.state }
            },
            hold: player.hold,
            queue: player.queue.iter().copied().collect(),
            incoming: Vec::new(),
            bag,
            ..Default::default()
        });
        self.observe_opening();
        Ok(())
    }

    fn update_battle(&mut self, view: &BattleView) -> io::Result<()> {
        let Some(root) = self.root.as_mut() else { return Ok(()) };
        if self.use_incoming {
            let now: Vec<IncomingGarbage> =
                view.incoming.iter().map(|g| IncomingGarbage { lines: g.lines, delay: if self.incoming_without_delay { 0 } else { g.delay } }).collect();
            if self.incoming_lag == 0 {
                root.incoming = now;
            } else {
                // 遅れの分だけ前の手の時点の予告を見せる
                self.incoming_history.push_back(now);
                while self.incoming_history.len() > self.incoming_lag + 1 {
                    self.incoming_history.pop_front();
                }
                root.incoming = if self.incoming_history.len() > self.incoming_lag { self.incoming_history.front().cloned().unwrap_or_default() } else { Vec::new() };
            }
        }
        if self.use_opponent {
            root.gauge = view.gauge;
            let o = view.opponent;
            root.opponent = Some(Opponent {
                board: o.state.board,
                incoming: o.incoming_lines(),
                gauge: o.gauge,
                back_to_back: o.state.back_to_back,
            });
            root.strategy = self.strategy.observe(&root.state.board, Some(&o.state.board)).forced(self.weights.c4w_response);
            // 【2026-11-10】速攻の相手(テンプレの途中で攻撃された)には、TSD の後 C4W に移る
            if self.opening.as_ref().is_some_and(|t| t.force_rival()) && root.strategy == tetrisai_engine::Strategy::Normal {
                root.strategy = tetrisai_engine::Strategy::C4WRival;
            }
            if self.opening.as_ref().is_some_and(|t| t.suppress_rival()) && root.strategy != tetrisai_engine::Strategy::Normal {
                root.strategy = tetrisai_engine::Strategy::Normal;
            }
            if let Some(tracker) = self.opening.as_mut() {
                if tracker.pivot_if_opponent_c4w(&o.state.board, self.weights.pivot_opening_rows) {
                    root.opening = tetrisai_engine::Opening::None;
                }
            }
        }
        Ok(())
    }

    fn play(&mut self, placement: &Placement, new_pieces: &[Piece]) -> io::Result<()> {
        if let Some(root) = self.root.as_mut() {
            root.play(placement, &self.rules);
            for &p in new_pieces {
                root.push_piece(p);
            }
        }
        self.observe_opening();
        Ok(())
    }

    fn suggest(&mut self) -> io::Result<Option<Placement>> {
        let Some(root) = &self.root else { return Ok(None) };
        let options = SearchOptions { width: self.width, max_depth: 6, speculate: self.speculate };
        let result = search(root, &options, &self.weights, &self.rules, &|| false);
        Ok(result.and_then(|r| r.candidates.first().map(|c| c.mv.placement)))
    }

    fn thinks_in_background(&self) -> bool {
        false
    }

    fn name(&self) -> String {
        self.label.clone()
    }
}
