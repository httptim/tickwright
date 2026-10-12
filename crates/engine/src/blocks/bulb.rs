//! `CopperBulbBlock` and `BellBlock`: blocks that react to a change of
//! power rather than to power itself.

use crate::level::{Level, flags};
use crate::pos::{Dir, Pos};
use crate::registry::StateId;

// --- CopperBulbBlock -----------------------------------------------------------

/// `checkAndFlip`: a rising edge toggles `lit`.
fn check_and_flip(level: &mut Level, pos: Pos, state: StateId) {
    let powered = level.has_neighbor_signal(pos);
    if powered == level.flag(state, "powered") {
        return;
    }
    let mut s = state;
    if !level.flag(state, "powered") {
        s = level.with_bool(s, "lit", !level.flag(state, "lit"));
    }
    level.set_block(pos, level.with_bool(s, "powered", powered), flags::ALL);
}

pub fn bulb_on_place(level: &mut Level, pos: Pos, state: StateId, old: StateId) {
    if level.block(old) != level.block(state) {
        check_and_flip(level, pos, state);
    }
}

pub fn bulb_neighbor_changed(level: &mut Level, pos: Pos, state: StateId) {
    check_and_flip(level, pos, state);
}

pub fn bulb_analog_output(level: &Level, state: StateId) -> i32 {
    if level.flag(state, "lit") { 15 } else { 0 }
}

// --- BellBlock -----------------------------------------------------------------

/// `neighborChanged`: a rising edge rings the bell (a block event), then the
/// new `powered` is written.
pub fn bell_neighbor_changed(level: &mut Level, pos: Pos, state: StateId) {
    let signal = level.has_neighbor_signal(pos);
    if signal == level.flag(state, "powered") {
        return;
    }
    if signal {
        // `attemptToRing` with no direction: `BellBlockEntity.onHit(facing)`.
        let facing = level.dir(state, "facing");
        level.block_event(pos, level.block(state), 1, dir_3d(facing));
    }
    level.set_block(pos, level.with_bool(state, "powered", signal), flags::ALL);
}

/// `Direction.get3DDataValue`.
fn dir_3d(d: Dir) -> i32 {
    d as i32
}

/// `updateShape` for a bell standing on the floor or hanging from the
/// ceiling: it drops when its support goes.
pub fn bell_update_shape(level: &Level, pos: Pos, state: StateId, dir: Dir) -> StateId {
    let support = match level.prop(state, "attachment") {
        "floor" => Dir::Down,
        "ceiling" => Dir::Up,
        _ => return state,
    };
    if dir == support {
        let p = pos.offset(support);
        if !super::is_face_sturdy(level, p, level.get(p), support.opposite()) {
            return level.air;
        }
    }
    state
}
