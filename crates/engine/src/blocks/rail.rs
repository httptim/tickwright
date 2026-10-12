//! `PoweredRailBlock` (powered and activator rails) with the parts of
//! `BaseRailBlock` they use.
//!
//! Rail shapes are taken as placed: `RailState`, which bends and connects
//! rails when one is placed, is not here yet. For the straight lines these
//! rails form it changes nothing, but a rail placed by a stimulus next to
//! others would not reconnect them.

use crate::level::{Level, flags};
use crate::pos::{Dir, Pos};
use crate::registry::StateId;

fn is_slope(shape: &str) -> bool {
    shape.starts_with("ascending_")
}

/// `BaseRailBlock.canSupportRigidBlock`, approximated by a sturdy face.
fn supports(level: &Level, pos: Pos) -> bool {
    super::is_face_sturdy(level, pos, level.get(pos), Dir::Up)
}

/// `shouldBeRemoved`: no support below, or none on the high side of a slope.
fn should_be_removed(level: &Level, pos: Pos, shape: &str) -> bool {
    if !supports(level, pos.below()) {
        return true;
    }
    match shape {
        "ascending_east" => !supports(level, pos.offset(Dir::East)),
        "ascending_west" => !supports(level, pos.offset(Dir::West)),
        "ascending_north" => !supports(level, pos.offset(Dir::North)),
        "ascending_south" => !supports(level, pos.offset(Dir::South)),
        _ => false,
    }
}

/// `findPoweredRailSignal`: follow the rail one way, through at most eight
/// powered rails, looking for one that has a signal of its own.
fn find_signal(level: &Level, pos: Pos, state: StateId, forward: bool, depth: i32) -> bool {
    if depth >= 8 {
        return false;
    }
    let (mut x, mut y, mut z) = (pos.x, pos.y, pos.z);
    let mut also_below = true;
    let mut shape = level.prop(state, "shape");
    match shape {
        "north_south" => z += if forward { 1 } else { -1 },
        "east_west" => x += if forward { -1 } else { 1 },
        "ascending_east" => {
            if forward {
                x -= 1;
            } else {
                x += 1;
                y += 1;
                also_below = false;
            }
            shape = "east_west";
        }
        "ascending_west" => {
            if forward {
                x -= 1;
                y += 1;
                also_below = false;
            } else {
                x += 1;
            }
            shape = "east_west";
        }
        "ascending_north" => {
            if forward {
                z += 1;
            } else {
                z -= 1;
                y += 1;
                also_below = false;
            }
            shape = "north_south";
        }
        "ascending_south" => {
            if forward {
                z += 1;
                y += 1;
                also_below = false;
            } else {
                z -= 1;
            }
            shape = "north_south";
        }
        _ => {}
    }
    let next = Pos::new(x, y, z);
    is_same_rail_with_power(level, state, next, forward, depth, shape)
        || (also_below && is_same_rail_with_power(level, state, next.below(), forward, depth, shape))
}

/// `isSameRailWithPower`: the same kind of rail, lined up, powered, and
/// either signalled itself or further along the chain.
fn is_same_rail_with_power(level: &Level, rail: StateId, pos: Pos, forward: bool, depth: i32, shape: &str) -> bool {
    let s = level.get(pos);
    if !level.is(s, level.block(rail)) {
        return false;
    }
    let other = level.prop(s, "shape");
    let crosses = match shape {
        "east_west" => matches!(other, "north_south" | "ascending_north" | "ascending_south"),
        "north_south" => matches!(other, "east_west" | "ascending_east" | "ascending_west"),
        _ => false,
    };
    if crosses || !level.flag(s, "powered") {
        return false;
    }
    level.has_neighbor_signal(pos) || find_signal(level, pos, s, forward, depth + 1)
}

/// `PoweredRailBlock.updateState`.
fn update_state(level: &mut Level, pos: Pos, state: StateId) {
    let powered = level.flag(state, "powered");
    let now = level.has_neighbor_signal(pos)
        || find_signal(level, pos, state, true, 0)
        || find_signal(level, pos, state, false, 0);
    if now != powered {
        let block = level.block(state);
        level.set_block(pos, level.with_bool(state, "powered", now), flags::ALL);
        level.update_neighbors_at(pos.below(), block);
        if is_slope(level.prop(state, "shape")) {
            level.update_neighbors_at(pos.above(), block);
        }
    }
}

/// `BaseRailBlock.neighborChanged`.
pub fn neighbor_changed(level: &mut Level, pos: Pos, state: StateId, moved_by_piston: bool) {
    if should_be_removed(level, pos, level.prop(state, "shape")) {
        let f = if moved_by_piston { flags::ALL | flags::MOVE_BY_PISTON } else { flags::ALL };
        level.set_block(pos, level.air, f);
    } else {
        update_state(level, pos, state);
    }
}

/// `BaseRailBlock.onPlace`: a fresh rail queues a neighbor update to itself,
/// which checks its power.
///
/// When only `powered` changes, the game also updates the neighbors of the
/// rail and of the block below it, before the write's own updates go out.
/// This is read from captures (`nucleation_rails_chain`), not the code.
pub fn on_place(level: &mut Level, pos: Pos, state: StateId, old: StateId) {
    let block = level.block(state);
    if !level.is(old, block) {
        level.neighbor_changed(pos, block);
    } else if level.flag(old, "powered") != level.flag(state, "powered") {
        level.update_neighbors_at(pos, block);
        level.update_neighbors_at(pos.below(), block);
        if is_slope(level.prop(state, "shape")) {
            level.update_neighbors_at(pos.above(), block);
        }
    }
}

pub fn affect_neighbors_after_removal(level: &mut Level, pos: Pos, state: StateId, moved_by_piston: bool) {
    if moved_by_piston {
        return;
    }
    let block = level.block(state);
    if is_slope(level.prop(state, "shape")) {
        level.update_neighbors_at(pos.above(), block);
    }
    level.update_neighbors_at(pos, block);
    level.update_neighbors_at(pos.below(), block);
}
