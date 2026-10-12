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

/// `TargetBlock.onProjectileHit`: the nearer the hit is to the middle of
/// the face, the stronger the signal, for 20 ticks after an arrow and 8
/// after anything else. A hit while it is still powered changes nothing.
pub fn target_hit(level: &mut Level, pos: Pos, state: StateId, at: [f64; 3], face: Dir, arrow: bool) {
    let off = |v: f64| (v - v.floor() - 0.5).abs();
    let (dx, dy, dz) = (off(at[0]), off(at[1]), off(at[2]));
    let d = match face.axis() {
        1 => dx.max(dz),
        2 => dx.max(dy),
        _ => dy.max(dz),
    };
    let power = ((15.0 * ((0.5 - d) / 0.5).clamp(0.0, 1.0)).ceil() as i32).max(1);
    let block = level.block(state);
    if !level.has_scheduled_tick(pos, block) {
        level.set_block(pos, level.with_int(state, "power", power), flags::ALL);
        level.schedule_tick(pos, block, if arrow { 20 } else { 8 }, 0);
    }
}

/// `TargetBlock.tick`: the signal ends.
pub fn target_tick(level: &mut Level, pos: Pos, state: StateId) {
    if level.int(state, "power") != 0 {
        level.set_block(pos, level.with_int(state, "power", 0), flags::ALL);
    }
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

/// `BaseCoralPlantTypeBlock.updateShape` and `BaseCoralWallFanBlock`'s: dead
/// coral pops when what holds it up goes. Waterlogged coral would schedule
/// a water tick, which isn't simulated.
pub fn coral_update_shape(level: &mut Level, pos: Pos, state: StateId, dir: Dir) -> StateId {
    if level.flag(state, "waterlogged") {
        level.unsupported("waterlogged coral");
        return state;
    }
    let support =
        if level.kind(state) == Kind::DeadCoralWallFan { level.dir(state, "facing").opposite() } else { Dir::Down };
    let at = pos.offset(support);
    if dir == support && !super::is_face_sturdy(level, at, level.get(at), support.opposite()) {
        level.air
    } else {
        state
    }
}

/// `CarpetBlock.updateShape`: a carpet with nothing under it pops. Any
/// block holds it up, even a moving piston.
pub fn carpet_update_shape(level: &mut Level, pos: Pos, state: StateId, dir: Dir) -> StateId {
    if dir == Dir::Down && level.is_air(level.get(pos.below())) { level.air } else { state }
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
