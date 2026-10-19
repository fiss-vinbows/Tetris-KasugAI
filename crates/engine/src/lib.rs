//! TetrisAI の思考部分: 評価関数と探索。

pub mod eval;
pub mod search;

pub use eval::{Garbage, Weights};
pub use search::{search, Candidate, IncomingGarbage, Root, SearchOptions, SearchResult};
