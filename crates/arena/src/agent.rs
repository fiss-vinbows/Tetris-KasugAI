//! 対局する側(エージェント)。TBP の bot プロセスと、同じプロセス内で探索するこのAIの2種類。

use std::io;

use tetrisai_core::{Piece, Placement, PlayerState, Rules};
use tetrisai_engine::{search, IncomingGarbage, OpeningTracker, Opponent, Root, SearchOptions, Weights};

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
            root: None,
        }
    }
}

impl EngineAgent {
    /// 局面が変わったら、序盤の作戦を選ぶ・進める・終える。
    fn observe_opening(&mut self) {
        if let (Some(tracker), Some(root)) = (self.opening.as_mut(), self.root.as_mut()) {
            root.opening = tracker.observe(root, &self.rules);
        }
    }
}

impl Agent for EngineAgent {
    fn start(&mut self, player: &Player, bag_state: &[Piece]) -> io::Result<()> {
        // 袋の切れ目(7個まるごと残っている)は「次は新しい袋」なので空にする
        let bag = if bag_state.len() == 7 { Vec::new() } else { bag_state.to_vec() };
        self.root = Some(Root {
            state: PlayerState { ..player.state },
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
            root.incoming = view.incoming.iter().map(|g| IncomingGarbage { lines: g.lines, delay: g.delay }).collect();
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
