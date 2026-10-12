//! Block behaviour: what each kind of block does when it is placed, removed,
//! updated, ticked or used, and what signals it gives.
//!
//! Each function mirrors the game method of the same name and dispatches on
//! [`Kind`]. Blocks the engine does not simulate yet are [`Kind::Other`] and
//! behave as inert, non-conducting blocks.

// Dispatch tables grow one arm per block kind as blocks are added.
#![allow(clippy::single_match, clippy::match_single_binding)]

mod lamp;
mod lever;

use crate::level::Level;
use crate::pos::{Dir, Pos};
use crate::registry::{BlockId, StateId};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Air,
    /// A full, opaque block with no behaviour: stone, obsidian and the like.
    Solid,
    Lever,
    RedstoneLamp,
    RedstoneWire,
    RedstoneTorch,
    RedstoneWallTorch,
    Repeater,
    Piston,
    StickyPiston,
    PistonHead,
    MovingPiston,
    Slime,
    Honey,
    Chest,
    Other,
}

impl Kind {
    pub fn of(name: &str) -> Kind {
        match name.strip_prefix("minecraft:").unwrap_or(name) {
            "air" | "cave_air" | "void_air" => Kind::Air,
            "stone" | "obsidian" | "cobblestone" | "dirt" | "smooth_stone" | "white_wool" | "iron_block"
            | "gold_block" | "diamond_block" | "bedrock" => Kind::Solid,
            "lever" => Kind::Lever,
            "redstone_lamp" => Kind::RedstoneLamp,
            "redstone_wire" => Kind::RedstoneWire,
            "redstone_torch" => Kind::RedstoneTorch,
            "redstone_wall_torch" => Kind::RedstoneWallTorch,
            "repeater" => Kind::Repeater,
            "piston" => Kind::Piston,
            "sticky_piston" => Kind::StickyPiston,
            "piston_head" => Kind::PistonHead,
            "moving_piston" => Kind::MovingPiston,
            "slime_block" => Kind::Slime,
            "honey_block" => Kind::Honey,
            "chest" | "trapped_chest" => Kind::Chest,
            _ => Kind::Other,
        }
    }
}

/// Block entity data. Only what phase 1 needs.
#[derive(Clone, Debug)]
pub enum BlockEntity {
    /// A chest: present only because it makes the block immovable.
    Chest,
}

impl BlockEntity {
    pub fn ticks(&self) -> bool {
        match self {
            BlockEntity::Chest => false,
        }
    }
}

pub fn has_block_entity(kind: Kind) -> bool {
    matches!(kind, Kind::Chest | Kind::MovingPiston)
}

pub fn new_block_entity(_level: &Level, _pos: Pos, state: StateId) -> Option<BlockEntity> {
    match _level.kind(state) {
        Kind::Chest => Some(BlockEntity::Chest),
        _ => None,
    }
}

pub fn tick_block_entity(_level: &mut Level, _pos: Pos) {}

pub fn has_analog_output(_level: &Level, _state: StateId) -> bool {
    false
}

// --- signals -----------------------------------------------------------------

/// `isRedstoneConductor`.
pub fn is_conductor(level: &Level, _pos: Pos, state: StateId) -> bool {
    match level.kind(state) {
        Kind::Solid | Kind::RedstoneLamp | Kind::Slime => true,
        Kind::Piston | Kind::StickyPiston => !level.flag(state, "extended"),
        _ => false,
    }
}

/// `isSignalSource`.
pub fn is_signal_source(level: &Level, state: StateId) -> bool {
    matches!(
        level.kind(state),
        Kind::Lever | Kind::RedstoneWire | Kind::RedstoneTorch | Kind::RedstoneWallTorch | Kind::Repeater
    )
}

/// `getSignal(state, level, pos, dir)`: weak power out of `pos`, as seen by
/// the block on its `dir.opposite()` side.
pub fn get_signal(level: &Level, pos: Pos, state: StateId, dir: Dir) -> i32 {
    match level.kind(state) {
        Kind::Lever => lever::get_signal(level, pos, state, dir),
        _ => 0,
    }
}

/// `getDirectSignal`: strong power.
pub fn get_direct_signal(level: &Level, pos: Pos, state: StateId, dir: Dir) -> i32 {
    match level.kind(state) {
        Kind::Lever => lever::get_direct_signal(level, pos, state, dir),
        _ => 0,
    }
}

// --- lifecycle ---------------------------------------------------------------

pub fn on_place(level: &mut Level, pos: Pos, state: StateId, old: StateId, moved_by_piston: bool) {
    let _ = (level, pos, state, old, moved_by_piston);
}

pub fn affect_neighbors_after_removal(level: &mut Level, pos: Pos, state: StateId, moved_by_piston: bool) {
    match level.kind(state) {
        Kind::Lever => lever::affect_neighbors_after_removal(level, pos, state, moved_by_piston),
        _ => {}
    }
}

pub fn neighbor_changed(level: &mut Level, pos: Pos, state: StateId, source: BlockId, moved_by_piston: bool) {
    let _ = (source, moved_by_piston);
    match level.kind(state) {
        Kind::RedstoneLamp => lamp::neighbor_changed(level, pos, state),
        _ => {}
    }
}

pub fn update_shape(
    level: &mut Level,
    pos: Pos,
    state: StateId,
    dir: Dir,
    neighbor: Pos,
    neighbor_state: StateId,
) -> StateId {
    let _ = (pos, dir, neighbor, neighbor_state);
    match level.kind(state) {
        _ => state,
    }
}

/// `updateIndirectNeighbourShapes`: only dust has any.
pub fn update_indirect_neighbour_shapes(level: &mut Level, pos: Pos, state: StateId, flags: i32, recursion: i32) {
    let _ = (level, pos, state, flags, recursion);
}

pub fn tick(level: &mut Level, pos: Pos, state: StateId) {
    match level.kind(state) {
        Kind::RedstoneLamp => lamp::tick(level, pos, state),
        _ => {}
    }
}

pub fn trigger_event(level: &mut Level, pos: Pos, state: StateId, a: i32, b: i32) -> bool {
    let _ = (level, pos, state, a, b);
    false
}

/// Right-click with an empty hand (`useWithoutItem`).
pub fn use_without_item(level: &mut Level, pos: Pos, state: StateId) {
    match level.kind(state) {
        Kind::Lever => lever::pull(level, pos, state),
        _ => {}
    }
}

// --- shared helpers ------------------------------------------------------------

/// `FaceAttachedHorizontalDirectionalBlock.getConnectedDirection`: the
/// direction from the block toward what it is attached to, reversed.
pub fn connected_direction(level: &Level, state: StateId) -> Dir {
    match level.prop(state, "face") {
        "ceiling" => Dir::Down,
        "floor" => Dir::Up,
        _ => level.dir(state, "facing"),
    }
}
