//! TetrisAI の思考部分: 評価関数と探索。

pub mod eval;
pub mod search;
pub mod shape;

pub use eval::{BattleContext, EvalContext, Garbage, Weights};
pub use search::{search, Candidate, IncomingGarbage, Opponent, Root, SearchOptions, SearchResult};
