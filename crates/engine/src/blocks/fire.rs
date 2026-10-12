//! `FireBlock`, as far as fire lit by a fireball on stone goes. Captures
//! have no player nearby, so fire never spreads, ages or burns out: each
//! tick only schedules the next. Fire that would need flammable blocks
//! around it to stay lit, or that could light a portal, is refused.

use super::is_face_sturdy;
use crate::level::Level;
use crate::pos::{Dir, Pos};
use crate::registry::StateId;

/// `FireBlock.getFireTickDelay`.
fn tick_delay(level: &mut Level) -> i32 {
    30 + level.random.next_int_bounded(10)
}

/// `BaseFireBlock.getState` for fire on a sturdy floor: the plain state.
pub fn state_at(level: &mut Level, pos: Pos) -> StateId {
    let below = pos.offset(Dir::Down);
    if !is_face_sturdy(level, below, level.get(below), Dir::Up) {
        level.unsupported("fire without a sturdy floor");
    }
    level.default_state(level.block_id("fire"))
}

/// `BaseFireBlock.onPlace` and `FireBlock.onPlace`.
pub fn on_place(level: &mut Level, pos: Pos, state: StateId, old: StateId) {
    if level.block(old) != level.block(state) {
        if Dir::ALL.iter().any(|&d| level.reg.name(level.get(pos.offset(d))) == "minecraft:obsidian") {
            level.unsupported("fire beside obsidian, which could light a portal");
            return;
        }
        if !survives(level, pos) {
            level.remove_block(pos);
        }
    }
    let delay = tick_delay(level);
    level.schedule_tick(pos, level.block(state), delay, 0);
}

/// `FireBlock.canSurvive`, for fire with nothing flammable around it.
fn survives(level: &mut Level, pos: Pos) -> bool {
    let below = pos.offset(Dir::Down);
    let sturdy = is_face_sturdy(level, below, level.get(below), Dir::Up);
    if !sturdy {
        level.unsupported("fire without a sturdy floor");
    }
    sturdy
}

/// `FireBlock.tick`, with fire spread off.
pub fn tick(level: &mut Level, pos: Pos, state: StateId) {
    let delay = tick_delay(level);
    level.schedule_tick(pos, level.block(state), delay, 0);
}

/// `FireBlock.updateShape`.
pub fn update_shape(level: &mut Level, pos: Pos, state: StateId) -> StateId {
    if survives(level, pos) { state } else { level.air }
}

/// `BaseFireBlock.entityInside`: things catch fire and burn, which isn't
/// simulated; arrows and fireballs aren't harmed.
pub fn entity_inside(level: &mut Level) {
    let burning = level
        .entities
        .last()
        .is_some_and(|e| !matches!(e.data, crate::entity::Data::Arrow(_) | crate::entity::Data::SmallFireball));
    if burning {
        level.unsupported("an entity in fire");
    }
}
