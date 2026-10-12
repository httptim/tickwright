//! Rotating and mirroring fixtures.
//!
//! Default redstone dust is locational: the same contraption can behave
//! differently when rotated or mirrored, because the game updates neighbors
//! in an order that depends on world coordinates. Every fixture is therefore
//! captured in all eight orientations, generated here so the capture harness
//! never needs to know how block states rotate.
//!
//! Conventions match the game: north is -z, east is +x, and rotations turn
//! clockwise when seen from above. A mirrored transform first flips the x
//! axis (east and west swap), then rotates.

use std::fmt;
use std::str::FromStr;

use anyhow::{Result, bail};

use crate::state::BlockState;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Transform {
    /// Clockwise quarter turns, 0 to 3.
    pub quarter_turns: u8,
    /// Flip the x axis before rotating.
    pub mirror_x: bool,
}

impl Transform {
    pub const IDENTITY: Transform = Transform { quarter_turns: 0, mirror_x: false };

    pub fn all() -> [Transform; 8] {
        let mut out = [Transform::IDENTITY; 8];
        for (i, t) in out.iter_mut().enumerate() {
            *t = Transform { quarter_turns: (i % 4) as u8, mirror_x: i >= 4 };
        }
        out
    }

    /// The size of a box after this transform.
    pub fn size(self, [sx, sy, sz]: [i32; 3]) -> [i32; 3] {
        if self.quarter_turns % 2 == 1 { [sz, sy, sx] } else { [sx, sy, sz] }
    }

    /// Where a position inside a box of `size` lands, staying inside the
    /// transformed box (so offsets stay non-negative).
    pub fn pos(self, [x, y, z]: [i32; 3], [sx, _sy, sz]: [i32; 3]) -> [i32; 3] {
        let (mut x, mut z, mut w, mut d) = (x, z, sx, sz);
        if self.mirror_x {
            x = w - 1 - x;
        }
        for _ in 0..self.quarter_turns {
            // One clockwise turn: north edge (z = 0) becomes the east edge.
            (x, z) = (d - 1 - z, x);
            (w, d) = (d, w);
        }
        [x, y, z]
    }

    /// Where an exact point inside a box of `size` lands: the continuous
    /// form of [`Transform::pos`], so a block's center stays its center.
    pub fn point(self, [x, y, z]: [f64; 3], [sx, _sy, sz]: [i32; 3]) -> [f64; 3] {
        let (mut x, mut z, mut w, mut d) = (x, z, sx as f64, sz as f64);
        if self.mirror_x {
            x = w - x;
        }
        for _ in 0..self.quarter_turns {
            (x, z) = (d - z, x);
            (w, d) = (d, w);
        }
        [x, y, z]
    }

    /// How a velocity turns with the box.
    pub fn vector(self, [x, y, z]: [f64; 3]) -> [f64; 3] {
        let (mut x, mut z) = (x, z);
        if self.mirror_x {
            x = -x;
        }
        for _ in 0..self.quarter_turns {
            (x, z) = (-z, x);
        }
        [x, y, z]
    }

    /// A point inside one block, in sixteenths from its least corner.
    pub fn sixteenths(self, [x, y, z]: [i32; 3]) -> [i32; 3] {
        let (mut x, mut z) = (x, z);
        if self.mirror_x {
            x = 16 - x;
        }
        for _ in 0..self.quarter_turns {
            (x, z) = (16 - z, x);
        }
        [x, y, z]
    }

    pub fn direction(self, dir: &str) -> String {
        let mut d = dir.to_string();
        if self.mirror_x {
            d = match d.as_str() {
                "east" => "west".into(),
                "west" => "east".into(),
                _ => d,
            };
        }
        for _ in 0..self.quarter_turns {
            d = match d.as_str() {
                "north" => "east".into(),
                "east" => "south".into(),
                "south" => "west".into(),
                "west" => "north".into(),
                _ => d,
            };
        }
        d
    }

    pub fn state(self, state: &BlockState) -> BlockState {
        if self == Transform::IDENTITY {
            return state.clone();
        }
        let mut out = BlockState { id: state.id.clone(), props: Default::default() };
        for (key, value) in &state.props {
            let (k, v) = self.property(&state.id, key, value);
            out.props.insert(k, v);
        }
        out
    }

    fn property(self, id: &str, key: &str, value: &str) -> (String, String) {
        const SIDES: [&str; 4] = ["north", "east", "south", "west"];
        if SIDES.contains(&key) {
            // Connection properties (dust, fences, walls, panes) move with
            // the side they describe.
            return (self.direction(key), value.to_string());
        }
        let v = match key {
            "facing" | "horizontal_facing" => self.direction(value),
            "axis" if self.quarter_turns % 2 == 1 => match value {
                "x" => "z".into(),
                "z" => "x".into(),
                other => other.into(),
            },
            "orientation" => value.split('_').map(|t| self.direction(t)).collect::<Vec<_>>().join("_"),
            "rotation" => match value.parse::<i32>() {
                Ok(r) => {
                    let r = if self.mirror_x { (16 - r) % 16 } else { r };
                    ((r + 4 * self.quarter_turns as i32) % 16).to_string()
                }
                Err(_) => value.into(),
            },
            "shape" if id.ends_with("rail") => self.rail_shape(value),
            "shape" if id.ends_with("_stairs") && self.mirror_x => swap_left_right(value),
            "hinge" if self.mirror_x => swap_left_right(value),
            "type" if self.mirror_x && id.ends_with("chest") => swap_left_right(value),
            _ => value.into(),
        };
        (key.to_string(), v)
    }

    fn rail_shape(self, shape: &str) -> String {
        if let Some(dir) = shape.strip_prefix("ascending_") {
            return format!("ascending_{}", self.direction(dir));
        }
        let parts: Vec<String> = shape.split('_').map(|t| self.direction(t)).collect();
        let has = |d: &str| parts.iter().any(|p| p == d);
        if has("north") && has("south") {
            return "north_south".into();
        }
        if has("east") && has("west") {
            return "east_west".into();
        }
        // Curves are named north or south first, then east or west.
        let ns = if has("north") { "north" } else { "south" };
        let ew = if has("east") { "east" } else { "west" };
        format!("{ns}_{ew}")
    }
}

fn swap_left_right(v: &str) -> String {
    if v.contains("left") { v.replace("left", "right") } else { v.replace("right", "left") }
}

impl fmt::Display for Transform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let m = if self.mirror_x { "m" } else { "" };
        write!(f, "{m}r{}", self.quarter_turns as u32 * 90)
    }
}

impl FromStr for Transform {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self> {
        let (mirror_x, rest) = match s.strip_prefix('m') {
            Some(r) => (true, r),
            None => (false, s),
        };
        let quarter_turns = match rest {
            "r0" => 0,
            "r90" => 1,
            "r180" => 2,
            "r270" => 3,
            _ => bail!("unknown transform {s:?}; expected r0, r90, r180, r270, or the same with an m prefix"),
        };
        Ok(Transform { quarter_turns, mirror_x })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(s: &str) -> BlockState {
        BlockState::parse(s).unwrap()
    }

    #[test]
    fn names_round_trip() {
        for t in Transform::all() {
            assert_eq!(t.to_string().parse::<Transform>().unwrap(), t);
        }
    }

    #[test]
    fn quarter_turn_moves_north_edge_to_east_edge() {
        let t = Transform { quarter_turns: 1, mirror_x: false };
        // A 3 wide (x) by 2 deep (z) box becomes 2 wide by 3 deep.
        assert_eq!(t.size([3, 1, 2]), [2, 1, 3]);
        assert_eq!(t.pos([0, 0, 0], [3, 1, 2]), [1, 0, 0]);
        assert_eq!(t.pos([2, 0, 0], [3, 1, 2]), [1, 0, 2]);
        assert_eq!(t.pos([0, 0, 1], [3, 1, 2]), [0, 0, 0]);
    }

    #[test]
    fn four_turns_are_identity() {
        let size = [5, 2, 3];
        for x in 0..5 {
            for z in 0..3 {
                let mut p = [x, 1, z];
                let mut s = size;
                let t = Transform { quarter_turns: 1, mirror_x: false };
                for _ in 0..4 {
                    p = t.pos(p, s);
                    s = t.size(s);
                }
                assert_eq!(p, [x, 1, z]);
            }
        }
    }

    #[test]
    fn positions_stay_inside_the_box() {
        let size = [4, 1, 7];
        for t in Transform::all() {
            let [w, _, d] = t.size(size);
            for x in 0..4 {
                for z in 0..7 {
                    let [nx, _, nz] = t.pos([x, 0, z], size);
                    assert!((0..w).contains(&nx) && (0..d).contains(&nz), "{t} {x},{z}");
                }
            }
        }
    }

    #[test]
    fn rotates_facing_and_connections() {
        let t = Transform { quarter_turns: 1, mirror_x: false };
        let s = t.state(&st("redstone_wire[north=side,east=none,south=up,west=none,power=3]"));
        assert_eq!(s.to_string(), "minecraft:redstone_wire[east=side,north=none,power=3,south=none,west=up]");
        let r = t.state(&st("repeater[facing=west,delay=2]"));
        assert_eq!(r.get("facing"), Some("north"));
        let p = t.state(&st("piston[facing=up,extended=false]"));
        assert_eq!(p.get("facing"), Some("up"));
    }

    #[test]
    fn mirror_swaps_east_west_and_handedness() {
        let t = Transform { quarter_turns: 0, mirror_x: true };
        assert_eq!(t.state(&st("lever[facing=east,face=wall,powered=false]")).get("facing"), Some("west"));
        assert_eq!(t.state(&st("oak_door[hinge=left]")).get("hinge"), Some("right"));
        assert_eq!(t.state(&st("stone_stairs[shape=inner_left]")).get("shape"), Some("inner_right"));
    }

    #[test]
    fn rail_shapes_stay_canonical() {
        let t = Transform { quarter_turns: 1, mirror_x: false };
        assert_eq!(t.state(&st("rail[shape=north_south]")).get("shape"), Some("east_west"));
        assert_eq!(t.state(&st("rail[shape=north_east]")).get("shape"), Some("south_east"));
        assert_eq!(t.state(&st("powered_rail[shape=ascending_north]")).get("shape"), Some("ascending_east"));
    }

    #[test]
    fn axis_swaps_on_odd_turns() {
        let t = Transform { quarter_turns: 3, mirror_x: true };
        assert_eq!(t.state(&st("oak_log[axis=x]")).get("axis"), Some("z"));
        assert_eq!(t.state(&st("oak_log[axis=y]")).get("axis"), Some("y"));
    }
}
