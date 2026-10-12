//! Blocks with little redstone behaviour of their own that builds still
//! lean on: leaves, target blocks, jukeboxes and tripwire hooks without
//! string.

use crate::level::{Level, flags};
use crate::pos::{Dir, Pos};
use crate::registry::StateId;

use super::Kind;

// --- LeavesBlock -----------------------------------------------------------------

/// `getOptionalDistanceAt`, with no distance read as 7.
pub(super) fn distance_at(level: &Level, state: StateId) -> i32 {
    match level.kind(state) {
        Kind::Log => 0,
        Kind::Leaves => level.int(state, "distance"),
        _ => 7,
    }
}

/// `LeavesBlock.updateShape`: any change next to the leaves schedules a
/// distance check unless the neighbour is a log and the leaves already know.
pub fn leaves_update_shape(level: &mut Level, pos: Pos, state: StateId, neighbor_state: StateId) -> StateId {
    let d = distance_at(level, neighbor_state) + 1;
    if d != 1 || level.int(state, "distance") != d {
        level.schedule_tick(pos, level.block(state), 1, 0);
    }
    state
}

/// `LeavesBlock.tick`: `setBlock(updateDistance(...), 3)`.
pub fn leaves_tick(level: &mut Level, pos: Pos, state: StateId) {
    let mut d = 7;
    for dir in Dir::ALL {
        let p = pos.offset(dir);
        d = d.min(distance_at(level, level.get(p)) + 1);
        if d == 1 {
            break;
        }
    }
    level.set_block(pos, level.with_int(state, "distance", d), flags::ALL);
}

// --- TargetBlock -----------------------------------------------------------------

/// Only projectiles power a target block; its signal is its `power`.
pub fn target_signal(level: &Level, state: StateId) -> i32 {
    level.int(state, "power")
}

// --- TripWireHookBlock -----------------------------------------------------------

/// Weak power from a powered hook, in every direction.
pub fn hook_signal(level: &Level, state: StateId) -> i32 {
    if level.flag(state, "powered") { 15 } else { 0 }
}

/// Strong power from a powered hook, into the block it hangs on.
pub fn hook_direct_signal(level: &Level, state: StateId, dir: Dir) -> i32 {
    if level.flag(state, "powered") && level.dir(state, "facing") == dir { 15 } else { 0 }
}

/// `TripWireHookBlock.updateShape`: a hook whose wall goes pops off.
pub fn hook_update_shape(level: &Level, pos: Pos, state: StateId, dir: Dir) -> StateId {
    let facing = level.dir(state, "facing");
    if dir == facing.opposite() {
        let wall = pos.offset(facing.opposite());
        if !super::is_face_sturdy(level, wall, level.get(wall), facing) {
            return level.air;
        }
    }
    state
}
