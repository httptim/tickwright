//! `RepeaterBlock`, with the parts of `DiodeBlock` it uses.
//!
//! `facing` points at the input; the repeater powers the block on the other
//! side.

use crate::level::{Level, flags};
use crate::pos::{Dir, Pos};
use crate::registry::StateId;

use super::Kind;

/// `TickPriority` values.
const EXTREMELY_HIGH: i32 = -3;
const VERY_HIGH: i32 = -2;
const HIGH: i32 = -1;

fn delay(level: &Level, state: StateId) -> i32 {
    level.int(state, "delay") * 2
}

/// `DiodeBlock.isDiode`: repeaters and comparators.
pub(super) fn is_diode(level: &Level, state: StateId) -> bool {
    matches!(level.kind(state), Kind::Repeater | Kind::Comparator)
}

/// `DiodeBlock.getInputSignal`.
pub(super) fn input_signal(level: &Level, pos: Pos, state: StateId) -> i32 {
    let dir = level.dir(state, "facing");
    let p = pos.offset(dir);
    let signal = level.get_signal(p, dir);
    if signal >= 15 {
        return signal;
    }
    let s = level.get(p);
    signal.max(if level.kind(s) == Kind::RedstoneWire { level.int(s, "power") } else { 0 })
}

pub(super) fn should_turn_on(level: &Level, pos: Pos, state: StateId) -> bool {
    input_signal(level, pos, state) > 0
}

/// `SignalGetter.getControlInputSignal`.
fn control_input_signal(level: &Level, pos: Pos, dir: Dir, only_diodes: bool) -> i32 {
    let s = level.get(pos);
    if only_diodes {
        return if is_diode(level, s) { level.get_direct_signal(pos, dir) } else { 0 };
    }
    match level.kind(s) {
        Kind::RedstoneBlock => 15,
        Kind::RedstoneWire => level.int(s, "power"),
        _ if super::is_signal_source(level, s) => level.get_direct_signal(pos, dir),
        _ => 0,
    }
}

/// `DiodeBlock.getAlternateSignal`: the stronger of the two side inputs.
/// Repeaters only listen to diodes there; comparators to any source.
pub(super) fn alternate_signal(level: &Level, pos: Pos, state: StateId, only_diodes: bool) -> i32 {
    let facing = level.dir(state, "facing");
    let (cw, ccw) = (facing.clockwise(), facing.counter_clockwise());
    control_input_signal(level, pos.offset(cw), cw, only_diodes).max(control_input_signal(
        level,
        pos.offset(ccw),
        ccw,
        only_diodes,
    ))
}

/// `isLocked`: a powered diode points into either side.
pub fn is_locked(level: &Level, pos: Pos, state: StateId) -> bool {
    alternate_signal(level, pos, state, true) > 0
}

/// `shouldPrioritize`: the diode outputs into a diode not facing back.
pub(super) fn should_prioritize(level: &Level, pos: Pos, state: StateId) -> bool {
    let dir = level.dir(state, "facing").opposite();
    let front = level.get(pos.offset(dir));
    is_diode(level, front) && level.dir(front, "facing") != dir
}

pub fn get_signal(level: &Level, _pos: Pos, state: StateId, dir: Dir) -> i32 {
    if level.flag(state, "powered") && level.dir(state, "facing") == dir { 15 } else { 0 }
}

pub fn tick(level: &mut Level, pos: Pos, state: StateId) {
    if is_locked(level, pos, state) {
        return;
    }
    let powered = level.flag(state, "powered");
    let on = should_turn_on(level, pos, state);
    if powered && !on {
        level.set_block(pos, level.with_bool(state, "powered", false), flags::CLIENTS);
    } else if !powered {
        level.set_block(pos, level.with_bool(state, "powered", true), flags::CLIENTS);
        if !on {
            level.schedule_tick(pos, level.block(state), delay(level, state), VERY_HIGH);
        }
    }
}

/// `DiodeBlock.neighborChanged`, first half: a diode that lost its support
/// breaks and tells the blocks around it. Returns whether it broke.
pub(super) fn break_if_unsupported(level: &mut Level, pos: Pos, state: StateId) -> bool {
    let below = pos.below();
    if can_survive_on(level, below, level.get(below)) {
        return false;
    }
    let block = level.block(state);
    level.remove_block(pos);
    for dir in Dir::ALL {
        level.update_neighbors_at(pos.offset(dir), block);
    }
    true
}

pub fn neighbor_changed(level: &mut Level, pos: Pos, state: StateId) {
    if break_if_unsupported(level, pos, state) {
        return;
    }
    if is_locked(level, pos, state) {
        return;
    }
    let powered = level.flag(state, "powered");
    let block = level.block(state);
    if powered != should_turn_on(level, pos, state) && !level.will_tick_this_tick(pos, block) {
        let priority = if should_prioritize(level, pos, state) {
            EXTREMELY_HIGH
        } else if powered {
            VERY_HIGH
        } else {
            HIGH
        };
        level.schedule_tick(pos, block, delay(level, state), priority);
    }
}

/// `updateNeighborsInFront`.
pub(super) fn update_neighbors_in_front(level: &mut Level, pos: Pos, state: StateId) {
    let facing = level.dir(state, "facing");
    let front = pos.offset(facing.opposite());
    let block = level.block(state);
    level.neighbor_changed(front, block);
    level.update_neighbors_except(front, block, facing);
}

pub fn on_place(level: &mut Level, pos: Pos, state: StateId) {
    update_neighbors_in_front(level, pos, state);
}

pub fn affect_neighbors_after_removal(level: &mut Level, pos: Pos, state: StateId, moved_by_piston: bool) {
    if !moved_by_piston {
        update_neighbors_in_front(level, pos, state);
    }
}

pub(super) fn can_survive_on(level: &Level, pos: Pos, state: StateId) -> bool {
    super::is_face_sturdy(level, pos, state, Dir::Up)
}

pub fn update_shape(
    level: &Level,
    pos: Pos,
    state: StateId,
    dir: Dir,
    neighbor: Pos,
    neighbor_state: StateId,
) -> StateId {
    if dir == Dir::Down && !can_survive_on(level, neighbor, neighbor_state) {
        return level.air;
    }
    let facing = level.dir(state, "facing");
    // Any direction off the facing axis, vertical included, re-checks the lock.
    if dir != facing && dir != facing.opposite() {
        level.with_bool(state, "locked", is_locked(level, pos, state))
    } else {
        state
    }
}

/// Right-click: step the delay.
pub fn use_without_item(level: &mut Level, pos: Pos, state: StateId) {
    let next = level.int(state, "delay") % 4 + 1;
    level.set_block(pos, level.with_int(state, "delay", next), flags::ALL);
}
