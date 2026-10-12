//! Blocks that turn into entities: falling blocks (`FallingBlock`) and TNT
//! (`TntBlock`).

use crate::entity;
use crate::level::Level;
use crate::pos::Pos;
use crate::registry::StateId;

/// `FallingBlock.getDelayAfterPlace`.
const DELAY: i32 = 2;

/// `FallingBlock.onPlace`.
pub fn on_place(level: &mut Level, pos: Pos, state: StateId) {
    level.schedule_tick(pos, level.block(state), DELAY, 0);
}

/// `FallingBlock.updateShape`: any neighbor change checks again.
pub fn update_shape(level: &mut Level, pos: Pos, state: StateId) -> StateId {
    level.schedule_tick(pos, level.block(state), DELAY, 0);
    state
}

/// `FallingBlock.tick`.
pub fn tick(level: &mut Level, pos: Pos, state: StateId) {
    if entity::is_free(level, level.get(pos.below())) && pos.y >= level.min_y() {
        entity::fall(level, pos, state);
    }
}

/// `TntBlock.onPlace`: placed where it is powered, it lights at once.
pub fn tnt_on_place(level: &mut Level, pos: Pos, state: StateId, old: StateId) {
    if level.block(old) != level.block(state) && level.has_neighbor_signal(pos) {
        entity::prime(level, pos);
        level.remove_block(pos);
    }
}

/// `TntBlock.neighborChanged`.
pub fn tnt_neighbor_changed(level: &mut Level, pos: Pos) {
    if level.has_neighbor_signal(pos) {
        entity::prime(level, pos);
        level.remove_block(pos);
    }
}
