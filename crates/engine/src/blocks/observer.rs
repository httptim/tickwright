//! `ObserverBlock`: watches the block it faces and, when that block's shape
//! update reaches it, pulses its back for two game ticks.
//!
//! It ignores neighbor updates entirely; only `updateShape` from the watched
//! side starts it. Its writes use flag 2, so blocks beside it never hear the
//! pulse: only the block behind it and that block's other neighbors do.

use crate::level::{Level, flags};
use crate::pos::{Dir, Pos};
use crate::registry::StateId;

pub fn update_shape(level: &mut Level, pos: Pos, state: StateId, dir: Dir) -> StateId {
    if level.dir(state, "facing") == dir && !level.flag(state, "powered") {
        start_signal(level, pos, state);
    }
    state
}

/// `startSignal`: guarded on a pending tick, not on `willTickThisTick`.
fn start_signal(level: &mut Level, pos: Pos, state: StateId) {
    let block = level.block(state);
    if !level.has_scheduled_tick(pos, block) {
        level.schedule_tick(pos, block, 2, 0);
    }
}

pub fn tick(level: &mut Level, pos: Pos, state: StateId) {
    if level.flag(state, "powered") {
        level.set_block(pos, level.with_bool(state, "powered", false), flags::CLIENTS);
    } else {
        level.set_block(pos, level.with_bool(state, "powered", true), flags::CLIENTS);
        level.schedule_tick(pos, level.block(state), 2, 0);
    }
    update_neighbors_in_front(level, pos, state);
}

/// `updateNeighborsInFront`: the block behind the observer, then that
/// block's neighbors except the observer itself.
fn update_neighbors_in_front(level: &mut Level, pos: Pos, state: StateId) {
    let facing = level.dir(state, "facing");
    let behind = pos.offset(facing.opposite());
    let block = level.block(state);
    level.neighbor_changed(behind, block);
    level.update_neighbors_except(behind, block, facing);
}

/// `getSignal` and `getDirectSignal`: 15 out of the back while powered.
pub fn get_signal(level: &Level, state: StateId, dir: Dir) -> i32 {
    if level.flag(state, "powered") && level.dir(state, "facing") == dir { 15 } else { 0 }
}

/// A powered observer placed fresh (pasted, or set by a command) turns off
/// at once unless a tick is already due.
pub fn on_place(level: &mut Level, pos: Pos, state: StateId, old: StateId) {
    let block = level.block(state);
    if !level.is(old, block) && level.flag(state, "powered") && !level.has_scheduled_tick(pos, block) {
        let off = level.with_bool(state, "powered", false);
        level.set_block(pos, off, flags::CLIENTS | flags::KNOWN_SHAPE);
        update_neighbors_in_front(level, pos, off);
    }
}

/// Removing an observer mid-pulse tells the block behind it the pulse ended.
pub fn affect_neighbors_after_removal(level: &mut Level, pos: Pos, state: StateId) {
    if level.flag(state, "powered") && level.has_scheduled_tick(pos, level.block(state)) {
        let off = level.with_bool(state, "powered", false);
        update_neighbors_in_front(level, pos, off);
    }
}
