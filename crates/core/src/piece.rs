//! ミノの種類・向き・配置位置。
//!
//! 座標系は TBP(Tetris Bot Protocol)と Cold Clear 2 に合わせる。
//! - x は左から 0..10、y は下から 0..40(y=0 が最下段)
//! - 配置位置 (x, y) はミノの「中心」で、向きごとの4マスはその中心からの相対位置で決まる
//!
//! こうしておくと、TBA の既存クライアント(`cold_clear_client.py`)の座標変換をそのまま使える。

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum Piece {
    I,
    O,
    T,
    L,
    J,
    S,
    Z,
}

impl Piece {
    pub const ALL: [Piece; 7] = [Piece::I, Piece::O, Piece::T, Piece::L, Piece::J, Piece::S, Piece::Z];

    /// 北向き(出現時の向き)での、中心から見た4マスの相対位置。Cold Clear 2 の data.rs と同じ。
    pub const fn cells(self) -> [(i8, i8); 4] {
        match self {
            Piece::I => [(-1, 0), (0, 0), (1, 0), (2, 0)],
            Piece::O => [(0, 0), (1, 0), (0, 1), (1, 1)],
            Piece::T => [(-1, 0), (0, 0), (1, 0), (0, 1)],
            Piece::L => [(-1, 0), (0, 0), (1, 0), (1, 1)],
            Piece::J => [(-1, 0), (0, 0), (1, 0), (-1, 1)],
            Piece::S => [(-1, 0), (0, 0), (0, 1), (1, 1)],
            Piece::Z => [(-1, 1), (0, 1), (0, 0), (1, 0)],
        }
    }

    pub fn from_char(c: char) -> Option<Piece> {
        Some(match c.to_ascii_uppercase() {
            'I' => Piece::I,
            'O' => Piece::O,
            'T' => Piece::T,
            'L' => Piece::L,
            'J' => Piece::J,
            'S' => Piece::S,
            'Z' => Piece::Z,
            _ => return None,
        })
    }

    pub const fn to_char(self) -> char {
        match self {
            Piece::I => 'I',
            Piece::O => 'O',
            Piece::T => 'T',
            Piece::L => 'L',
            Piece::J => 'J',
            Piece::S => 'S',
            Piece::Z => 'Z',
        }
    }
}

/// 向き。並びは北から時計回り(添字として使う)。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rotation {
    North,
    East,
    South,
    West,
}

impl Rotation {
    pub const ALL: [Rotation; 4] = [Rotation::North, Rotation::East, Rotation::South, Rotation::West];

    /// 中心を軸に、北向きの相対位置をこの向きへ回す。Cold Clear 2 の rotate_cell と同じ。
    pub const fn rotate_cell(self, (x, y): (i8, i8)) -> (i8, i8) {
        match self {
            Rotation::North => (x, y),
            Rotation::East => (y, -x),
            Rotation::South => (-x, -y),
            Rotation::West => (-y, x),
        }
    }

    pub const fn cw(self) -> Rotation {
        match self {
            Rotation::North => Rotation::East,
            Rotation::East => Rotation::South,
            Rotation::South => Rotation::West,
            Rotation::West => Rotation::North,
        }
    }

    pub const fn ccw(self) -> Rotation {
        match self {
            Rotation::North => Rotation::West,
            Rotation::East => Rotation::North,
            Rotation::South => Rotation::East,
            Rotation::West => Rotation::South,
        }
    }

    pub const fn index(self) -> usize {
        self as usize
    }
}

/// ミノの種類・向き・中心座標。TBP の `location` と同じ形。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct PieceLocation {
    #[serde(rename = "type")]
    pub piece: Piece,
    #[serde(rename = "orientation")]
    pub rotation: Rotation,
    pub x: i8,
    pub y: i8,
}

impl PieceLocation {
    pub const fn new(piece: Piece, rotation: Rotation, x: i8, y: i8) -> Self {
        PieceLocation { piece, rotation, x, y }
    }

    /// 盤面上の4マス(x, y)。
    pub fn cells(&self) -> [(i8, i8); 4] {
        self.piece.cells().map(|c| {
            let (dx, dy) = self.rotation.rotate_cell(c);
            (self.x + dx, self.y + dy)
        })
    }

    /// 4マスを (y, x) の昇順に並べたもの。配置の同一性の比較に使う。
    pub fn sorted_cells(&self) -> [(i8, i8); 4] {
        let mut cells = self.cells();
        cells.sort_by_key(|&(x, y)| (y, x));
        cells
    }

    /// 同じ4マスを占める配置の代表形。O・S・Z・I は向きが違っても同じ4マスになる組があるため、
    /// Cold Clear 2 の canonical_form と同じ規則で1つにそろえる(TBP で返す配置もこの形)。
    pub const fn canonical_form(&self) -> PieceLocation {
        let s = *self;
        match self.piece {
            Piece::T | Piece::J | Piece::L => s,
            Piece::O => match self.rotation {
                Rotation::North => s,
                Rotation::East => PieceLocation { rotation: Rotation::North, y: s.y - 1, ..s },
                Rotation::South => PieceLocation { rotation: Rotation::North, x: s.x - 1, y: s.y - 1, ..s },
                Rotation::West => PieceLocation { rotation: Rotation::North, x: s.x - 1, ..s },
            },
            Piece::S | Piece::Z => match self.rotation {
                Rotation::North | Rotation::East => s,
                Rotation::South => PieceLocation { rotation: Rotation::North, y: s.y - 1, ..s },
                Rotation::West => PieceLocation { rotation: Rotation::East, x: s.x - 1, ..s },
            },
            Piece::I => match self.rotation {
                Rotation::North | Rotation::East => s,
                Rotation::South => PieceLocation { rotation: Rotation::North, x: s.x - 1, ..s },
                Rotation::West => PieceLocation { rotation: Rotation::East, y: s.y + 1, ..s },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn 代表形は同じ4マスを占める() {
        for piece in Piece::ALL {
            for rotation in Rotation::ALL {
                let loc = PieceLocation::new(piece, rotation, 4, 10);
                assert_eq!(loc.sorted_cells(), loc.canonical_form().sorted_cells(), "{piece:?} {rotation:?}");
            }
        }
    }

    #[test]
    fn 回転は4回で元に戻る() {
        for r in Rotation::ALL {
            assert_eq!(r.cw().cw().cw().cw(), r);
            assert_eq!(r.cw().ccw(), r);
        }
    }
}
