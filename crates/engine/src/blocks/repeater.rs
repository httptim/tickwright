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

fn is_diode(level: &Level, state: StateId) -> bool {
    level.kind(state) == Kind::Repeater
}

/// `getInputSignal`.
fn input_signal(level: &Level, pos: Pos, state: StateId) -> i32 {
    let dir = level.dir(state, "facing");
    let p = pos.offset(dir);
    let signal = level.get_signal(p, dir);
    if signal >= 15 {
        return signal;
    }
    let s = level.get(p);
    signal.max(if level.kind(s) == Kind::RedstoneWire { level.int(s, "power") } else { 0 })
}

fn should_turn_on(level: &Level, pos: Pos, state: StateId) -> bool {
    input_signal(level, pos, state) > 0
}

/// `SignalGetter.getControlInputSignal` with `onlyDiodes`.
fn control_input_signal(level: &Level, pos: Pos, dir: Dir) -> i32 {
    if is_diode(level, level.get(pos)) { level.get_direct_signal(pos, dir) } else { 0 }
}

/// `isLocked`: a powered diode points into either side.
pub fn is_locked(level: &Level, pos: Pos, state: StateId) -> bool {
    let facing = level.dir(state, "facing");
    let (cw, ccw) = (facing.clockwise(), facing.counter_clockwise());
    control_input_signal(level, pos.offset(cw), cw).max(control_input_signal(level, pos.offset(ccw), ccw)) > 0
}

/// `shouldPrioritize`: the repeater outputs into a diode not facing back.
fn should_prioritize(level: &Level, pos: Pos, state: StateId) -> bool {
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

pub fn neighbor_changed(level: &mut Level, pos: Pos, state: StateId) {
    let below = pos.below();
    if !can_survive_on(level, below, level.get(below)) {
        let block = level.block(state);
        level.remove_block(pos);
        for dir in Dir::ALL {
            level.update_neighbors_at(pos.offset(dir), block);
        }
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
fn update_neighbors_in_front(level: &mut Level, pos: Pos, state: StateId) {
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

fn can_survive_on(level: &Level, pos: Pos, state: StateId) -> bool {
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
