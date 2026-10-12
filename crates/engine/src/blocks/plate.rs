//! Pressure plates (`BasePressurePlateBlock`): wooden and stone plates,
//! which are on or off, and weighted plates, whose power counts entities.

use super::Kind;
use crate::entity;
use crate::game_event::GameEvent;
use crate::level::{Level, flags};
use crate::physics::Aabb;
use crate::pos::{Dir, Pos};
use crate::registry::StateId;

/// `BasePressurePlateBlock.TOUCH_AABB`.
const TOUCH: Aabb = Aabb { min: [0.0625, 0.0, 0.0625], max: [0.9375, 0.25, 0.9375] };

fn weighted(level: &Level, state: StateId) -> bool {
    level.kind(state) == Kind::WeightedPlate
}

/// `getSignalForState`.
pub fn signal(level: &Level, state: StateId) -> i32 {
    if weighted(level, state) {
        level.int(state, "power")
    } else if level.flag(state, "powered") {
        15
    } else {
        0
    }
}

fn with_signal(level: &Level, state: StateId, s: i32) -> StateId {
    if weighted(level, state) { level.with_int(state, "power", s) } else { level.with_bool(state, "powered", s > 0) }
}

/// `getPressedTime`.
fn pressed_time(level: &Level, state: StateId) -> i32 {
    if weighted(level, state) { 10 } else { 20 }
}

/// `getSignalStrength`: what the entities on the plate add up to.
fn signal_strength(level: &Level, pos: Pos, state: StateId) -> i32 {
    let b = TOUCH.offset(pos.x as f64, pos.y as f64, pos.z as f64);
    let name = level.reg.name(state);
    // Stone plates only count mobs, and the engine has none.
    let counts = |e: &entity::Entity| !name.contains("stone_pressure_plate") || e.is_living();
    let n = level.entities.iter().filter(|e| !e.removed && counts(e) && e.bounding_box().intersects(&b)).count() as i32;
    if weighted(level, state) {
        let max = if name == "minecraft:heavy_weighted_pressure_plate" { 150 } else { 15 };
        let n = n.min(max);
        if n > 0 { ((n.min(max) as f32 / max as f32) * 15.0).ceil() as i32 } else { 0 }
    } else if n > 0 {
        15
    } else {
        0
    }
}

/// `entityInside`: an unpressed plate checks as soon as something touches it.
pub fn entity_inside(level: &mut Level, pos: Pos, state: StateId) {
    let s = signal(level, state);
    if s == 0 {
        check_pressed(level, pos, state, s);
    }
}

/// `tick`: a pressed plate checks again until nothing is on it.
pub fn tick(level: &mut Level, pos: Pos, state: StateId) {
    let s = signal(level, state);
    if s > 0 {
        check_pressed(level, pos, state, s);
    }
}

/// `checkPressed`.
fn check_pressed(level: &mut Level, pos: Pos, state: StateId, old: i32) {
    let new = signal_strength(level, pos, state);
    if old != new {
        let s = with_signal(level, state, new);
        level.set_block(pos, s, flags::CLIENTS);
        update_neighbours(level, pos, s);
    }
    if new == 0 && old > 0 {
        level.game_event(GameEvent::BlockDeactivate, pos);
    } else if new > 0 && old == 0 {
        level.game_event(GameEvent::BlockActivate, pos);
    }
    if new > 0 {
        level.schedule_tick(pos, level.block(state), pressed_time(level, state), 0);
    }
}

fn update_neighbours(level: &mut Level, pos: Pos, state: StateId) {
    let block = level.block(state);
    level.update_neighbors_at(pos, block);
    level.update_neighbors_at(pos.below(), block);
}

pub fn affect_neighbors_after_removal(level: &mut Level, pos: Pos, state: StateId, moved_by_piston: bool) {
    if !moved_by_piston && signal(level, state) > 0 {
        update_neighbours(level, pos, state);
    }
}

pub fn get_direct_signal(level: &Level, state: StateId, dir: Dir) -> i32 {
    if dir == Dir::Up { signal(level, state) } else { 0 }
}

/// `updateShape`: a plate whose support goes pops off.
pub fn update_shape(level: &Level, pos: Pos, state: StateId, dir: Dir) -> StateId {
    let below = pos.below();
    if dir == Dir::Down && !super::is_face_sturdy(level, below, level.get(below), Dir::Up) { level.air } else { state }
}
