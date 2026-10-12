//! `ButtonBlock`. Stone and polished blackstone buttons stay pressed for 20
//! game ticks, wooden ones for 30. Wooden buttons can also be held down by
//! arrows, but there are no entities yet, so nothing keeps them pressed.

use super::connected_direction;
use crate::level::{Level, flags};
use crate::pos::Pos;
use crate::registry::StateId;

/// `BlockSetType.ticksToStayPressed`.
fn ticks_to_stay_pressed(level: &Level, state: StateId) -> i32 {
    match level.reg.name(state) {
        "minecraft:stone_button" | "minecraft:polished_blackstone_button" => 20,
        _ => 30,
    }
}

/// `useWithoutItem`: press, unless already pressed.
pub fn press(level: &mut Level, pos: Pos, state: StateId) {
    if level.flag(state, "powered") {
        return;
    }
    level.set_block(pos, level.with_bool(state, "powered", true), flags::ALL);
    update_neighbours(level, pos, state);
    level.schedule_tick(pos, level.block(state), ticks_to_stay_pressed(level, state), 0);
}

/// `tick` and `checkPressed`: with no arrows, nothing keeps a button pressed.
pub fn tick(level: &mut Level, pos: Pos, state: StateId) {
    if level.flag(state, "powered") {
        level.set_block(pos, level.with_bool(state, "powered", false), flags::ALL);
        update_neighbours(level, pos, state);
    }
}

fn update_neighbours(level: &mut Level, pos: Pos, state: StateId) {
    let block = level.block(state);
    let behind = connected_direction(level, state).opposite();
    level.update_neighbors_at(pos, block);
    level.update_neighbors_at(pos.offset(behind), block);
}

pub fn affect_neighbors_after_removal(level: &mut Level, pos: Pos, state: StateId, moved_by_piston: bool) {
    if !moved_by_piston && level.flag(state, "powered") {
        update_neighbours(level, pos, state);
    }
}
