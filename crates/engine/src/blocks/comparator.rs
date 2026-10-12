//! `ComparatorBlock`, on top of the `DiodeBlock` parts in [`super::repeater`].
//!
//! `facing` points at the input, as for repeaters. The output strength lives
//! in the block entity, not the block state: `powered` only says whether it
//! is above zero.

use crate::level::{Level, flags};
use crate::pos::{Dir, Pos};
use crate::registry::StateId;

use super::{BlockEntity, repeater};

const HIGH: i32 = -1;

fn subtract(level: &Level, state: StateId) -> bool {
    level.prop(state, "mode") == "subtract"
}

/// The block entity's stored output signal.
pub fn output(level: &Level, pos: Pos) -> i32 {
    match level.block_entities.get(&pos) {
        Some(BlockEntity::Comparator(signal)) => *signal,
        _ => 0,
    }
}

/// `ComparatorBlock.getInputSignal`: the diode input, unless a block with an
/// analog output sits behind it, directly or through one conductor.
fn input_signal(level: &Level, pos: Pos, state: StateId) -> i32 {
    let mut signal = repeater::input_signal(level, pos, state);
    let dir = level.dir(state, "facing");
    let p = pos.offset(dir);
    let s = level.get(p);
    if super::has_analog_output(level, s) {
        signal = super::analog_output(level, p, s);
    } else if signal < 15 && super::is_conductor(level, p, s) {
        let p = p.offset(dir);
        let s = level.get(p);
        // Item frames are entities, which phase 2 does not have yet.
        if super::has_analog_output(level, s) {
            signal = super::analog_output(level, p, s);
        }
    }
    signal
}

/// `calculateOutputSignal`.
fn calculate_output(level: &Level, pos: Pos, state: StateId) -> i32 {
    let input = input_signal(level, pos, state);
    if input == 0 {
        return 0;
    }
    let side = repeater::alternate_signal(level, pos, state, false);
    if side > input {
        0
    } else if subtract(level, state) {
        input - side
    } else {
        input
    }
}

/// `shouldTurnOn`.
pub(super) fn should_turn_on(level: &Level, pos: Pos, state: StateId) -> bool {
    let input = input_signal(level, pos, state);
    if input == 0 {
        return false;
    }
    let side = repeater::alternate_signal(level, pos, state, false);
    input > side || (input == side && !subtract(level, state))
}

pub fn get_signal(level: &Level, pos: Pos, state: StateId, dir: Dir) -> i32 {
    if level.flag(state, "powered") && level.dir(state, "facing") == dir { output(level, pos) } else { 0 }
}

pub fn neighbor_changed(level: &mut Level, pos: Pos, state: StateId) {
    if !repeater::break_if_unsupported(level, pos, state) {
        check_tick_on_neighbor(level, pos, state);
    }
}

/// `ComparatorBlock.checkTickOnNeighbor`.
fn check_tick_on_neighbor(level: &mut Level, pos: Pos, state: StateId) {
    let block = level.block(state);
    if level.will_tick_this_tick(pos, block) {
        return;
    }
    let out = calculate_output(level, pos, state);
    if out != output(level, pos) || level.flag(state, "powered") != should_turn_on(level, pos, state) {
        let priority = if repeater::should_prioritize(level, pos, state) { HIGH } else { 0 };
        level.schedule_tick(pos, block, 2, priority);
    }
}

pub fn tick(level: &mut Level, pos: Pos, state: StateId) {
    refresh_output_state(level, pos, state);
}

/// `refreshOutputState`: store the new strength, and in compare mode or when
/// it changed, update `powered` and the block in front.
fn refresh_output_state(level: &mut Level, pos: Pos, state: StateId) {
    let out = calculate_output(level, pos, state);
    let old = match level.block_entities.get_mut(&pos) {
        Some(BlockEntity::Comparator(signal)) => std::mem::replace(signal, out),
        _ => 0,
    };
    if old != out || !subtract(level, state) {
        let on = should_turn_on(level, pos, state);
        let powered = level.flag(state, "powered");
        if powered && !on {
            level.set_block(pos, level.with_bool(state, "powered", false), flags::CLIENTS);
        } else if !powered && on {
            level.set_block(pos, level.with_bool(state, "powered", true), flags::CLIENTS);
        }
        repeater::update_neighbors_in_front(level, pos, state);
    }
}

/// Right-click: switch between compare and subtract.
pub fn use_without_item(level: &mut Level, pos: Pos, state: StateId) {
    let next = level.with(state, "mode", if subtract(level, state) { "compare" } else { "subtract" });
    level.set_block(pos, next, flags::CLIENTS);
    refresh_output_state(level, pos, next);
}
