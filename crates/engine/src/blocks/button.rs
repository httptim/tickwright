//! `ButtonBlock`, for the buttons arrows cannot press (stone and polished
//! blackstone), which stay pressed for 20 game ticks.

use super::connected_direction;
use crate::level::{Level, flags};
use crate::pos::Pos;
use crate::registry::StateId;

/// `BlockSetType.STONE.ticksToStayPressed`.
const TICKS_TO_STAY_PRESSED: i32 = 20;

/// `useWithoutItem`: press, unless already pressed.
pub fn press(level: &mut Level, pos: Pos, state: StateId) {
    if level.flag(state, "powered") {
        return;
    }
    level.set_block(pos, level.with_bool(state, "powered", true), flags::ALL);
    update_neighbours(level, pos, state);
    level.schedule_tick(pos, level.block(state), TICKS_TO_STAY_PRESSED, 0);
}

/// `tick` and `checkPressed`: nothing keeps a stone button pressed.
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
