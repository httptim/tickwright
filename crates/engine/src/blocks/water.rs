//! Water: `LiquidBlock` for the block and `FlowingFluid`/`WaterFluid` for
//! how it spreads.
//!
//! The fluid lives in the block state. `water[level=0]` is a source, levels
//! 1 to 7 are flowing water with amount `8 - level`, and levels 8 and up are
//! falling water with amount 8. Waterlogged blocks hold a source too.
//! Fluids tick in their own queue (`ServerLevel.fluidTicks`), after block
//! ticks; source and flowing water are separate fluid types there.

use crate::level::{Level, flags};
use crate::pos::{Dir, Pos};
use crate::registry::StateId;

use super::Kind;

/// `WaterFluid.getTickDelay`.
pub const TICK_DELAY: i32 = 5;
/// `WaterFluid.getSlopeFindDistance`.
const SLOPE_FIND_DISTANCE: i32 = 4;
/// `WaterFluid.getDropOff`.
const DROP_OFF: i32 = 1;

/// A fluid type, as `LevelTicks<Fluid>` keys it.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Fluid {
    Empty,
    Water,
    FlowingWater,
}

impl Fluid {
    fn is_water(self) -> bool {
        self != Fluid::Empty
    }
}

/// `FluidState`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct FluidState {
    pub fluid: Fluid,
    amount: i32,
    falling: bool,
}

const EMPTY: FluidState = FluidState { fluid: Fluid::Empty, amount: 0, falling: false };
const SOURCE: FluidState = FluidState { fluid: Fluid::Water, amount: 8, falling: false };

fn flowing(amount: i32, falling: bool) -> FluidState {
    FluidState { fluid: Fluid::FlowingWater, amount, falling }
}

impl FluidState {
    pub fn is_empty(self) -> bool {
        self.fluid == Fluid::Empty
    }

    pub fn is_source(self) -> bool {
        self.fluid == Fluid::Water
    }
}

/// Blocks that implement `SimpleWaterloggedBlock` (a `LiquidBlockContainer`).
fn is_container(level: &Level, state: StateId) -> bool {
    match level.kind(state) {
        Kind::Trapdoor | Kind::Chest | Kind::PoweredRail => true,
        Kind::Slab => level.prop(state, "type") != "double",
        _ => false,
    }
}

/// `BlockState.getFluidState`.
pub fn fluid_state(level: &Level, state: StateId) -> FluidState {
    match level.kind(state) {
        Kind::Water => match level.int(state, "level") {
            0 => SOURCE,
            l if l < 8 => flowing(8 - l, false),
            _ => flowing(8, true),
        },
        _ if is_container(level, state) && level.flag(state, "waterlogged") => SOURCE,
        _ => EMPTY,
    }
}

/// `FluidState.createLegacyBlock`.
fn legacy_block(level: &Level, fs: FluidState) -> StateId {
    if fs.is_empty() {
        return level.air;
    }
    let l = if fs.is_source() { 0 } else { 8 - fs.amount.min(8) + if fs.falling { 8 } else { 0 } };
    level.with_int(level.default_state(level.block_id("water")), "level", l)
}

// --- shapes ------------------------------------------------------------------

/// `BlockBehaviour.isSolid` (the legacy "solid" flag, worked out from the
/// collision shape's size), which also decides `blocksMotion`.
pub fn is_solid(level: &Level, state: StateId) -> bool {
    !matches!(
        level.kind(state),
        Kind::Air
            | Kind::Water
            | Kind::Lever
            | Kind::Button
            | Kind::RedstoneWire
            | Kind::RedstoneTorch
            | Kind::RedstoneWallTorch
            | Kind::Repeater
            | Kind::Comparator
            | Kind::PoweredRail
    )
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Shape {
    Empty,
    Full,
    Partial,
}

/// Whether the collision shape is `Shapes.empty()`, `Shapes.block()`, or
/// something else.
fn collision(level: &Level, state: StateId) -> Shape {
    match level.kind(state) {
        Kind::Air
        | Kind::Water
        | Kind::Lever
        | Kind::Button
        | Kind::RedstoneWire
        | Kind::RedstoneTorch
        | Kind::RedstoneWallTorch
        | Kind::PoweredRail => Shape::Empty,
        Kind::Solid
        | Kind::Glass
        | Kind::RedstoneBlock
        | Kind::RedstoneLamp
        | Kind::Observer
        | Kind::NoteBlock
        | Kind::Dropper
        | Kind::Slime => Shape::Full,
        Kind::Slab if level.prop(state, "type") == "double" => Shape::Full,
        Kind::Piston | Kind::StickyPiston if !level.flag(state, "extended") => Shape::Full,
        _ => Shape::Partial,
    }
}

/// Whether the collision shape covers the whole `dir` face of the block.
fn face_full(level: &Level, pos: Pos, state: StateId, dir: Dir) -> bool {
    match level.kind(state) {
        Kind::Door => {
            let facing = level.dir(state, "facing");
            let side = if !level.flag(state, "open") {
                facing
            } else if level.prop(state, "hinge") == "right" {
                facing.counter_clockwise()
            } else {
                facing.clockwise()
            };
            dir == side.opposite()
        }
        Kind::Repeater | Kind::Comparator | Kind::Composter => dir == Dir::Down,
        Kind::Chest | Kind::Cake | Kind::Honey | Kind::Hopper => false,
        _ => super::is_face_sturdy(level, pos, state, dir),
    }
}

/// `FlowingFluid.canPassThroughWall`. Two partial shapes are taken to seal
/// the boundary only when one of them covers the whole face.
fn can_pass_through_wall(level: &Level, dir: Dir, from: Pos, from_state: StateId, to: Pos, to_state: StateId) -> bool {
    let (a, b) = (collision(level, from_state), collision(level, to_state));
    if a == Shape::Full || b == Shape::Full {
        return false;
    }
    if a == Shape::Empty && b == Shape::Empty {
        return true;
    }
    !(face_full(level, from, from_state, dir) || face_full(level, to, to_state, dir.opposite()))
}

// --- what can hold water -------------------------------------------------------

/// `FlowingFluid.canHoldAnyFluid`.
fn can_hold_any_fluid(level: &Level, state: StateId) -> bool {
    is_container(level, state) || (!is_solid(level, state) && level.kind(state) != Kind::Door)
}

/// `FlowingFluid.canHoldSpecificFluid`: a waterloggable block takes only a
/// source, and only if it is dry.
fn can_hold_specific_fluid(level: &Level, state: StateId, fluid: Fluid) -> bool {
    !is_container(level, state) || (fluid == Fluid::Water && !level.flag(state, "waterlogged"))
}

fn can_hold_fluid(level: &Level, state: StateId, fluid: Fluid) -> bool {
    can_hold_any_fluid(level, state) && can_hold_specific_fluid(level, state, fluid)
}

/// `WaterFluid.canBeReplacedWith`; empty fluid can always be replaced.
fn can_be_replaced_with(fs: FluidState, fluid: Fluid, dir: Dir) -> bool {
    fs.is_empty() || (dir == Dir::Down && !fluid.is_water())
}

/// `FlowingFluid.canMaybePassThrough`.
fn can_maybe_pass_through(
    level: &Level,
    from: Pos,
    from_state: StateId,
    dir: Dir,
    to: Pos,
    to_state: StateId,
    to_fluid: FluidState,
) -> bool {
    can_pass_through_wall(level, dir, from, from_state, to, to_state)
        && !to_fluid.is_source()
        && can_hold_any_fluid(level, to_state)
}

/// `FlowingFluid.isWaterHole`.
fn is_water_hole(level: &Level, top: Pos, top_state: StateId, bottom: Pos, bottom_state: StateId) -> bool {
    if !can_pass_through_wall(level, Dir::Down, top, top_state, bottom, bottom_state) {
        return false;
    }
    fluid_state(level, bottom_state).fluid.is_water() || can_hold_fluid(level, bottom_state, Fluid::FlowingWater)
}

fn is_hole(level: &Level, pos: Pos) -> bool {
    is_water_hole(level, pos, level.get(pos), pos.below(), level.get(pos.below()))
}

// --- FlowingFluid --------------------------------------------------------------

/// `FlowingFluid.getNewLiquid`: what the water at `pos` should be, from its
/// neighbors.
fn new_liquid(level: &Level, pos: Pos, state: StateId) -> FluidState {
    let mut highest = 0;
    let mut sources = 0;
    for dir in Dir::HORIZONTAL {
        let n = pos.offset(dir);
        let ns = level.get(n);
        let nf = fluid_state(level, ns);
        if nf.fluid.is_water() && can_pass_through_wall(level, dir, pos, state, n, ns) {
            if nf.is_source() {
                sources += 1;
            }
            highest = highest.max(nf.amount);
        }
    }
    if sources >= 2 {
        let below = level.get(pos.below());
        if is_solid(level, below) || fluid_state(level, below).is_source() {
            return SOURCE;
        }
    }
    let above = pos.above();
    let above_state = level.get(above);
    if fluid_state(level, above_state).fluid.is_water()
        && can_pass_through_wall(level, Dir::Up, pos, state, above, above_state)
    {
        return flowing(8, true);
    }
    let amount = highest - DROP_OFF;
    if amount <= 0 { EMPTY } else { flowing(amount, false) }
}

/// `FlowingFluid.tick`, run from the fluid tick queue.
pub fn tick(level: &mut Level, pos: Pos, fluid: Fluid) {
    let mut state = level.get(pos);
    let mut fs = fluid_state(level, state);
    if fs.fluid != fluid {
        return;
    }
    if !fs.is_source() {
        let new = new_liquid(level, pos, state);
        if new.is_empty() {
            fs = new;
            state = level.air;
            level.set_block(pos, state, flags::ALL);
        } else if new != fs {
            fs = new;
            state = legacy_block(level, new);
            level.set_block(pos, state, flags::ALL);
            level.schedule_fluid_tick(pos, new.fluid, TICK_DELAY);
        }
    }
    spread(level, pos, state, fs);
}

/// `FlowingFluid.spread`: down if it can, otherwise (or also, with three
/// sources around) to the sides.
fn spread(level: &mut Level, pos: Pos, state: StateId, fs: FluidState) {
    if fs.is_empty() {
        return;
    }
    let below = pos.below();
    let below_state = level.get(below);
    let below_fluid = fluid_state(level, below_state);
    if can_maybe_pass_through(level, pos, state, Dir::Down, below, below_state, below_fluid) {
        let new = new_liquid(level, below, below_state);
        if can_be_replaced_with(below_fluid, new.fluid, Dir::Down)
            && can_hold_specific_fluid(level, below_state, new.fluid)
        {
            spread_to(level, below, below_state, new);
            if source_neighbor_count(level, pos) >= 3 {
                spread_to_sides(level, pos, fs, state);
            }
            return;
        }
    }
    if fs.is_source() || !is_water_hole(level, pos, state, below, below_state) {
        spread_to_sides(level, pos, fs, state);
    }
}

fn source_neighbor_count(level: &Level, pos: Pos) -> usize {
    Dir::HORIZONTAL.iter().filter(|&&d| fluid_state(level, level.get(pos.offset(d))).is_source()).count()
}

/// `FlowingFluid.spreadToSides`.
fn spread_to_sides(level: &mut Level, pos: Pos, fs: FluidState, state: StateId) {
    let amount = if fs.falling { 7 } else { fs.amount - DROP_OFF };
    if amount <= 0 {
        return;
    }
    for (dir, new) in get_spread(level, pos, state) {
        let n = pos.offset(dir);
        spread_to(level, n, level.get(n), new);
    }
}

/// `FlowingFluid.getSpread`: the sides with the shortest way down, in
/// `EnumMap` (direction ordinal) order.
fn get_spread(level: &Level, pos: Pos, state: StateId) -> Vec<(Dir, FluidState)> {
    let mut lowest = 1000;
    let mut result: Vec<(Dir, FluidState)> = Vec::new();
    for dir in Dir::HORIZONTAL {
        let n = pos.offset(dir);
        let ns = level.get(n);
        let nf = fluid_state(level, ns);
        if !can_maybe_pass_through(level, pos, state, dir, n, ns, nf) {
            continue;
        }
        let new = new_liquid(level, n, ns);
        if !can_hold_specific_fluid(level, ns, new.fluid) {
            continue;
        }
        let distance = if is_hole(level, n) { 0 } else { slope_distance(level, n, 1, dir.opposite(), ns) };
        if distance < lowest {
            result.clear();
        }
        if distance <= lowest {
            if can_be_replaced_with(nf, new.fluid, dir) {
                result.push((dir, new));
            }
            lowest = distance;
        }
    }
    result.sort_by_key(|&(d, _)| d as u8);
    result
}

/// `FlowingFluid.getSlopeDistance`.
fn slope_distance(level: &Level, pos: Pos, pass: i32, from: Dir, state: StateId) -> i32 {
    let mut lowest = 1000;
    for dir in Dir::HORIZONTAL {
        if dir == from {
            continue;
        }
        let n = pos.offset(dir);
        let ns = level.get(n);
        let nf = fluid_state(level, ns);
        if can_maybe_pass_through(level, pos, state, dir, n, ns, nf)
            && can_hold_specific_fluid(level, ns, Fluid::FlowingWater)
        {
            if is_hole(level, n) {
                return pass;
            }
            if pass < SLOPE_FIND_DISTANCE {
                lowest = lowest.min(slope_distance(level, n, pass + 1, dir.opposite(), ns));
            }
        }
    }
    lowest
}

/// `FlowingFluid.spreadTo`. Whatever the water replaces just drops as items.
fn spread_to(level: &mut Level, pos: Pos, state: StateId, fs: FluidState) {
    if is_container(level, state) {
        place_liquid(level, pos, state, fs);
    } else {
        level.set_block(pos, legacy_block(level, fs), flags::ALL);
    }
}

/// `SimpleWaterloggedBlock.placeLiquid`.
fn place_liquid(level: &mut Level, pos: Pos, state: StateId, fs: FluidState) {
    if !level.flag(state, "waterlogged") && fs.is_source() {
        level.set_block(pos, level.with_bool(state, "waterlogged", true), flags::ALL);
        level.schedule_fluid_tick(pos, Fluid::Water, TICK_DELAY);
    }
}

// --- LiquidBlock ---------------------------------------------------------------

pub fn on_place(level: &mut Level, pos: Pos, state: StateId) {
    level.schedule_fluid_tick(pos, fluid_state(level, state).fluid, TICK_DELAY);
}

pub fn neighbor_changed(level: &mut Level, pos: Pos, state: StateId) {
    level.schedule_fluid_tick(pos, fluid_state(level, state).fluid, TICK_DELAY);
}

pub fn update_shape(level: &mut Level, pos: Pos, state: StateId, neighbor_state: StateId) -> StateId {
    if fluid_state(level, state).is_source() || fluid_state(level, neighbor_state).is_source() {
        level.schedule_fluid_tick(pos, fluid_state(level, state).fluid, TICK_DELAY);
    }
    state
}
