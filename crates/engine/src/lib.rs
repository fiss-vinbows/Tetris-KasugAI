//! TetrisAI の思考部分: 評価関数と探索。

pub mod aim;
pub mod eval;
pub mod opening;
pub mod pco;
pub mod search;
pub mod shape;
pub mod strategy;

pub use eval::{BattleContext, EvalContext, Garbage, Weights};
pub use opening::{split_style, Opening, OpeningChoice, OpeningMode, OpeningTracker};
pub use strategy::{Strategy, StrategyTracker};
pub use search::{search, Candidate, IncomingGarbage, Opponent, Root, SearchOptions, SearchResult};
