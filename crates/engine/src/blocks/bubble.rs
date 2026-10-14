//! Bubble columns (`BubbleColumnBlock`), the soul sand and magma blocks
//! that make them, and cobwebs: blocks whose part here is how they move
//! entities.

use crate::level::{Level, flags};
use crate::physics::Vec3;
use crate::pos::{Dir, Pos};
use crate::registry::StateId;

use super::Kind;
use super::water::{self, Fluid};

/// How long soul sand and magma wait before building their column.
const SOURCE_DELAY: i32 = 20;
/// `BubbleColumnBlock`'s own check delay.
const COLUMN_DELAY: i32 = 5;

// --- soul sand and magma -------------------------------------------------------

/// `SoulSandBlock.updateShape` / `MagmaBlock.updateShape`: water arriving
/// on top starts a column. (Placing them schedules nothing in 26.3.)
pub fn source_update_shape(level: &mut Level, pos: Pos, state: StateId, dir: Dir, neighbor_state: StateId) -> StateId {
    if dir == Dir::Up && level.kind(neighbor_state) == Kind::Water {
        level.schedule_tick(pos, level.block(state), SOURCE_DELAY, 0);
    }
    state
}

/// `tick`: build the column above.
pub fn source_tick(level: &mut Level, pos: Pos, state: StateId) {
    update_column(level, pos.above(), state);
}

// --- the column ------------------------------------------------------------------

/// `BubbleColumnBlock.canExistIn`: a column or still water source.
fn can_exist_in(level: &Level, state: StateId) -> bool {
    level.kind(state) == Kind::BubbleColumn || level.kind(state) == Kind::Water && level.int(state, "level") == 0
}

/// `getColumnState`: what a column over `below` is.
fn column_state(level: &Level, below: StateId) -> StateId {
    let column = level.default_state(level.block_id("bubble_column"));
    match level.kind(below) {
        Kind::BubbleColumn => below,
        Kind::SoulSand => level.with_bool(column, "drag", false),
        Kind::Magma => level.with_bool(column, "drag", true),
        _ => level.default_state(level.block_id("water")),
    }
}

/// `BubbleColumnBlock.updateColumn(level, pos, below)`: the column from
/// `pos` up, through every source it reaches.
pub fn update_column(level: &mut Level, pos: Pos, below: StateId) {
    if !can_exist_in(level, level.get(pos)) {
        return;
    }
    let column = column_state(level, below);
    level.set_block(pos, column, flags::CLIENTS);
    let mut p = pos.above();
    while can_exist_in(level, level.get(p)) {
        if !level.set_block(p, column, flags::CLIENTS) {
            return;
        }
        p = p.above();
    }
}

/// `canSurvive`: on soul sand, magma or more column.
fn can_survive(level: &Level, pos: Pos) -> bool {
    matches!(level.kind_at(pos.below()), Kind::BubbleColumn | Kind::SoulSand | Kind::Magma)
}

/// `BubbleColumnBlock.updateShape`.
pub fn column_update_shape(level: &mut Level, pos: Pos, state: StateId, dir: Dir, neighbor_state: StateId) -> StateId {
    level.schedule_fluid_tick(pos, Fluid::Water, water::TICK_DELAY);
    if !can_survive(level, pos)
        || dir == Dir::Down
        || dir == Dir::Up && level.kind(neighbor_state) != Kind::BubbleColumn && can_exist_in(level, neighbor_state)
    {
        level.schedule_tick(pos, level.block(state), COLUMN_DELAY, 0);
    }
    state
}

/// `BubbleColumnBlock.tick`.
pub fn column_tick(level: &mut Level, pos: Pos, state: StateId) {
    let _ = state;
    let below = level.get(pos.below());
    update_column(level, pos, below);
}

/// `BubbleColumnBlock.entityInside`: lift or drag, harder at the open top.
pub fn column_entity_inside(level: &mut Level, k: usize, pos: Pos, state: StateId) {
    let drag = level.flag(state, "drag");
    let above = level.get(pos.above());
    let open = level.physics().shape(above).is_empty() && water::fluid_state(level, above).is_empty();
    let e = &mut level.entities[k];
    let m = e.motion;
    if open {
        // `Entity.onAboveBubbleColumn`.
        let y = if drag { (m.y - 0.03).max(-0.9) } else { (m.y + 0.1).min(1.8) };
        e.motion = Vec3::new(m.x, y, m.z);
        // `sendBubbleColumnParticles`: splash and bubble positions, twice.
        for _ in 0..8 {
            level.random.next_double();
        }
    } else {
        // `Entity.onInsideBubbleColumn`.
        let y = if drag { (m.y - 0.03).max(-0.3) } else { (m.y + 0.06).min(0.7) };
        e.motion = Vec3::new(m.x, y, m.z);
    }
}

// --- cobweb ------------------------------------------------------------------------

/// `WebBlock.entityInside`: `makeStuckInBlock` at a quarter speed
/// sideways and a twentieth vertically.
pub fn web_entity_inside(level: &mut Level, k: usize) {
    level.entities[k].stuck_speed_multiplier = Vec3::new(0.25, 0.05f32 as f64, 0.25);
}
