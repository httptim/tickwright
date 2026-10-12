//! Block positions and directions, with the game's iteration orders.
//!
//! The order in which the game visits neighbors decides the order of every
//! update it sends, so each order the game uses is spelled out here rather
//! than derived.

use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Pos {
    pub x: i32,
    pub y: i32,
    pub z: i32,
}

impl Pos {
    pub const fn new(x: i32, y: i32, z: i32) -> Self {
        Pos { x, y, z }
    }

    pub fn offset(self, dir: Dir) -> Pos {
        let [dx, dy, dz] = dir.step();
        Pos::new(self.x + dx, self.y + dy, self.z + dz)
    }

    pub fn above(self) -> Pos {
        self.offset(Dir::Up)
    }

    pub fn below(self) -> Pos {
        self.offset(Dir::Down)
    }

    /// `Vec3i.hashCode`, which decides `HashSet<BlockPos>` iteration order.
    pub fn java_hash(self) -> i32 {
        self.y.wrapping_add(self.z.wrapping_mul(31)).wrapping_mul(31).wrapping_add(self.x)
    }
}

impl fmt::Debug for Pos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "[{}, {}, {}]", self.x, self.y, self.z)
    }
}

/// `Direction`, declared in the game's order (`Direction.values()`).
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub enum Dir {
    Down,
    Up,
    North,
    South,
    West,
    East,
}

impl Dir {
    /// `Direction.values()`.
    pub const ALL: [Dir; 6] = [Dir::Down, Dir::Up, Dir::North, Dir::South, Dir::West, Dir::East];

    /// `Direction.Plane.HORIZONTAL`: north, east, south, west.
    pub const HORIZONTAL: [Dir; 4] = [Dir::North, Dir::East, Dir::South, Dir::West];

    pub fn step(self) -> [i32; 3] {
        match self {
            Dir::Down => [0, -1, 0],
            Dir::Up => [0, 1, 0],
            Dir::North => [0, 0, -1],
            Dir::South => [0, 0, 1],
            Dir::West => [-1, 0, 0],
            Dir::East => [1, 0, 0],
        }
    }

    pub fn opposite(self) -> Dir {
        match self {
            Dir::Down => Dir::Up,
            Dir::Up => Dir::Down,
            Dir::North => Dir::South,
            Dir::South => Dir::North,
            Dir::West => Dir::East,
            Dir::East => Dir::West,
        }
    }

    pub fn is_horizontal(self) -> bool {
        !matches!(self, Dir::Down | Dir::Up)
    }

    /// Clockwise when seen from above.
    pub fn clockwise(self) -> Dir {
        match self {
            Dir::North => Dir::East,
            Dir::East => Dir::South,
            Dir::South => Dir::West,
            Dir::West => Dir::North,
            d => d,
        }
    }

    pub fn counter_clockwise(self) -> Dir {
        match self {
            Dir::North => Dir::West,
            Dir::West => Dir::South,
            Dir::South => Dir::East,
            Dir::East => Dir::North,
            d => d,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Dir::Down => "down",
            Dir::Up => "up",
            Dir::North => "north",
            Dir::South => "south",
            Dir::West => "west",
            Dir::East => "east",
        }
    }

    pub fn from_name(s: &str) -> Option<Dir> {
        Dir::ALL.into_iter().find(|d| d.name() == s)
    }
}

/// `NeighborUpdater.UPDATE_ORDER`: the order of neighbor-changed updates.
pub const UPDATE_ORDER: [Dir; 6] = [Dir::West, Dir::East, Dir::Down, Dir::Up, Dir::North, Dir::South];

/// `BlockBehaviour.UPDATE_SHAPE_ORDER`: the order of shape updates.
pub const UPDATE_SHAPE_ORDER: [Dir; 6] = [Dir::West, Dir::East, Dir::North, Dir::South, Dir::Down, Dir::Up];

/// Iteration order of a default-capacity `java.util.HashSet<BlockPos>`
/// holding `positions`, inserted in the given order.
///
/// A `HashSet` with the default 16 buckets visits buckets in index order and
/// each bucket in insertion order. The bucket is the spread hash
/// (`h ^ (h >>> 16)`) masked to the table size. Correct for up to 12
/// elements, the point where the table would resize.
pub fn java_hash_set_order(positions: &[Pos]) -> Vec<Pos> {
    assert!(positions.len() <= 12, "the HashSet would have resized");
    let mut buckets: [Vec<Pos>; 16] = Default::default();
    for &p in positions {
        let h = p.java_hash();
        let spread = h ^ ((h as u32) >> 16) as i32;
        let bucket = &mut buckets[(spread & 15) as usize];
        if !bucket.contains(&p) {
            bucket.push(p);
        }
    }
    buckets.into_iter().flatten().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn java_hash_matches_vec3i() {
        // (y + z * 31) * 31 + x
        assert_eq!(Pos::new(1, 2, 3).java_hash(), (2 + 3 * 31) * 31 + 1);
        assert_eq!(Pos::new(-5, 100, -7).java_hash(), (100 + -7 * 31) * 31 - 5);
    }

    #[test]
    fn hash_set_order_depends_on_position() {
        let around = |p: Pos| {
            let mut v = vec![p];
            v.extend(Dir::ALL.iter().map(|d| p.offset(*d)));
            v
        };
        let a = java_hash_set_order(&around(Pos::new(0, 100, 0)));
        assert_eq!(a.len(), 7);
        // Same shape, different relative order somewhere: the reason dust is locational.
        let rel =
            |v: &[Pos], o: Pos| -> Vec<[i32; 3]> { v.iter().map(|p| [p.x - o.x, p.y - o.y, p.z - o.z]).collect() };
        let base = rel(&a, Pos::new(0, 100, 0));
        assert!((0..16).any(|x| {
            let o = Pos::new(x, 100, 0);
            rel(&java_hash_set_order(&around(o)), o) != base
        }));
    }
}
