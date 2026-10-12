//! Block behaviour: what each kind of block does when it is placed, removed,
//! updated, ticked or used, and what signals it gives.
//!
//! Each function mirrors the game method of the same name and dispatches on
//! [`Kind`]. Blocks the engine does not simulate yet are [`Kind::Other`];
//! [`crate::run::run`] refuses fixtures that use them rather than running
//! them wrong.

// Dispatch tables grow one arm per block kind as blocks are added.
#![allow(clippy::single_match, clippy::match_single_binding)]

mod button;
mod lamp;
mod lever;
mod piston;
mod repeater;
mod torch;
mod wire;

use crate::level::Level;
use crate::pos::{Dir, Pos};
use crate::registry::{BlockId, StateId};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Air,
    /// A full, opaque block with no behaviour: stone, obsidian and the like.
    Solid,
    Lever,
    /// A button arrows cannot press: stone or polished blackstone.
    StoneButton,
    /// Full, see-through, not a conductor: glass and stained glass.
    Glass,
    Slab,
    RedstoneBlock,
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
            n if n.ends_with("_concrete") => Kind::Solid,
            "lever" => Kind::Lever,
            "stone_button" | "polished_blackstone_button" => Kind::StoneButton,
            n if n == "glass" || n == "tinted_glass" || n.ends_with("_stained_glass") => Kind::Glass,
            "stone_slab" | "smooth_stone_slab" | "cobblestone_slab" | "oak_slab" => Kind::Slab,
            "redstone_block" => Kind::RedstoneBlock,
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
    /// `PistonMovingBlockEntity`.
    Piston(piston::Moving),
}

impl BlockEntity {
    pub fn ticks(&self) -> bool {
        match self {
            BlockEntity::Chest => false,
            BlockEntity::Piston(_) => true,
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

pub fn tick_block_entity(level: &mut Level, pos: Pos) {
    if let Some(BlockEntity::Piston(_)) = level.block_entities.get(&pos) {
        piston::tick_moving(level, pos);
    }
}

pub fn has_analog_output(_level: &Level, _state: StateId) -> bool {
    false
}

// --- signals -----------------------------------------------------------------

/// `isRedstoneConductor`.
pub fn is_conductor(level: &Level, _pos: Pos, state: StateId) -> bool {
    // Pistons are never conductors, extended or not.
    match level.kind(state) {
        Kind::Solid | Kind::RedstoneLamp | Kind::Slime => true,
        Kind::Slab => level.prop(state, "type") == "double",
        _ => false,
    }
}

/// `isSignalSource`.
pub fn is_signal_source(level: &Level, state: StateId) -> bool {
    match level.kind(state) {
        Kind::RedstoneWire => level.wire_signals.get(),
        k => matches!(
            k,
            Kind::Lever
                | Kind::StoneButton
                | Kind::RedstoneBlock
                | Kind::RedstoneTorch
                | Kind::RedstoneWallTorch
                | Kind::Repeater
        ),
    }
}

/// `isFaceSturdy(level, pos, dir)` for the blocks phase 1 uses: whether the
/// block's support shape covers its whole `dir` face.
pub fn is_face_sturdy(level: &Level, pos: Pos, state: StateId, dir: Dir) -> bool {
    match level.kind(state) {
        Kind::Solid | Kind::RedstoneLamp | Kind::RedstoneBlock | Kind::Slime | Kind::Glass => true,
        Kind::Slab => match level.prop(state, "type") {
            "double" => true,
            "top" => dir == Dir::Up,
            _ => dir == Dir::Down,
        },
        // An extended base is a 12-pixel slab: only its back face is whole.
        Kind::Piston | Kind::StickyPiston => {
            !level.flag(state, "extended") || level.dir(state, "facing").opposite() == dir
        }
        Kind::PistonHead => level.dir(state, "facing") == dir,
        Kind::MovingPiston => piston::moving_face_sturdy(level, pos, dir),
        _ => false,
    }
}

/// `getSignal(state, level, pos, dir)`: weak power out of `pos`, as seen by
/// the block on its `dir.opposite()` side.
pub fn get_signal(level: &Level, pos: Pos, state: StateId, dir: Dir) -> i32 {
    match level.kind(state) {
        Kind::Lever | Kind::StoneButton => lever::get_signal(level, pos, state, dir),
        Kind::RedstoneBlock => 15,
        Kind::Repeater => repeater::get_signal(level, pos, state, dir),
        Kind::RedstoneWire => wire::get_signal(level, pos, state, dir),
        Kind::RedstoneTorch | Kind::RedstoneWallTorch => torch::get_signal(level, pos, state, dir),
        _ => 0,
    }
}

/// `getDirectSignal`: strong power.
pub fn get_direct_signal(level: &Level, pos: Pos, state: StateId, dir: Dir) -> i32 {
    match level.kind(state) {
        Kind::Lever | Kind::StoneButton => lever::get_direct_signal(level, pos, state, dir),
        Kind::RedstoneWire => wire::get_direct_signal(level, pos, state, dir),
        Kind::Repeater => repeater::get_signal(level, pos, state, dir),
        Kind::RedstoneTorch | Kind::RedstoneWallTorch => torch::get_direct_signal(level, pos, state, dir),
        _ => 0,
    }
}

// --- lifecycle ---------------------------------------------------------------

pub fn on_place(level: &mut Level, pos: Pos, state: StateId, old: StateId, moved_by_piston: bool) {
    let _ = moved_by_piston;
    match level.kind(state) {
        Kind::RedstoneWire => wire::on_place(level, pos, state, old),
        Kind::Repeater => repeater::on_place(level, pos, state),
        Kind::Piston | Kind::StickyPiston => piston::on_place(level, pos, state, old),
        Kind::RedstoneTorch | Kind::RedstoneWallTorch => torch::on_place(level, pos, state),
        _ => {}
    }
}

pub fn affect_neighbors_after_removal(level: &mut Level, pos: Pos, state: StateId, moved_by_piston: bool) {
    match level.kind(state) {
        Kind::Lever => lever::affect_neighbors_after_removal(level, pos, state, moved_by_piston),
        Kind::StoneButton => button::affect_neighbors_after_removal(level, pos, state, moved_by_piston),
        Kind::RedstoneWire => wire::affect_neighbors_after_removal(level, pos, state, moved_by_piston),
        Kind::Repeater => repeater::affect_neighbors_after_removal(level, pos, state, moved_by_piston),
        Kind::PistonHead => piston::head_affect_neighbors_after_removal(level, pos, state),
        Kind::RedstoneTorch | Kind::RedstoneWallTorch => {
            torch::affect_neighbors_after_removal(level, pos, state, moved_by_piston)
        }
        _ => {}
    }
}

pub fn neighbor_changed(level: &mut Level, pos: Pos, state: StateId, source: BlockId, moved_by_piston: bool) {
    let _ = moved_by_piston;
    match level.kind(state) {
        Kind::RedstoneLamp => lamp::neighbor_changed(level, pos, state),
        Kind::RedstoneWire => wire::neighbor_changed(level, pos, state),
        Kind::Repeater => repeater::neighbor_changed(level, pos, state),
        Kind::Piston | Kind::StickyPiston => piston::neighbor_changed(level, pos, state),
        Kind::PistonHead => piston::head_neighbor_changed(level, pos, state, source),
        Kind::RedstoneTorch | Kind::RedstoneWallTorch => torch::neighbor_changed(level, pos, state),
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
    match level.kind(state) {
        Kind::RedstoneWire => wire::update_shape(level, pos, state, dir, neighbor, neighbor_state),
        Kind::Repeater => repeater::update_shape(level, pos, state, dir, neighbor, neighbor_state),
        Kind::PistonHead => piston::head_update_shape(level, pos, state, dir),
        Kind::Lever | Kind::StoneButton => face_attached_update_shape(level, pos, state, dir),
        Kind::RedstoneTorch | Kind::RedstoneWallTorch => torch::update_shape(level, pos, state, dir),
        _ => state,
    }
}

/// `updateIndirectNeighbourShapes`: only dust has any.
pub fn update_indirect_neighbour_shapes(level: &mut Level, pos: Pos, state: StateId, flags: i32, recursion: i32) {
    if level.kind(state) == Kind::RedstoneWire {
        wire::update_indirect_neighbour_shapes(level, pos, state, flags, recursion);
    }
}

pub fn tick(level: &mut Level, pos: Pos, state: StateId) {
    match level.kind(state) {
        Kind::RedstoneLamp => lamp::tick(level, pos, state),
        Kind::RedstoneTorch | Kind::RedstoneWallTorch => torch::tick(level, pos, state),
        Kind::Repeater => repeater::tick(level, pos, state),
        Kind::StoneButton => button::tick(level, pos, state),
        _ => {}
    }
}

pub fn trigger_event(level: &mut Level, pos: Pos, state: StateId, a: i32, b: i32) -> bool {
    match level.kind(state) {
        Kind::Piston | Kind::StickyPiston => piston::trigger_event(level, pos, state, a, b),
        _ => false,
    }
}

/// Right-click with an empty hand (`useWithoutItem`).
pub fn use_without_item(level: &mut Level, pos: Pos, state: StateId) {
    match level.kind(state) {
        Kind::Lever => lever::pull(level, pos, state),
        Kind::StoneButton => button::press(level, pos, state),
        Kind::Repeater => repeater::use_without_item(level, pos, state),
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

/// `FaceAttachedHorizontalDirectionalBlock.updateShape`: a lever or button
/// whose support loses its sturdy face pops off.
fn face_attached_update_shape(level: &Level, pos: Pos, state: StateId, dir: Dir) -> StateId {
    let support = connected_direction(level, state).opposite();
    if dir == support {
        let p = pos.offset(support);
        if !is_face_sturdy(level, p, level.get(p), support.opposite()) {
            return level.air;
        }
    }
    state
}
