//! `LeverBlock`.

use super::connected_direction;
use crate::level::{Level, flags};
use crate::pos::{Dir, Pos};
use crate::registry::StateId;

/// `LeverBlock.pull`: flip, write with updates, then update the neighbors
/// of the lever and of the block it is attached to.
pub fn pull(level: &mut Level, pos: Pos, state: StateId) {
    let on = !level.flag(state, "powered");
    let state = level.with_bool(state, "powered", on);
    level.set_block(pos, state, flags::ALL);
    update_neighbours(level, pos, state);
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

pub fn get_signal(level: &Level, _pos: Pos, state: StateId, _dir: Dir) -> i32 {
    if level.flag(state, "powered") { 15 } else { 0 }
}

pub fn get_direct_signal(level: &Level, _pos: Pos, state: StateId, dir: Dir) -> i32 {
    if level.flag(state, "powered") && connected_direction(level, state) == dir { 15 } else { 0 }
}
