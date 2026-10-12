//! `RedstoneLampBlock`: turns on at once, turns off four ticks later.

use crate::level::{Level, flags};
use crate::pos::Pos;
use crate::registry::StateId;

pub fn neighbor_changed(level: &mut Level, pos: Pos, state: StateId) {
    let lit = level.flag(state, "lit");
    if lit != level.has_neighbor_signal(pos) {
        if lit {
            level.schedule_tick(pos, level.block(state), 4, 0);
        } else {
            level.set_block(pos, level.with_bool(state, "lit", true), flags::CLIENTS);
        }
    }
}

pub fn tick(level: &mut Level, pos: Pos, state: StateId) {
    if level.flag(state, "lit") && !level.has_neighbor_signal(pos) {
        level.set_block(pos, level.with_bool(state, "lit", false), flags::CLIENTS);
    }
}
