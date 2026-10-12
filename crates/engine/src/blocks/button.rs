//! `ButtonBlock`. Stone and polished blackstone buttons stay pressed for 20
//! game ticks, wooden ones for 30. Wooden buttons are also pressed, and kept
//! pressed, by an arrow inside them.

use super::connected_direction;
use crate::entity::Data;
use crate::game_event::GameEvent;
use crate::level::{Level, flags};
use crate::physics::Aabb;
use crate::pos::{Dir, Pos};
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
    level.game_event(GameEvent::BlockActivate, pos);
}

/// `BlockSetType.canButtonBeActivatedByArrows`: every wooden set.
fn arrows_can_press(level: &Level, state: StateId) -> bool {
    !matches!(level.reg.name(state), "minecraft:stone_button" | "minecraft:polished_blackstone_button")
}

/// `entityInside`: an arrow presses an unpressed wooden button.
pub fn entity_inside(level: &mut Level, pos: Pos, state: StateId) {
    if arrows_can_press(level, state) && !level.flag(state, "powered") {
        check_pressed(level, pos, state);
    }
}

/// `tick`: a pressed button lets go unless an arrow still holds it.
pub fn tick(level: &mut Level, pos: Pos, state: StateId) {
    if level.flag(state, "powered") {
        check_pressed(level, pos, state);
    }
}

/// `checkPressed`.
fn check_pressed(level: &mut Level, pos: Pos, state: StateId) {
    let pressed = arrows_can_press(level, state) && {
        let b = shape(level, state).offset(pos.x as f64, pos.y as f64, pos.z as f64);
        level.entities.iter().any(|e| !e.removed && matches!(e.data, Data::Arrow(_)) && e.bounding_box().intersects(&b))
    };
    if pressed != level.flag(state, "powered") {
        level.set_block(pos, level.with_bool(state, "powered", pressed), flags::ALL);
        update_neighbours(level, pos, state);
        level.game_event(if pressed { GameEvent::BlockActivate } else { GameEvent::BlockDeactivate }, pos);
    }
    if pressed {
        level.schedule_tick(pos, level.block(state), ticks_to_stay_pressed(level, state), 0);
    }
}

/// The button's outline (`getShape`): 6 by 4 pixels, 2 deep, or 1 deep
/// while pressed, against the block it hangs on.
fn shape(level: &Level, state: StateId) -> Aabb {
    let px = |v: f64| v / 16.0;
    let depth = if level.flag(state, "powered") { 1.0 } else { 2.0 };
    let facing = level.dir(state, "facing");
    let x_axis = matches!(facing, Dir::East | Dir::West);
    let (lo, hi) = (px(5.0), px(11.0));
    let (lo4, hi4) = (px(6.0), px(10.0));
    match level.prop(state, "face") {
        "floor" | "ceiling" => {
            let (y0, y1) = if level.prop(state, "face") == "floor" { (0.0, px(depth)) } else { (1.0 - px(depth), 1.0) };
            if x_axis { Aabb::new(lo4, y0, lo, hi4, y1, hi) } else { Aabb::new(lo, y0, lo4, hi, y1, hi4) }
        }
        _ => match facing {
            Dir::North => Aabb::new(lo, lo4, 1.0 - px(depth), hi, hi4, 1.0),
            Dir::South => Aabb::new(lo, lo4, 0.0, hi, hi4, px(depth)),
            Dir::West => Aabb::new(1.0 - px(depth), lo4, lo, 1.0, hi4, hi),
            _ => Aabb::new(0.0, lo4, lo, px(depth), hi4, hi),
        },
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
