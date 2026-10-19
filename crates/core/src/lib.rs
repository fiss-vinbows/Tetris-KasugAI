//! TetrisAI の土台: 盤面・ミノ・SRS・手の生成・ルール。
//!
//! 座標系は TBP / Cold Clear 2 と同じ(x は左から、y は下から、配置はミノの中心)。

pub mod board;
pub mod movegen;
pub mod piece;
pub mod rules;
pub mod srs;

pub use board::Board;
pub use movegen::{generate, generate_into, Move, Placement, Spawn};
pub use piece::{Piece, PieceLocation, Rotation};
pub use rules::{ClearResult, PlayerState, Rules};
pub use srs::Spin;
