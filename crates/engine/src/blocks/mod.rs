//! Block behaviour: what each kind of block does when it is placed, removed,
//! updated, ticked or used, and what signals it gives.
//!
//! Each function mirrors the game method of the same name and dispatches on
//! [`Kind`]; which blocks have which kind is up to the version's rule module
//! ([`crate::rules`]). Blocks the engine does not simulate yet are [`Kind::Other`];
//! [`crate::run::run`] refuses fixtures that use them rather than running
//! them wrong.

// Dispatch tables grow one arm per block kind as blocks are added.
#![allow(clippy::single_match, clippy::match_single_binding)]

mod bubble;
mod bulb;
mod button;
mod comparator;
pub mod container;
pub mod crafter;
pub mod daylight;
pub mod dispenser;
mod door;
mod falling;
mod lamp;
pub mod lectern;
pub mod plate;
mod sculk;
mod shelf;
pub use sculk::post as post_game_event;
mod lever;
mod note;
mod observer;
mod piston;
pub mod placement;
mod plain;
pub(crate) mod rail;
mod repeater;
mod torch;
mod tripwire;
pub mod water;
mod wire;

use crate::level::Level;
use crate::orientation::Orientation;
use crate::pos::{Dir, Pos};
use crate::registry::{BlockId, StateId};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Air,
    /// A full, opaque block with no behaviour: stone, obsidian and the like.
    Solid,
    Lever,
    /// Any button; how long it stays pressed depends on its material.
    Button,
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
    Observer,
    Comparator,
    /// Blocks whose only behaviour here is an analog output read from their
    /// state: composters and cakes.
    Composter,
    Cake,
    NoteBlock,
    Door,
    Trapdoor,
    /// Powered and activator rails.
    PoweredRail,
    /// Plain rails, which can bend.
    Rail,
    /// Water, source or flowing.
    Water,
    Hopper,
    Dropper,
    Dispenser,
    /// A barrel: a 27-slot container that is a full block.
    Barrel,
    PowderSnow,
    /// Logs and wood: plain full blocks that leaves count distance from.
    Log,
    Leaves,
    Target,
    Jukebox,
    ShulkerBox,
    TripwireHook,
    /// Waxed copper bulbs (unwaxed ones oxidize on random ticks).
    CopperBulb,
    Bell,
    Crafter,
    Lectern,
    DaylightDetector,
    Tripwire,
    SculkSensor,
    CalibratedSculkSensor,
    Shelf,
    /// Sand, gravel and the like: they fall when nothing holds them up.
    FallingBlock,
    Tnt,
    /// Plain torches, which only need support.
    Torch,
    WallTorch,
    /// Wooden and stone pressure plates: on or off.
    PressurePlate,
    /// Light and heavy weighted pressure plates: power from how many
    /// entities are on them.
    WeightedPlate,
    /// Soul sand and magma: bubble column sources.
    SoulSand,
    Magma,
    BubbleColumn,
    Cobweb,
    Other,
}

/// Block entity data. Only what phase 1 needs.
#[derive(Clone, Debug)]
pub enum BlockEntity {
    /// `ComparatorBlockEntity`: the output strength.
    Comparator(i32),
    /// A chest's or barrel's 27 slots.
    Chest(Vec<container::Stack>),
    Hopper(container::Hopper),
    /// A dropper's or dispenser's 9 slots.
    Dropper(Vec<container::Stack>),
    /// `BellBlockEntity`: only its ticker's place in the list matters here.
    Bell,
    /// A shulker box's 27 slots. Its ticker only animates the lid.
    ShulkerBox(Vec<container::Stack>),
    /// An empty jukebox, which has no ticker.
    Jukebox,
    /// `PistonMovingBlockEntity`.
    Piston(piston::Moving),
    Crafter(crafter::Crafter),
    Lectern(lectern::Lectern),
    DaylightDetector,
    Sculk(sculk::Sensor),
    /// A shelf's 3 slots.
    Shelf(Vec<container::Stack>),
}

impl BlockEntity {
    pub fn ticks(&self) -> bool {
        match self {
            BlockEntity::Chest(_)
            | BlockEntity::Dropper(_)
            | BlockEntity::Comparator(_)
            | BlockEntity::Jukebox
            | BlockEntity::Lectern(_)
            | BlockEntity::Shelf(_) => false,
            BlockEntity::Piston(_)
            | BlockEntity::Hopper(_)
            | BlockEntity::Bell
            | BlockEntity::ShulkerBox(_)
            | BlockEntity::Crafter(_)
            | BlockEntity::DaylightDetector
            | BlockEntity::Sculk(_) => true,
        }
    }
}

pub fn has_block_entity(kind: Kind) -> bool {
    matches!(
        kind,
        Kind::Chest
            | Kind::Barrel
            | Kind::MovingPiston
            | Kind::Comparator
            | Kind::Hopper
            | Kind::Dropper
            | Kind::Dispenser
            | Kind::Bell
            | Kind::ShulkerBox
            | Kind::Jukebox
            | Kind::Crafter
            | Kind::Lectern
            | Kind::DaylightDetector
            | Kind::SculkSensor
            | Kind::CalibratedSculkSensor
            | Kind::Shelf
    )
}

pub fn new_block_entity(_level: &Level, _pos: Pos, state: StateId) -> Option<BlockEntity> {
    match _level.kind(state) {
        Kind::Chest | Kind::Barrel => Some(container::new_chest()),
        Kind::Hopper => Some(BlockEntity::Hopper(container::Hopper::new())),
        Kind::Dropper | Kind::Dispenser => Some(container::new_dropper()),
        Kind::Bell => Some(BlockEntity::Bell),
        Kind::ShulkerBox => Some(BlockEntity::ShulkerBox(vec![container::Stack::default(); 27])),
        Kind::Jukebox => Some(BlockEntity::Jukebox),
        Kind::Comparator => Some(BlockEntity::Comparator(0)),
        Kind::Crafter => Some(BlockEntity::Crafter(crafter::Crafter::new())),
        Kind::Lectern => Some(BlockEntity::Lectern(lectern::Lectern::default())),
        Kind::DaylightDetector => Some(BlockEntity::DaylightDetector),
        Kind::Shelf => Some(BlockEntity::Shelf(vec![container::Stack::default(); 3])),
        Kind::SculkSensor | Kind::CalibratedSculkSensor => Some(BlockEntity::Sculk(sculk::Sensor::default())),
        _ => None,
    }
}

pub fn tick_block_entity(level: &mut Level, pos: Pos) {
    match level.block_entities.get(&pos) {
        Some(BlockEntity::Piston(_)) => piston::tick_moving(level, pos),
        Some(BlockEntity::Hopper(_)) => container::push_items_tick(level, pos),
        Some(BlockEntity::Crafter(_)) => crafter::tick_entity(level, pos),
        Some(BlockEntity::DaylightDetector) => daylight::tick_entity(level, pos),
        Some(BlockEntity::Sculk(_)) => sculk::tick_entity(level, pos),
        _ => {}
    }
}

/// `hasAnalogOutputSignal`.
pub fn has_analog_output(level: &Level, state: StateId) -> bool {
    matches!(
        level.kind(state),
        Kind::Chest
            | Kind::Barrel
            | Kind::Composter
            | Kind::Cake
            | Kind::Hopper
            | Kind::Dropper
            | Kind::Dispenser
            | Kind::ShulkerBox
            | Kind::Jukebox
            | Kind::CopperBulb
            | Kind::Crafter
            | Kind::Lectern
            | Kind::SculkSensor
            | Kind::CalibratedSculkSensor
            | Kind::Shelf
    )
}

/// `getAnalogOutputSignal`.
pub fn analog_output(level: &Level, pos: Pos, state: StateId) -> i32 {
    match level.kind(state) {
        Kind::Chest | Kind::Barrel | Kind::Hopper | Kind::Dropper | Kind::Dispenser | Kind::ShulkerBox => {
            container::analog_output(level, pos, state)
        }
        Kind::CopperBulb => bulb::bulb_analog_output(level, state),
        Kind::Crafter => crafter::analog_output(level, pos),
        Kind::Lectern => lectern::analog_output(level, pos, state),
        Kind::Shelf => shelf::analog_output(level, pos),
        Kind::SculkSensor | Kind::CalibratedSculkSensor => sculk::analog_output(level, pos, state),
        Kind::Composter => level.int(state, "level"),
        Kind::Cake => (7 - level.int(state, "bites")) * 2,
        _ => 0,
    }
}

// --- signals -----------------------------------------------------------------

/// `isRedstoneConductor`.
pub fn is_conductor(level: &Level, _pos: Pos, state: StateId) -> bool {
    // Pistons are never conductors, extended or not.
    match level.kind(state) {
        Kind::Solid
        | Kind::RedstoneLamp
        | Kind::Slime
        | Kind::NoteBlock
        | Kind::Dropper
        | Kind::Dispenser
        | Kind::Barrel
        | Kind::Log
        | Kind::Target
        | Kind::Jukebox
        | Kind::ShulkerBox
        | Kind::Crafter
        | Kind::FallingBlock
        | Kind::Tnt
        | Kind::SoulSand
        | Kind::Magma => true,
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
                | Kind::Button
                | Kind::RedstoneBlock
                | Kind::RedstoneTorch
                | Kind::RedstoneWallTorch
                | Kind::Repeater
                | Kind::Observer
                | Kind::Comparator
                | Kind::Target
                | Kind::Jukebox
                | Kind::TripwireHook
                | Kind::Lectern
                | Kind::DaylightDetector
                | Kind::SculkSensor
                | Kind::CalibratedSculkSensor
                | Kind::PressurePlate
                | Kind::WeightedPlate
        ),
    }
}

/// `isFaceSturdy(level, pos, dir)` for the blocks phase 1 uses: whether the
/// block's support shape covers its whole `dir` face.
pub fn is_face_sturdy(level: &Level, pos: Pos, state: StateId, dir: Dir) -> bool {
    match level.kind(state) {
        Kind::Solid
        | Kind::RedstoneLamp
        | Kind::RedstoneBlock
        | Kind::Slime
        | Kind::Glass
        | Kind::Observer
        | Kind::NoteBlock
        | Kind::Dropper
        | Kind::Dispenser
        | Kind::Barrel
        | Kind::PowderSnow
        | Kind::Log
        | Kind::Target
        | Kind::Jukebox
        | Kind::ShulkerBox
        | Kind::CopperBulb
        | Kind::Crafter
        | Kind::FallingBlock
        | Kind::Tnt
        | Kind::SoulSand
        | Kind::Magma => true,
        Kind::Trapdoor => door::trapdoor_face_sturdy(level, state, dir),
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
        Kind::Lever | Kind::Button => lever::get_signal(level, pos, state, dir),
        Kind::RedstoneBlock => 15,
        Kind::Target => plain::target_signal(level, state),
        Kind::TripwireHook => plain::hook_signal(level, state),
        Kind::Lectern => lectern::get_signal(level, state),
        Kind::PressurePlate | Kind::WeightedPlate => plate::signal(level, state),
        Kind::DaylightDetector => daylight::get_signal(level, state),
        Kind::SculkSensor | Kind::CalibratedSculkSensor => sculk::get_signal(level, state, dir),
        Kind::Observer => observer::get_signal(level, state, dir),
        Kind::Comparator => comparator::get_signal(level, pos, state, dir),
        Kind::Repeater => repeater::get_signal(level, pos, state, dir),
        Kind::RedstoneWire => wire::get_signal(level, pos, state, dir),
        Kind::RedstoneTorch | Kind::RedstoneWallTorch => torch::get_signal(level, pos, state, dir),
        _ => 0,
    }
}

/// `getDirectSignal`: strong power.
pub fn get_direct_signal(level: &Level, pos: Pos, state: StateId, dir: Dir) -> i32 {
    match level.kind(state) {
        Kind::Lever | Kind::Button => lever::get_direct_signal(level, pos, state, dir),
        Kind::RedstoneWire => wire::get_direct_signal(level, pos, state, dir),
        Kind::Repeater => repeater::get_signal(level, pos, state, dir),
        Kind::Observer => observer::get_signal(level, state, dir),
        Kind::Comparator => comparator::get_signal(level, pos, state, dir),
        Kind::TripwireHook => plain::hook_direct_signal(level, state, dir),
        Kind::Lectern => lectern::get_direct_signal(level, state, dir),
        Kind::PressurePlate | Kind::WeightedPlate => plate::get_direct_signal(level, state, dir),
        Kind::SculkSensor | Kind::CalibratedSculkSensor => sculk::get_direct_signal(level, state, dir),
        Kind::RedstoneTorch | Kind::RedstoneWallTorch => torch::get_direct_signal(level, pos, state, dir),
        _ => 0,
    }
}

// --- lifecycle ---------------------------------------------------------------

pub fn on_place(level: &mut Level, pos: Pos, state: StateId, old: StateId, moved_by_piston: bool) {
    let _ = moved_by_piston;
    match level.kind(state) {
        Kind::RedstoneWire => wire::on_place(level, pos, state, old),
        Kind::Repeater | Kind::Comparator => repeater::on_place(level, pos, state),
        Kind::Piston | Kind::StickyPiston => piston::on_place(level, pos, state, old),
        Kind::RedstoneTorch | Kind::RedstoneWallTorch => torch::on_place(level, pos, state),
        Kind::Observer => observer::on_place(level, pos, state, old),
        Kind::PoweredRail => rail::on_place(level, pos, state, old),
        Kind::Rail => rail::plain_on_place(level, pos, state, old),
        Kind::Water => water::on_place(level, pos, state),
        Kind::Hopper => container::hopper_on_place(level, pos, state, old),
        Kind::CopperBulb => bulb::bulb_on_place(level, pos, state, old),
        Kind::Tripwire => tripwire::on_place(level, pos, state, old),
        Kind::Shelf => shelf::on_place(level, pos, state, old),
        Kind::FallingBlock => falling::on_place(level, pos, state),
        Kind::Tnt => falling::tnt_on_place(level, pos, state, old),
        _ => {}
    }
}

pub fn affect_neighbors_after_removal(level: &mut Level, pos: Pos, state: StateId, moved_by_piston: bool) {
    match level.kind(state) {
        Kind::Lever => lever::affect_neighbors_after_removal(level, pos, state, moved_by_piston),
        Kind::Button => button::affect_neighbors_after_removal(level, pos, state, moved_by_piston),
        Kind::RedstoneWire => wire::affect_neighbors_after_removal(level, pos, state, moved_by_piston),
        Kind::Repeater | Kind::Comparator => {
            repeater::affect_neighbors_after_removal(level, pos, state, moved_by_piston)
        }
        Kind::PistonHead => piston::head_affect_neighbors_after_removal(level, pos, state),
        Kind::Observer => observer::affect_neighbors_after_removal(level, pos, state),
        Kind::PoweredRail => rail::affect_neighbors_after_removal(level, pos, state, moved_by_piston),
        Kind::Rail => rail::plain_affect_neighbors_after_removal(level, pos, state, moved_by_piston),
        Kind::RedstoneTorch | Kind::RedstoneWallTorch => {
            torch::affect_neighbors_after_removal(level, pos, state, moved_by_piston)
        }
        Kind::Lectern => lectern::affect_neighbors_after_removal(level, pos, state),
        Kind::PressurePlate | Kind::WeightedPlate => {
            plate::affect_neighbors_after_removal(level, pos, state, moved_by_piston)
        }
        Kind::Tripwire => tripwire::affect_neighbors_after_removal(level, pos, state, moved_by_piston),
        Kind::SculkSensor | Kind::CalibratedSculkSensor => sculk::affect_neighbors_after_removal(level, pos, state),
        Kind::TripwireHook => tripwire::hook_affect_neighbors_after_removal(level, pos, state, moved_by_piston),
        // `Containers.updateNeighboursAfterDestroy`.
        Kind::Chest
        | Kind::Barrel
        | Kind::Hopper
        | Kind::Dropper
        | Kind::Dispenser
        | Kind::ShulkerBox
        | Kind::Jukebox
        | Kind::Crafter => level.update_neighbour_for_output_signal(pos, level.block(state)),
        _ => {}
    }
}

pub fn neighbor_changed(
    level: &mut Level,
    pos: Pos,
    state: StateId,
    source: BlockId,
    moved_by_piston: bool,
    orientation: Option<Orientation>,
) {
    match level.kind(state) {
        Kind::RedstoneLamp => lamp::neighbor_changed(level, pos, state),
        Kind::RedstoneWire => wire::neighbor_changed(level, pos, state, source, orientation),
        Kind::Repeater => repeater::neighbor_changed(level, pos, state),
        Kind::Comparator => comparator::neighbor_changed(level, pos, state),
        Kind::NoteBlock => note::neighbor_changed(level, pos, state),
        Kind::PoweredRail => rail::neighbor_changed(level, pos, state, moved_by_piston),
        Kind::Rail => rail::plain_neighbor_changed(level, pos, state, source, moved_by_piston),
        Kind::Trapdoor => door::trapdoor_neighbor_changed(level, pos, state),
        Kind::Door => door::door_neighbor_changed(level, pos, state, source),
        Kind::Piston | Kind::StickyPiston => piston::neighbor_changed(level, pos, state),
        Kind::PistonHead => piston::head_neighbor_changed(level, pos, state, source),
        Kind::RedstoneTorch | Kind::RedstoneWallTorch => torch::neighbor_changed(level, pos, state),
        Kind::Water => water::neighbor_changed(level, pos, state),
        Kind::Hopper => container::hopper_neighbor_changed(level, pos, state),
        Kind::Dropper | Kind::Dispenser => container::dropper_neighbor_changed(level, pos, state),
        Kind::CopperBulb => bulb::bulb_neighbor_changed(level, pos, state),
        Kind::Bell => bulb::bell_neighbor_changed(level, pos, state),
        Kind::Crafter => crafter::neighbor_changed(level, pos, state),
        Kind::Shelf => shelf::neighbor_changed(level, pos, state),
        Kind::Tnt => falling::tnt_neighbor_changed(level, pos),
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
        Kind::Lever | Kind::Button => face_attached_update_shape(level, pos, state, dir),
        Kind::RedstoneTorch | Kind::RedstoneWallTorch | Kind::Torch | Kind::WallTorch => {
            torch::update_shape(level, pos, state, dir)
        }
        Kind::FallingBlock => falling::update_shape(level, pos, state),
        Kind::PressurePlate | Kind::WeightedPlate => plate::update_shape(level, pos, state, dir),
        Kind::Observer => observer::update_shape(level, pos, state, dir),
        Kind::NoteBlock => note::update_shape(level, pos, state, dir),
        Kind::Door => door::door_update_shape(level, pos, state, dir, neighbor, neighbor_state),
        Kind::Comparator | Kind::Cake => survives_on_below(level, state, dir, neighbor, neighbor_state),
        Kind::Water => water::update_shape(level, pos, state, neighbor_state),
        Kind::Bell => bulb::bell_update_shape(level, pos, state, dir),
        Kind::Leaves => plain::leaves_update_shape(level, pos, state, neighbor_state),
        Kind::TripwireHook => plain::hook_update_shape(level, pos, state, dir),
        Kind::Tripwire => tripwire::update_shape(level, state, dir, neighbor_state),
        Kind::Chest => chest_update_shape(level, state, dir, neighbor_state),
        Kind::SoulSand | Kind::Magma => bubble::source_update_shape(level, pos, state, dir, neighbor_state),
        Kind::BubbleColumn => bubble::column_update_shape(level, pos, state, dir, neighbor_state),
        _ => state,
    }
}

/// `ChestBlock.getConnectedDirection`: where the other half of a double
/// chest is.
fn chest_connected_direction(level: &Level, state: StateId) -> Dir {
    let facing = level.dir(state, "facing");
    if level.prop(state, "type") == "left" { facing.clockwise() } else { facing.counter_clockwise() }
}

/// `ChestBlock.updateShape`: a single chest joins a half-chest beside it
/// that reaches for it; a half whose partner goes becomes single.
fn chest_update_shape(level: &Level, state: StateId, dir: Dir, neighbor_state: StateId) -> StateId {
    if level.block(neighbor_state) == level.block(state) && dir.is_horizontal() {
        let nt = level.prop(neighbor_state, "type");
        if level.prop(state, "type") == "single"
            && nt != "single"
            && level.dir(state, "facing") == level.dir(neighbor_state, "facing")
            && chest_connected_direction(level, neighbor_state) == dir.opposite()
        {
            return level.with(state, "type", if nt == "left" { "right" } else { "left" });
        }
    } else if level.prop(state, "type") != "single" && chest_connected_direction(level, state) == dir {
        return level.with(state, "type", "single");
    }
    state
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
        Kind::Button => button::tick(level, pos, state),
        Kind::Observer => observer::tick(level, pos, state),
        Kind::Comparator => comparator::tick(level, pos, state),
        Kind::Dropper => container::dropper_tick(level, pos, state),
        Kind::Dispenser => dispenser::tick(level, pos, state),
        Kind::Crafter => crafter::tick(level, pos, state),
        Kind::Lectern => lectern::tick(level, pos, state),
        Kind::TripwireHook => tripwire::hook_tick(level, pos, state),
        Kind::SculkSensor | Kind::CalibratedSculkSensor => sculk::tick(level, pos, state),
        Kind::Leaves => plain::leaves_tick(level, pos, state),
        Kind::FallingBlock => falling::tick(level, pos, state),
        Kind::PressurePlate | Kind::WeightedPlate => plate::tick(level, pos, state),
        Kind::Tripwire => tripwire::tick(level, pos, state),
        Kind::SoulSand | Kind::Magma => bubble::source_tick(level, pos, state),
        Kind::BubbleColumn => bubble::column_tick(level, pos, state),
        _ => {}
    }
}

pub fn trigger_event(level: &mut Level, pos: Pos, state: StateId, a: i32, b: i32) -> bool {
    match level.kind(state) {
        Kind::Piston | Kind::StickyPiston => piston::trigger_event(level, pos, state, a, b),
        // The sound and particles have no effect on the world.
        Kind::NoteBlock | Kind::Bell => true,
        _ => false,
    }
}

/// `entityInside`: an entity's box is inside the block's space.
pub fn entity_inside(level: &mut Level, pos: Pos, state: StateId) {
    match level.kind(state) {
        Kind::PressurePlate | Kind::WeightedPlate => plate::entity_inside(level, pos, state),
        Kind::Tripwire => tripwire::entity_inside(level, pos, state),
        Kind::Hopper => container::hopper_entity_inside(level, pos),
        Kind::BubbleColumn => bubble::column_entity_inside(level, pos, state),
        Kind::Cobweb => bubble::web_entity_inside(level),
        _ => {}
    }
}

/// Right-click with an empty hand (`useWithoutItem`).
pub fn use_without_item(level: &mut Level, pos: Pos, state: StateId) {
    match level.kind(state) {
        Kind::Lever => lever::pull(level, pos, state),
        Kind::Button => button::press(level, pos, state),
        Kind::Repeater => repeater::use_without_item(level, pos, state),
        Kind::Comparator => comparator::use_without_item(level, pos, state),
        Kind::NoteBlock => note::use_without_item(level, pos, state),
        Kind::Trapdoor => door::trapdoor_use(level, pos, state),
        Kind::Door => door::door_use(level, pos, state),
        Kind::DaylightDetector => daylight::use_without_item(level, pos, state),
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

/// `updateShape` for blocks that need a sturdy top below them: diodes and
/// cakes break when it goes.
fn survives_on_below(level: &Level, state: StateId, dir: Dir, below: Pos, below_state: StateId) -> StateId {
    if dir == Dir::Down && !is_face_sturdy(level, below, below_state, Dir::Up) { level.air } else { state }
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
