//! Entities: items, falling blocks, primed TNT and minecarts, moved the way the game's
//! `Entity.move` moves them, against the blocks' collision boxes.
//!
//! The level keeps its entities in the order they were added, which is the
//! order the game ticks them in. Entities added while entities are ticking
//! wait for the next tick, as in `EntityTickList`.

mod arrow;
pub mod explosion;
mod falling;
mod fireball;
mod fluid;
pub mod item;
mod minecart;
pub mod snbt;
mod tnt;

use anyhow::{Context, Result, bail};

use crate::blocks::Kind;
use crate::level::Level;
use crate::physics::{Aabb, Physics, Vec3};
use crate::pos::Pos;
use crate::random::JavaRandom;
use crate::registry::StateId;

pub use falling::fall;
pub use item::Stack;
pub use tnt::prime;

/// `0.98F`, which most drag uses, widened to double.
pub const DRAG: f64 = 0.98f32 as f64;

#[derive(Clone, Debug, Default)]
pub enum Data {
    Item(item::Item),
    FallingBlock(falling::Falling),
    Tnt(tnt::Tnt),
    Minecart(minecart::Minecart),
    SmallFireball,
    Arrow(arrow::Arrow),
    /// The empty slot an entity leaves while it is out of the list
    /// (`Level::take_entity`).
    #[default]
    Gone,
}

#[derive(Clone, Debug, Default)]
pub struct Entity {
    pub id: i32,
    pub data: Data,
    pub pos: Vec3,
    pub motion: Vec3,
    pub on_ground: bool,
    pub horizontal_collision: bool,
    pub vertical_collision: bool,
    pub removed: bool,
    pub tick_count: i32,
    /// The entity's own random source (`Entity.random`).
    pub random: JavaRandom,
    main_support: Option<Pos>,
    on_ground_no_blocks: bool,
    /// `stuckSpeedMultiplier`, set by cobwebs for the next move.
    pub stuck_speed_multiplier: Vec3,
    /// `wasTouchingWater` and the water's height over the entity's feet.
    pub was_touching_water: bool,
    pub fluid_height: f64,
    pub first_tick: bool,
    /// `pistonDeltas` and the game time they were counted in.
    piston_deltas: [f64; 3],
    piston_deltas_time: i64,
    /// The vertical motion the entity started its tick with.
    start_motion_y: f64,
    /// `noPhysics`: an item that began its tick inside a block moves
    /// through blocks, piston pushes included, until its next tick.
    pub no_physics: bool,
    /// Where the entity began this tick (`oldPosition`).
    pub old_pos: Vec3,
    /// `movementThisTick`, and the moves its blocks last felt
    /// (`finalMovementsThisTick`).
    movements: Vec<Movement>,
    final_movements: Vec<Movement>,
    /// While the entity is out of the level's list, the index of the slot
    /// it left (`Level::take_entity`).
    slot: Option<usize>,
}

impl Entity {
    pub fn new(level: &mut Level, data: Data, pos: Vec3) -> Entity {
        level.entity_counter += 1;
        let mut random = JavaRandom::new(level.next_unique_seed());
        // `Mth.createInsecureUUID`.
        random.next_long();
        random.next_long();
        if let Data::Item(_) = data {
            // `bobOffs` and the starting yaw.
            random.next_float();
            random.next_float();
        }
        Entity { id: level.entity_counter, data, pos, random, first_tick: true, ..Default::default() }
    }

    pub fn type_name(&self) -> &'static str {
        match self.data {
            Data::Item(_) => "minecraft:item",
            Data::FallingBlock(_) => "minecraft:falling_block",
            Data::Tnt(_) => "minecraft:tnt",
            Data::Minecart(ref m) => m.type_name(),
            Data::SmallFireball => "minecraft:small_fireball",
            Data::Arrow(_) => "minecraft:arrow",
            Data::Gone => "minecraft:none",
        }
    }

    /// `EntityDimensions`: width and height, as the game's floats.
    fn dimensions(&self) -> (f32, f32) {
        match self.data {
            Data::Item(_) => (0.25, 0.25),
            Data::Minecart(_) => (0.98, 0.7),
            Data::SmallFireball => (0.3125, 0.3125),
            Data::Arrow(_) => (0.5, 0.5),
            Data::FallingBlock(_) | Data::Tnt(_) | Data::Gone => (0.98, 0.98),
        }
    }

    /// `getBbWidth`.
    pub fn width(&self) -> f32 {
        self.dimensions().0
    }

    /// `getBoundingBox`, from `EntityDimensions.makeBoundingBox`.
    pub fn bounding_box(&self) -> Aabb {
        self.box_at(self.pos)
    }

    /// `makeBoundingBox(position)`: the entity's box were it at `p`.
    pub fn box_at(&self, p: Vec3) -> Aabb {
        let (w, h) = self.dimensions();
        let f = (w / 2.0) as f64;
        let h = h as f64;
        Aabb::new(p.x - f, p.y, p.z - f, p.x + f, p.y + h, p.z + f)
    }

    /// `blockPosition`.
    pub fn block_position(&self) -> Pos {
        Pos::new(floor(self.pos.x), floor(self.pos.y), floor(self.pos.z))
    }

    /// Whether it is a `LivingEntity`. None of the engine's entities are.
    pub fn is_living(&self) -> bool {
        false
    }

    pub fn discard(&mut self) {
        self.removed = true;
    }

    /// What the harness records about the entity besides where it is.
    pub fn detail(&self, level: &Level) -> Option<String> {
        match &self.data {
            Data::Item(i) => Some(format!("minecraft:{}*{}", i.stack.id, i.stack.count)),
            Data::Tnt(t) => Some(format!("fuse={}", t.fuse)),
            Data::FallingBlock(f) => Some(level.reg.format(f.state)),
            Data::Minecart(m) => m.items.as_ref().map(|i| crate::blocks::container::describe(i)),
            Data::SmallFireball | Data::Arrow(_) | Data::Gone => None,
        }
    }

    /// `getOnPos(yOffset)`: the block the entity stands on, or the one
    /// `y_offset` below its feet.
    pub fn on_pos(&self, level: &Level, y_offset: f32) -> Pos {
        match self.main_support {
            Some(p) => {
                if y_offset <= 1.0E-5 {
                    return p;
                }
                let state = level.get(p);
                let name = level.reg.name(state);
                let fence = name.ends_with("_fence");
                let wall = name.ends_with("_wall");
                let gate = name.ends_with("_fence_gate");
                if (y_offset as f64 <= 0.5 && fence) || wall || gate {
                    p
                } else {
                    Pos::new(p.x, floor(self.pos.y - y_offset as f64), p.z)
                }
            }
            None => Pos::new(floor(self.pos.x), floor(self.pos.y - y_offset as f64), floor(self.pos.z)),
        }
    }

    /// `clearMovementThisTick`.
    pub fn clear_movements(&mut self) {
        self.movements.clear();
    }

    /// `isSupportedBy`: whether `pos` is the block the entity stands on.
    pub fn is_supported_by(&self, pos: Pos) -> bool {
        self.main_support == Some(pos)
    }

    /// `getBlockPosBelowThatAffectsMyMovement`. An item looks almost a
    /// whole block down (`ItemEntity` overrides it): resting on a honey
    /// block that is the block under the honey, and flying up to a block
    /// above the honey's top it is the honey.
    pub fn pos_below_affecting_movement(&self, level: &Level) -> Pos {
        let y_offset = if matches!(self.data, Data::Item(_)) { 0.999999 } else { 0.500001 };
        self.on_pos(level, y_offset)
    }
}

/// `Mth.floor`.
pub fn floor(v: f64) -> i32 {
    let i = v as i32;
    if v < i as f64 { i - 1 } else { i }
}

// --- the level's entity list -----------------------------------------------

impl Level {
    /// `ServerLevel.addFreshEntity`.
    pub fn add_entity(&mut self, e: Entity) {
        self.entities.push(e);
    }

    /// `ServerLevel.tick`'s entity pass: each entity that is not removed
    /// gets `tickCount++` and its `tick()`.
    pub(crate) fn tick_entities(&mut self) {
        let n = self.entities.len();
        for i in 0..n {
            if self.entities[i].removed {
                continue;
            }
            let mut e = self.take_entity(i);
            e.old_pos = e.pos;
            e.tick_count += 1;
            tick(self, &mut e);
            self.put_entity(e);
        }
        self.entities.retain(|e| !e.removed);
    }

    /// Takes the entity at `i` out of the list to tick or move it, until
    /// `put_entity` puts it back. Its slot holds an empty entity that counts
    /// as removed, so nothing that looks for entities finds it: the empty
    /// entity sits at the origin with a TNT-sized box, where a pressure
    /// plate or tripwire would otherwise feel it.
    pub(crate) fn take_entity(&mut self, i: usize) -> Entity {
        let mut e = std::mem::replace(&mut self.entities[i], Entity { removed: true, ..Default::default() });
        e.slot = Some(i);
        e
    }

    /// Puts an entity taken out with `take_entity` back in its slot.
    pub(crate) fn put_entity(&mut self, mut e: Entity) {
        let i = e.slot.take().expect("an entity out of the level's list");
        self.entities[i] = e;
    }

    /// Puts `e`, an entity out of the list (`take_entity`) to tick or to be
    /// moved by a piston, back in its own slot while `f` runs, so blocks
    /// and containers looking for entities find it where it belongs, then
    /// takes it out again. `f` gets its index in the list. Entities spawned
    /// meanwhile, such as TNT a pressure plate lights, go on the end of the
    /// list: the game adds them to `EntityTickList` at once, to tick from
    /// the next tick.
    pub(crate) fn with_listed<R>(&mut self, e: &mut Entity, f: impl FnOnce(&mut Level, usize) -> R) -> R {
        let k = e.slot.expect("an entity out of the level's list");
        let id = e.id;
        std::mem::swap(&mut self.entities[k], e);
        let r = f(self, k);
        debug_assert_eq!(self.entities[k].id, id, "the entity in slot {k} changed while blocks saw it");
        std::mem::swap(&mut self.entities[k], e);
        r
    }

    pub fn physics(&self) -> &'static Physics {
        Physics::get(self.reg.version).expect("block physics for this version")
    }

    /// Something the engine can't simulate happened; the run fails with it.
    pub fn unsupported(&mut self, what: impl Into<String>) {
        if self.unsupported.is_none() {
            self.unsupported = Some(what.into());
        }
    }
}

fn tick(level: &mut Level, e: &mut Entity) {
    e.start_motion_y = e.motion.y;
    match e.data {
        Data::Item(_) => item::tick(level, e),
        Data::FallingBlock(_) => falling::tick(level, e),
        Data::Tnt(_) => tnt::tick(level, e),
        Data::Minecart(_) => minecart::tick(level, e),
        Data::SmallFireball => fireball::tick(level, e),
        Data::Arrow(_) => arrow::tick(level, e),
        Data::Gone => {}
    }
}

/// `/summon <entity> x y z {nbt}`. Entity types and data the engine
/// doesn't simulate are refused.
pub fn summon(level: &mut Level, entity: &str, at: Vec3, motion: Option<Vec3>, nbt: Option<&str>) -> Result<()> {
    let name = entity.strip_prefix("minecraft:").unwrap_or(entity);
    let Some(known) = known_data(name) else {
        bail!("summoning {entity} is not supported by the engine yet");
    };
    let tag = parse_data(nbt)?;
    if let Some(k) = tag.keys().into_iter().find(|k| !known.contains(k)) {
        bail!("summon data {k} for {entity} is not supported by the engine yet");
    }
    spawn(level, name, &tag, at, motion)
}

/// An entity that is part of a build when it is placed. Types the engine
/// doesn't simulate are left out, and so is saved data it doesn't model
/// (a saved entity carries its UUID, rotation, air supply and the like),
/// except item stacks with more than an id and a count, which are refused
/// as in summon data. Returns whether the entity was created.
pub fn place_start_entity(
    level: &mut Level,
    entity: &str,
    at: Vec3,
    motion: Option<Vec3>,
    nbt: Option<&str>,
) -> Result<bool> {
    let name = entity.strip_prefix("minecraft:").unwrap_or(entity);
    if known_data(name).is_none() {
        return Ok(false);
    }
    spawn(level, name, &parse_data(nbt)?, at, motion)?;
    Ok(true)
}

/// Each entity type the engine simulates, without `minecraft:`, and the
/// summon data keys it reads.
const KNOWN_DATA: &[(&str, &[&str])] = &[
    ("item", &["Item", "PickupDelay", "Age"]),
    ("tnt", &["fuse"]),
    ("falling_block", &["BlockState", "Time", "DropItem", "CancelDrop"]),
    ("minecart", &[]),
    ("furnace_minecart", &[]),
    ("chest_minecart", &["Items"]),
    ("hopper_minecart", &["Items"]),
    ("small_fireball", &[]),
    ("arrow", &[]),
];

fn known_data(name: &str) -> Option<&'static [&'static str]> {
    KNOWN_DATA.iter().find(|(n, _)| *n == name).map(|(_, keys)| *keys)
}

/// The entity types the engine simulates, without `minecraft:`. A build's
/// entities of any other type are left out of the world and carried in its
/// file unchanged.
pub fn simulated() -> impl Iterator<Item = &'static str> {
    KNOWN_DATA.iter().map(|(name, _)| *name)
}

fn parse_data(nbt: Option<&str>) -> Result<snbt::Tag> {
    Ok(match nbt {
        Some(t) => snbt::parse(t).with_context(|| format!("summon data {t}"))?,
        None => snbt::Tag::Compound(Default::default()),
    })
}

/// Refuses an item stack (`ItemStack.CODEC`) that carries more than an id
/// and a count. The engine's stacks have no components, and components
/// change how an item stacks: `max_stack_size` sets its stack size, and
/// stacks merge only with stacks whose components are the same, so even a
/// custom name keeps two stacks apart.
fn check_item_stack(entity: &str, path: &str, stack: &snbt::Tag) -> Result<()> {
    for key in stack.keys() {
        match key {
            "id" | "count" => {}
            "components" => {
                if let Some(component) = stack.get(key).and_then(|c| c.keys().first().copied()) {
                    bail!(
                        "the item component {component} in {entity} data ({path}) is not supported by the engine yet"
                    );
                }
            }
            _ => bail!("{entity} data {path}.{key} is not supported by the engine yet"),
        }
    }
    Ok(())
}

fn spawn(level: &mut Level, name: &str, tag: &snbt::Tag, at: Vec3, motion: Option<Vec3>) -> Result<()> {
    let num = |key: &str| tag.get(key).and_then(|t| t.number());
    let data = match name {
        "item" => {
            let it = tag.get("Item").context("an item needs Item")?;
            check_item_stack(name, "Item", it)?;
            let id = it.get("id").and_then(|t| t.string()).context("Item.id")?;
            let count = it.get("count").and_then(|t| t.number()).unwrap_or(1.0) as i32;
            let mut item = item::Item::new(Stack::new(id, count));
            if let Some(d) = num("PickupDelay") {
                item.pickup_delay = d as i32;
            }
            if let Some(a) = num("Age") {
                item.age = a as i32;
            }
            Data::Item(item)
        }
        "tnt" => Data::Tnt(tnt::Tnt { fuse: num("fuse").unwrap_or(80.0) as i32 }),
        "falling_block" => {
            // `FallingBlockEntity.readAdditionalSaveData`: sand unless told.
            let state = match tag.get("BlockState") {
                Some(b) => {
                    let name = b.get("Name").and_then(|t| t.string()).context("BlockState.Name")?;
                    let props: Vec<String> = match b.get("Properties") {
                        Some(snbt::Tag::Compound(m)) => {
                            m.iter().map(|(k, v)| format!("{k}={}", v.string().unwrap_or_default())).collect()
                        }
                        _ => Vec::new(),
                    };
                    let text = if props.is_empty() { name.to_string() } else { format!("{name}[{}]", props.join(",")) };
                    level.reg.parse(&text)?
                }
                None => level.default_state(level.block_id("sand")),
            };
            if level.kind(state) == Kind::Other {
                bail!("a falling {} is not supported by the engine yet", level.reg.name(state));
            }
            Data::FallingBlock(falling::Falling {
                state,
                time: num("Time").unwrap_or(0.0) as i32,
                drop_item: num("DropItem").is_none_or(|v| v != 0.0),
                cancel_drop: num("CancelDrop").is_some_and(|v| v != 0.0),
            })
        }
        "small_fireball" => Data::SmallFireball,
        "arrow" => Data::Arrow(Default::default()),
        _ => {
            let mut cart = minecart::Minecart::new(name);
            if let (Some(items), Some(snbt::Tag::List(list))) = (cart.items.as_mut(), tag.get("Items")) {
                for (i, it) in list.iter().enumerate() {
                    let slot = it.get("Slot").and_then(|t| t.number()).context("Items[].Slot")? as usize;
                    let mut stack = it.clone();
                    if let snbt::Tag::Compound(m) = &mut stack {
                        m.remove("Slot");
                    }
                    check_item_stack(name, &format!("Items[{i}]"), &stack)?;
                    let id = it.get("id").and_then(|t| t.string()).context("Items[].id")?;
                    let count = it.get("count").and_then(|t| t.number()).unwrap_or(1.0) as i32;
                    let id = if id.contains(':') { id.to_string() } else { format!("minecraft:{id}") };
                    *items.get_mut(slot).context("Items[].Slot out of range")? =
                        crate::blocks::container::Stack::new(&id, count);
                }
            }
            Data::Minecart(cart)
        }
    };
    let mut e = Entity::new(level, data, at);
    if let Some(m) = motion {
        e.motion = clamp_loaded_motion(m);
    }
    level.add_entity(e);
    Ok(())
}

/// `Entity.load` keeps each loaded motion component within ±10.
fn clamp_loaded_motion(m: Vec3) -> Vec3 {
    let c = |v: f64| if v.abs() > 10.0 { 0.0 } else { v };
    Vec3::new(c(m.x), c(m.y), c(m.z))
}

// --- movement --------------------------------------------------------------

/// Buffers that moves and block walks reuse, so that once they have grown
/// moving an entity allocates nothing. Each use takes a buffer out of the
/// level, clears it, and puts it back, and they hold no state between
/// uses: a copy of the level starts with empty ones. A use nested inside
/// another (a block an entity feels moving another entity) finds an empty
/// buffer and allocates its own, which is safe, only slower.
#[derive(Default)]
pub(crate) struct Scratch {
    /// `Entity.visitedBlocks`: the blocks that felt the entity in this
    /// `applyEffectsFromBlocks`.
    visited: Vec<Pos>,
    /// The blocks one walk (`forEachBlockIntersectedBetween`) has visited.
    seen: Vec<Pos>,
    /// The boxes a move might hit.
    shapes: Shapes,
}

impl Clone for Scratch {
    fn clone(&self) -> Scratch {
        Scratch::default()
    }
}

/// `PistonMovingBlockEntity.moveEntityByPiston` for the entity at `index`
/// in the level's list: `Entity.move(MoverType.PISTON, ...)` with blocks
/// moving toward `noclip` passing through it, then the blocks it is in
/// feel it (`applyEffectsFromBlocks`), every time, even when the push came
/// to nothing. A minecart's own `move` has them feel it first as well, so
/// a cart feels them twice on every push, a push the limit cut to nothing
/// included.
pub fn move_by_piston(level: &mut Level, index: usize, noclip: crate::pos::Dir, movement: Vec3) {
    let mut e = level.take_entity(index);
    let from = e.pos;
    level.piston_noclip = Some(noclip);
    if e.no_physics {
        // `Entity.move` without physics: no piston limit, no collisions.
        e.pos = e.pos + movement;
        e.horizontal_collision = false;
        e.vertical_collision = false;
    } else {
        let m = limit_piston_movement(level, &mut e, movement);
        if m != Vec3::ZERO {
            if e.stuck_speed_multiplier.length_sqr() > 1.0E-7 {
                level.unsupported("a piston pushing an entity stuck in a block");
            }
            move_entity(level, &mut e, m);
        } else if let Data::Minecart(_) = e.data {
            // `Entity.move` returns at once, but `AbstractMinecart.move`
            // still goes on to feel the blocks.
            minecart_after_move(level, &mut e);
        }
    }
    let to = e.pos;
    apply_effects_between(level, &mut e, from, to);
    remove_latest_movement(&mut e);
    level.piston_noclip = None;
    level.put_entity(e);
}

/// `Entity.limitPistonMovement`: pistons move an entity at most 0.51 along
/// each axis per game tick, and a push of 1.0E-5 or less is none at all.
fn limit_piston_movement(level: &Level, e: &mut Entity, movement: Vec3) -> Vec3 {
    if movement.length_sqr() <= 1.0E-7 {
        return movement;
    }
    if e.piston_deltas_time != level.game_time {
        e.piston_deltas = [0.0; 3];
        e.piston_deltas_time = level.game_time;
    }
    let axis = if movement.x != 0.0 {
        0
    } else if movement.y != 0.0 {
        1
    } else {
        2
    };
    let d = (movement.get(axis as u8) + e.piston_deltas[axis]).clamp(-0.51, 0.51);
    let amount = d - e.piston_deltas[axis];
    e.piston_deltas[axis] = d;
    if amount.abs() <= 1.0E-5f32 as f64 { Vec3::ZERO } else { Vec3::ZERO.with(axis as u8, amount) }
}

/// `Mth.equal(double, double)`.
fn mth_equal(a: f64, b: f64) -> bool {
    (b - a).abs() < 1.0E-5f32 as f64
}

/// One step an entity moved this tick (`Entity.Movement`): where from and
/// to, and for a move `Entity.move` made, the movement it was asked for,
/// whose axes set the order the blocks it passed are felt in.
#[derive(Clone, Copy, Debug)]
pub struct Movement {
    from: Vec3,
    to: Vec3,
    asked: Option<Vec3>,
}

/// `Entity.addMovementThisTick`: at most 100 are kept, the first two
/// merging into one.
fn add_movement(e: &mut Entity, m: Movement) {
    if e.movements.len() >= 100 {
        let first = e.movements.remove(0);
        let second = e.movements.remove(0);
        e.movements.insert(0, Movement { from: first.from, to: second.to, asked: None });
    }
    e.movements.push(m);
}

/// `Entity.removeLatestMovementRecording`.
fn remove_latest_movement(e: &mut Entity) {
    e.movements.pop();
}

/// `Entity.applyEffectsFromBlocks()`: the blocks the entity's moves this
/// tick took it through feel it, or those between where it began the tick
/// and where it is if it made none. The moves are kept for
/// [`apply_effects_for_last_movements`].
pub fn apply_effects_from_blocks(level: &mut Level, e: &mut Entity) {
    // The moves become the last moves, and the list of moves this tick
    // takes over the last moves' buffer, so nothing is copied.
    std::mem::swap(&mut e.movements, &mut e.final_movements);
    e.movements.clear();
    let pos = e.pos;
    match e.final_movements.last() {
        None => e.final_movements.push(Movement { from: e.old_pos, to: pos, asked: None }),
        Some(last) if last.to.distance_to_sqr(pos) > 9.9999994E-11f32 as f64 => {
            let from = last.to;
            e.final_movements.push(Movement { from, to: pos, asked: None })
        }
        _ => {}
    }
    apply_effects_for_last_movements(level, e);
}

/// `Entity.applyEffectsFromBlocksForLastMovements`: an entity that didn't
/// move this tick feels the blocks of its last moves again.
pub fn apply_effects_for_last_movements(level: &mut Level, e: &mut Entity) {
    // Out of the entity while it is borrowed, and back after: no block
    // reads an entity's moves.
    let moves = std::mem::take(&mut e.final_movements);
    apply_movements(level, e, &moves);
    e.final_movements = moves;
}

/// `Entity.applyEffectsFromBlocks(from, to)`.
pub fn apply_effects_between(level: &mut Level, e: &mut Entity, from: Vec3, to: Vec3) {
    apply_movements(level, e, &[Movement { from, to, asked: None }]);
}

/// `Entity.applyEffectsFromBlocks(movements)`: the block under an entity on
/// the ground feels it step on it, then each block it passed through feels
/// it once (`checkInsideBlocks`). An entity without physics feels nothing.
fn apply_movements(level: &mut Level, e: &mut Entity, moves: &[Movement]) {
    if e.removed || e.no_physics {
        return;
    }
    if e.on_ground {
        step_on(level, e);
    }
    let mut visited = std::mem::take(&mut level.entity_scratch.visited);
    visited.clear();
    for m in moves {
        let delta = m.to - m.from;
        let mut left = 16;
        match m.asked {
            // A move's axes are felt one at a time, in its axis order.
            Some(asked) if delta.length_sqr() > 0.0 => {
                let mut pos = m.from;
                for axis in axis_step_order(asked) {
                    let d = delta.get(axis);
                    if d != 0.0 {
                        let to = pos.with(axis, pos.get(axis) + d);
                        left -= check_inside_blocks(level, e, pos, to, &mut visited, left);
                        pos = to;
                    }
                }
            }
            _ => left -= check_inside_blocks(level, e, m.from, m.to, &mut visited, 16),
        }
        if left <= 0 {
            check_inside_blocks(level, e, m.to, m.to, &mut visited, 1);
        }
    }
    level.entity_scratch.visited = visited;
}

/// `Entity.checkInsideBlocks(from, to, ...)`: each block the box passes
/// moving from `from` to `to` feels the entity, once per
/// `applyEffectsFromBlocks`, up to `max_iterations` steps of the walk.
/// The entity is out of the level's list, so it goes back in its slot
/// while blocks see it (`Level::with_listed`), and comes back changed.
/// Returns how many steps the walk took.
fn check_inside_blocks(
    level: &mut Level,
    e: &mut Entity,
    from: Vec3,
    to: Vec3,
    visited: &mut Vec<Pos>,
    max_iterations: i32,
) -> i32 {
    let target = e.box_at(to).deflate(1.0E-5f32 as f64);
    let moved_far = from.distance_to_sqr(to) > 0.9999900000002526f64 * 0.9999900000002526;
    let mut iterations = 0;
    let mut seen = std::mem::take(&mut level.entity_scratch.seen);
    for_each_block_between(from, to, target, &mut seen, |p, iteration| {
        if e.removed || iteration >= max_iterations {
            return false;
        }
        iterations = iteration;
        let state = level.get(p);
        if level.is_air(state) || visited.contains(&p) || !moved_into(level, e, p, state, from, to) {
            return true;
        }
        visited.push(p);
        let cell = Aabb::new(p.x as f64, p.y as f64, p.z as f64, p.x as f64 + 1.0, p.y as f64 + 1.0, p.z as f64 + 1.0);
        let precise = moved_far || target.intersects(&cell);
        level.with_listed(e, |level, k| crate::blocks::entity_inside(level, k, p, state, precise));
        true
    });
    level.entity_scratch.seen = seen;
    iterations + 1
}

/// Whether the entity moving from `from` to `to` touches the block's
/// `getEntityInsideCollisionShape`. That is the whole block for every
/// block the engine simulates but tripwire and powder snow, so the walk
/// reaching it is enough; string is felt only if the box sweeps through
/// its thin shape (`collidedWithShapeMovingFrom`). Powder snow is taken
/// as a whole block here, and any entity it is felt by is refused
/// (`blocks::entity_inside`). A block the entity misses is not counted as
/// visited, so a later part of the move can still reach it.
fn moved_into(level: &Level, e: &Entity, p: Pos, state: StateId, from: Vec3, to: Vec3) -> bool {
    match level.kind(state) {
        Kind::Tripwire => {
            let shape = crate::blocks::tripwire::shape(level, state).offset(p.x as f64, p.y as f64, p.z as f64);
            collided_along_vector(e.box_at(from), to - from, &[shape])
        }
        _ => true,
    }
}

/// `AABB.collidedAlongVector`: whether box `b` moving by `v` touches any of
/// `shapes`, tracing its centre against each shape grown by half the box.
fn collided_along_vector(b: Aabb, v: Vec3, shapes: &[Aabb]) -> bool {
    // `AABB.getCenter`, by `Mth.lerp`.
    let from: [f64; 3] = std::array::from_fn(|a| b.min[a] + 0.5 * (b.max[a] - b.min[a]));
    let to = [from[0] + v.x, from[1] + v.y, from[2] + v.z];
    let half: [f64; 3] = std::array::from_fn(|a| (b.max[a] - b.min[a]) * 0.5 - 1.0E-7);
    let contains = |s: &Aabb, p: [f64; 3]| (0..3).all(|a| p[a] >= s.min[a] && p[a] < s.max[a]);
    shapes.iter().any(|s| {
        let grown = s.inflate(half[0], half[1], half[2]);
        contains(&grown, to) || contains(&grown, from) || grown.clip(from, to).is_some()
    })
}

/// `Direction.axisStepOrder`: y first, then the larger of x and z.
fn axis_step_order(v: Vec3) -> [u8; 3] {
    if v.x.abs() < v.z.abs() { [1, 2, 0] } else { [1, 0, 2] }
}

/// `BlockGetter.forEachBlockIntersectedBetween`: visits the blocks a box
/// passes moving from `from` to `to` (`target` is the box at `to`), in the
/// order the game visits them, each with the step of the walk it came in,
/// until `visit` returns false. The box's blocks where it started come
/// first, then those its leading corner sweeps through, then the rest of
/// the blocks it ends in. `seen` is a buffer for the blocks visited so
/// far. Returns whether the walk got to its end.
fn for_each_block_between(
    from: Vec3,
    to: Vec3,
    target: Aabb,
    seen: &mut Vec<Pos>,
    mut visit: impl FnMut(Pos, i32) -> bool,
) -> bool {
    let travel = to - from;
    if travel.length_sqr() < (1.0E-5f32 * 1.0E-5f32) as f64 {
        // `BlockPos.betweenClosed`: x fastest, then y, then z.
        let (lo, hi) = (floor_corner(target.min), floor_corner(target.max));
        for z in lo[2]..=hi[2] {
            for y in lo[1]..=hi[1] {
                for x in lo[0]..=hi[0] {
                    if !visit(Pos::new(x, y, z), 0) {
                        return false;
                    }
                }
            }
        }
        return true;
    }
    seen.clear();
    let start = target.offset(-travel.x, -travel.y, -travel.z);
    let started = for_each_corner_in_direction(floor_corner(start.min), floor_corner(start.max), travel, |p| {
        seen.push(p);
        visit(p, 0)
    });
    if !started {
        return false;
    }
    let Some(iterations) = collisions_along_travel(seen, travel, target, &mut visit) else {
        return false;
    };
    for_each_corner_in_direction(floor_corner(target.min), floor_corner(target.max), travel, |p| {
        if seen.contains(&p) {
            return true;
        }
        seen.push(p);
        visit(p, iterations + 1)
    })
}

fn floor_corner(c: [f64; 3]) -> [i32; 3] {
    [floor(c[0]), floor(c[1]), floor(c[2])]
}

/// `BlockPos.betweenCornersInDirection`: visits the blocks between two
/// corners, starting from the corner the direction comes from, the axes
/// walked in `axisStepOrder` with the last fastest, until `visit` returns
/// false. Returns whether it visited them all.
fn for_each_corner_in_direction(a: [i32; 3], b: [i32; 3], dir: Vec3, mut visit: impl FnMut(Pos) -> bool) -> bool {
    let min = [a[0].min(b[0]), a[1].min(b[1]), a[2].min(b[2])];
    let max = [a[0].max(b[0]), a[1].max(b[1]), a[2].max(b[2])];
    let start: [i32; 3] = std::array::from_fn(|i| if dir.get(i as u8) >= 0.0 { min[i] } else { max[i] });
    let axes = axis_step_order(dir);
    let step = |axis: u8| if dir.get(axis) >= 0.0 { 1 } else { -1 };
    let span = |axis: u8| max[axis as usize] - min[axis as usize];
    for i in 0..=span(axes[0]) {
        for j in 0..=span(axes[1]) {
            for k in 0..=span(axes[2]) {
                let mut c = start;
                c[axes[0] as usize] += step(axes[0]) * i;
                c[axes[1] as usize] += step(axes[1]) * j;
                c[axes[2] as usize] += step(axes[2]) * k;
                if !visit(Pos::new(c[0], c[1], c[2])) {
                    return false;
                }
            }
        }
    }
    true
}

/// `BlockGetter.addCollisionsAlongTravel`: the walk of one corner of the
/// box through the blocks it crosses, visiting at each step the blocks of
/// the box there that `seen` doesn't hold yet. Returns the number of
/// steps, or `None` if `visit` stopped the walk.
fn collisions_along_travel(
    seen: &mut Vec<Pos>,
    delta: Vec3,
    target: Aabb,
    visit: &mut impl FnMut(Pos, i32) -> bool,
) -> Option<i32> {
    let size: [f64; 3] = std::array::from_fn(|a| target.max[a] - target.min[a]);
    let corner_dir = furthest_corner(delta);
    // `AABB.getCenter`, by `Mth.lerp`.
    let center: [f64; 3] = std::array::from_fn(|a| target.min[a] + 0.5 * (target.max[a] - target.min[a]));
    let to_corner: [f64; 3] = std::array::from_fn(|a| center[a] + size[a] * 0.5 * corner_dir[a] as f64);
    let d = [delta.x, delta.y, delta.z];
    let from_corner = [to_corner[0] - d[0], to_corner[1] - d[1], to_corner[2] - d[2]];
    let mut cell = floor_corner(from_corner);
    let sign: [i32; 3] = d.map(|v| {
        if v == 0.0 {
            0
        } else if v > 0.0 {
            1
        } else {
            -1
        }
    });
    let t_delta: [f64; 3] = std::array::from_fn(|a| if sign[a] == 0 { f64::MAX } else { sign[a] as f64 / d[a] });
    let mut t: [f64; 3] = std::array::from_fn(|a| {
        let frac = from_corner[a] - floor(from_corner[a]) as f64;
        t_delta[a] * if sign[a] > 0 { 1.0 - frac } else { frac }
    });
    let mut iterations = 0;
    while t[0] <= 1.0 || t[1] <= 1.0 || t[2] <= 1.0 {
        let a = if t[0] < t[1] {
            if t[0] < t[2] { 0 } else { 2 }
        } else if t[1] < t[2] {
            1
        } else {
            2
        };
        cell[a] += sign[a];
        t[a] += t_delta[a];
        let cell_box = Aabb::new(
            cell[0] as f64,
            cell[1] as f64,
            cell[2] as f64,
            cell[0] as f64 + 1.0,
            cell[1] as f64 + 1.0,
            cell[2] as f64 + 1.0,
        );
        if let Some(hit) = cell_box.clip(from_corner, to_corner) {
            iterations += 1;
            let e = 1.0E-5f32 as f64;
            let mut opposite = [0; 3];
            for k in 0..3 {
                let h = hit[k].clamp(cell[k] as f64 + e, cell[k] as f64 + 1.0 - e);
                opposite[k] = floor(h - size[k] * corner_dir[k] as f64);
            }
            let step = iterations;
            let walked = for_each_corner_in_direction(cell, opposite, delta, |p| {
                if seen.contains(&p) {
                    return true;
                }
                seen.push(p);
                visit(p, step)
            });
            if !walked {
                return None;
            }
        }
    }
    Some(iterations)
}

/// `BlockGetter.getFurthestCorner`, as 26.3 has it.
fn furthest_corner(d: Vec3) -> [i32; 3] {
    let (xd, yd, zd) = (d.x.abs(), d.y.abs(), d.z.abs());
    let xs = if d.x >= 0.0 { 1 } else { -1 };
    let ys = if d.y >= 0.0 { 1 } else { -1 };
    let zs = if d.z >= 0.0 { 1 } else { -1 };
    if xd <= yd && xd <= zd {
        [-xs, -zs, ys]
    } else if yd <= zd {
        [zs, -ys, -xs]
    } else {
        [-ys, xs, -zs]
    }
}

/// `applyGravity`.
pub fn apply_gravity(e: &mut Entity, gravity: f64) {
    e.motion = Vec3::new(e.motion.x, e.motion.y - gravity, e.motion.z);
}

/// `Entity.baseTick`, for what the engine's entities do in it.
pub fn base_tick(level: &Level, e: &mut Entity) {
    fluid::update_in_water(level, e);
    e.first_tick = false;
}

/// `updateInWaterStateAndDoFluidPushing`.
pub fn update_in_water(level: &Level, e: &mut Entity) {
    fluid::update_in_water(level, e);
}

/// `Entity.move(MoverType.SELF, movement)`.
pub fn move_entity(level: &mut Level, e: &mut Entity, mut movement: Vec3) {
    if e.no_physics {
        e.pos = e.pos + movement;
        e.horizontal_collision = false;
        e.vertical_collision = false;
        return;
    }
    if e.stuck_speed_multiplier.length_sqr() > 1.0E-7 {
        let s = e.stuck_speed_multiplier;
        movement = movement.multiply(s.x, s.y, s.z);
        e.stuck_speed_multiplier = Vec3::ZERO;
        e.motion = Vec3::ZERO;
    }
    let mut shapes = std::mem::take(&mut level.entity_scratch.shapes);
    let moved = collide(level, e, movement, &mut shapes);
    level.entity_scratch.shapes = shapes;
    let d = moved.length_sqr();
    if d > 1.0E-7 || movement.length_sqr() - d < 1.0E-7 {
        let from = e.pos;
        e.pos = e.pos + moved;
        add_movement(e, Movement { from, to: e.pos, asked: Some(movement) });
    }
    let xc = !mth_equal(movement.x, moved.x);
    let zc = !mth_equal(movement.z, moved.z);
    e.horizontal_collision = xc || zc;
    e.vertical_collision = movement.y != moved.y;
    let below = e.vertical_collision && movement.y < 0.0;
    e.on_ground = below;
    check_supporting_block(level, e, below, Some(moved));
    if e.removed {
        return;
    }
    if e.horizontal_collision {
        let m = e.motion;
        e.motion = Vec3::new(if xc { 0.0 } else { m.x }, m.y, if zc { 0.0 } else { m.z });
    }
    if movement.y != moved.y {
        // `Block.updateEntityMovementAfterFallOn`, for the block under the
        // entity's feet.
        let p = e.on_pos(level, 0.2);
        let state = level.get(p);
        match level.kind(state) {
            // `SlimeBlock.bounceUp`: nothing the engine has steps carefully.
            // 26.3 captures show an entity only bounces when it was already
            // moving down at the start of its tick; one that was still lands
            // as on any other block.
            Kind::Slime if e.start_motion_y < 0.0 => {
                let f = if e.is_living() { 1.0 } else { 0.8f32 as f64 };
                let y = slime_impact_speed(e.motion.y, moved.y / movement.y);
                e.motion = Vec3::new(e.motion.x, -y * f, e.motion.z);
            }
            _ => e.motion = e.motion.multiply(1.0, 0.0, 1.0),
        }
    }
    let f = block_speed_factor(level, e);
    if f != 1.0 {
        e.motion = e.motion.multiply(f as f64, 1.0, f as f64);
    }
    if let Data::Minecart(_) = e.data {
        minecart_after_move(level, e);
    }
}

/// The end of `AbstractMinecart.move` (`OldMinecartBehavior`): after
/// `Entity.move`, a cart feels the blocks where it is, and forgets its
/// moves.
fn minecart_after_move(level: &mut Level, e: &mut Entity) {
    let at = e.pos;
    apply_effects_between(level, e, at, at);
    e.movements.clear();
}

/// The vertical motion a falling entity bounces off slime with (still
/// negative), given its motion `y` for the tick and the fraction `t` of the
/// tick's fall it got through before landing. 26.3 captures show the bounce
/// is not just `-y`: it is as if the entity had picked up gravity and drag
/// for that part of the tick, `(y - g*t) * (1 - (1 - drag)*t)`, for the
/// entities' gravity `g` of 0.04 and drag of 0.98. This was fitted exactly
/// over two dozen bounces of items and TNT dropped from different heights
/// (`slime_bounce_heights`, `slime_drop`).
fn slime_impact_speed(y: f64, t: f64) -> f64 {
    const G: f64 = 0.04;
    (y - G * t) * (1.0 - (1.0 - DRAG) * t)
}

/// `Block.stepOn`, for the block under the entity's feet.
pub fn step_on(level: &Level, e: &mut Entity) {
    let state = level.get(e.on_pos(level, 0.2));
    if level.kind(state) == Kind::Slime {
        let d = e.motion.y.abs();
        if d < 0.1 {
            let f = 0.4 + d * 0.2;
            e.motion = e.motion.multiply(f, 1.0, f);
        }
    }
}

/// `getBlockSpeedFactor`.
fn block_speed_factor(level: &Level, e: &Entity) -> f32 {
    let state = level.get(e.block_position());
    let f = level.physics().factors(level.block(state)).speed;
    if matches!(level.kind(state), Kind::Water | Kind::BubbleColumn) {
        return f;
    }
    if f == 1.0 {
        let below = level.get(e.pos_below_affecting_movement(level));
        level.physics().factors(level.block(below)).speed
    } else {
        f
    }
}

/// `checkSupportingBlock`.
fn check_supporting_block(level: &Level, e: &mut Entity, on_ground: bool, movement: Option<Vec3>) {
    if on_ground {
        let b = e.bounding_box();
        let slab = Aabb::new(b.min[0], b.min[1] - 1.0E-6, b.min[2], b.max[0], b.min[1], b.max[2]);
        let mut found = find_supporting_block(level, e, slab);
        if found.is_some() || e.on_ground_no_blocks {
            e.main_support = found;
        } else if let Some(m) = movement {
            found = find_supporting_block(level, e, slab.offset(-m.x, 0.0, -m.z));
            e.main_support = found;
        }
        e.on_ground_no_blocks = found.is_none();
    } else {
        e.on_ground_no_blocks = false;
        e.main_support = None;
    }
}

/// `CollisionGetter.findSupportingBlock`: of the blocks under the entity,
/// the one whose centre is nearest its position.
fn find_supporting_block(level: &Level, e: &Entity, b: Aabb) -> Option<Pos> {
    let mut best: Option<Pos> = None;
    let mut d = f64::MAX;
    for_each_block_collision(level, b, |p, _| {
        let c = Vec3::new(p.x as f64 + 0.5, p.y as f64 + 0.5, p.z as f64 + 0.5);
        let dist = c.distance_to_sqr(e.pos);
        let greater = |q: Pos| (p.y, p.z, p.x) > (q.y, q.z, q.x);
        if dist < d || dist == d && best.is_none_or(greater) {
            best = Some(p);
            d = dist;
        }
        true
    });
    best
}

/// `BlockCollisions`: visits each block whose collision shape touches `b`,
/// in the game's search order, with its collision boxes relative to the
/// block, until `visit` returns false. Returns whether it visited them all.
fn for_each_block_collision(level: &Level, b: Aabb, mut visit: impl FnMut(Pos, &[Aabb]) -> bool) -> bool {
    let physics = level.physics();
    let x0 = floor(b.min[0] - 1.0E-7) - 1;
    let x1 = floor(b.max[0] + 1.0E-7) + 1;
    let y0 = floor(b.min[1] - 1.0E-7) - 1;
    let y1 = floor(b.max[1] + 1.0E-7) + 1;
    let z0 = floor(b.min[2] - 1.0E-7) - 1;
    let z1 = floor(b.max[2] + 1.0E-7) + 1;
    for z in z0..=z1 {
        for y in y0..=y1 {
            for x in x0..=x1 {
                let edges = (x == x0 || x == x1) as u8 + (y == y0 || y == y1) as u8 + (z == z0 || z == z1) as u8;
                if edges == 3 {
                    continue;
                }
                let p = Pos::new(x, y, z);
                let state = level.get(p);
                // A moving piston's shape is dynamic, which the game counts
                // as a large shape.
                let moving = level.kind(state) == Kind::MovingPiston;
                if edges == 1 && !(moving || physics.has_large_shape(state)) || edges == 2 && !moving {
                    continue;
                }
                let touches =
                    |shape: &[Aabb]| shape.iter().any(|s| s.offset(x as f64, y as f64, z as f64).intersects(&b));
                let go_on = if moving {
                    let shape = moving_collision_shape(level, p);
                    !touches(&shape) || visit(p, &shape)
                } else {
                    let shape = physics.shape(state);
                    !touches(shape) || visit(p, shape)
                };
                if !go_on {
                    return false;
                }
            }
        }
    }
    true
}

/// `getCollisionShape` for the state at `p`.
fn collision_shape(level: &Level, p: Pos, state: StateId) -> Vec<Aabb> {
    if level.kind(state) == Kind::MovingPiston {
        return moving_collision_shape(level, p);
    }
    level.physics().shape(state).to_vec()
}

/// `PistonMovingBlockEntity.getCollisionShape`: a retracting piston base
/// is its extended self, and what moves (the block, or the head) sits
/// where its progress has taken it.
fn moving_collision_shape(level: &Level, p: Pos) -> Vec<Aabb> {
    let Some(crate::blocks::BlockEntity::Piston(m)) = level.block_entities.get(&p) else {
        return Vec::new();
    };
    let physics = level.physics();
    let base_kind = level.kind(m.moved_state);
    let is_base = matches!(base_kind, Kind::Piston | Kind::StickyPiston);
    let mut out = if !m.extending && m.is_source_piston && is_base {
        physics.shape(level.with_bool(m.moved_state, "extended", true)).to_vec()
    } else {
        Vec::new()
    };
    let movement = if m.extending { m.direction } else { m.direction.opposite() };
    if m.progress < 1.0 && level.piston_noclip == Some(movement) {
        return out;
    }
    let moved = if m.is_source_piston {
        let head = level.default_state(level.block_id("piston_head"));
        let head = level.with_dir(head, "facing", m.direction);
        let head = level.with(head, "type", if base_kind == Kind::StickyPiston { "sticky" } else { "normal" });
        level.with_bool(head, "short", m.extending != (1.0 - m.progress < 0.25))
    } else {
        m.moved_state
    };
    let progress = if m.extending { m.progress - 1.0 } else { 1.0 - m.progress } as f64;
    let [dx, dy, dz] = m.direction.step();
    out.extend(
        physics.shape(moved).iter().map(|b| b.offset(dx as f64 * progress, dy as f64 * progress, dz as f64 * progress)),
    );
    out
}

/// `Entity.moveTowardsClosestSpace`: an entity stuck in a block heads for
/// the nearest side whose neighbour isn't a full block (up if none is
/// nearer), at a speed from its own random.
pub fn move_towards_closest_space(level: &Level, e: &mut Entity, x: f64, y: f64, z: f64) {
    use crate::pos::Dir;
    let pos = Pos::new(floor(x), floor(y), floor(z));
    let offset = [x - pos.x as f64, y - pos.y as f64, z - pos.z as f64];
    let mut closest_dir = Dir::Up;
    let mut closest = f64::MAX;
    for d in [Dir::North, Dir::South, Dir::West, Dir::East, Dir::Up] {
        let p = pos.offset(d);
        if !collision_shape_full_block(level, p) {
            let axis_offset = offset[d.axis() as usize];
            let positive = d.step().iter().sum::<i32>() > 0;
            let d_offset = if positive { 1.0 - axis_offset } else { axis_offset };
            if d_offset < closest {
                closest = d_offset;
                closest_dir = d;
            }
        }
    }
    let speed = e.random.next_float() * 0.2 + 0.1;
    let step = closest_dir.step().iter().sum::<i32>() as f32;
    let v = (step * speed) as f64;
    let m = e.motion.scale(0.75);
    e.motion = match closest_dir.axis() {
        0 => Vec3::new(v, m.y, m.z),
        1 => Vec3::new(m.x, v, m.z),
        _ => Vec3::new(m.x, m.y, v),
    };
}

/// `isCollisionShapeFullBlock`: whether the block's collision shape,
/// a moving piston's as it is now included, is exactly the whole block.
fn collision_shape_full_block(level: &Level, p: Pos) -> bool {
    let state = level.get(p);
    if level.kind(state) != Kind::MovingPiston {
        return level.physics().is_full_block(state);
    }
    let boxes = moving_collision_shape(level, p);
    if boxes.iter().any(|b| (0..3).any(|a| b.min[a] < 0.0 || b.max[a] > 1.0)) {
        return false;
    }
    // The boxes lie inside the block; they fill it if their union's volume
    // is the block's, counted over the grid their edges make.
    let mut cuts: [Vec<f64>; 3] = Default::default();
    for (a, c) in cuts.iter_mut().enumerate() {
        c.extend([0.0, 1.0]);
        for b in &boxes {
            c.extend([b.min[a], b.max[a]]);
        }
        c.sort_by(f64::total_cmp);
        c.dedup();
    }
    for xs in cuts[0].windows(2) {
        for ys in cuts[1].windows(2) {
            for zs in cuts[2].windows(2) {
                let mid = [(xs[0] + xs[1]) / 2.0, (ys[0] + ys[1]) / 2.0, (zs[0] + zs[1]) / 2.0];
                if !boxes.iter().any(|b| (0..3).all(|a| b.min[a] <= mid[a] && mid[a] <= b.max[a])) {
                    return false;
                }
            }
        }
    }
    true
}

/// `noCollision(entity, box)` against blocks.
pub fn no_block_collision(level: &Level, b: Aabb) -> bool {
    for_each_block_collision(level, b, |_, _| false)
}

/// The boxes a move collides with, in world coordinates: each entity's or
/// block's boxes together, in the order the game collects them.
#[derive(Default)]
struct Shapes {
    boxes: Vec<Aabb>,
    /// Where each entity's or block's boxes end in `boxes`.
    ends: Vec<usize>,
}

impl Shapes {
    fn clear(&mut self) {
        self.boxes.clear();
        self.ends.clear();
    }

    fn push(&mut self, boxes: impl IntoIterator<Item = Aabb>) {
        self.boxes.extend(boxes);
        self.ends.push(self.boxes.len());
    }

    fn is_empty(&self) -> bool {
        self.ends.is_empty()
    }

    /// Each entity's or block's boxes.
    fn iter(&self) -> impl Iterator<Item = &[Aabb]> {
        let starts = std::iter::once(0).chain(self.ends.iter().copied());
        starts.zip(self.ends.iter().copied()).map(|(s, e)| &self.boxes[s..e])
    }
}

/// `Entity.collide`, for entities that can't step up. `shapes` is a buffer
/// for the boxes the move might hit.
fn collide(level: &Level, e: &Entity, movement: Vec3, shapes: &mut Shapes) -> Vec3 {
    if movement.length_sqr() == 0.0 {
        return movement;
    }
    let b = e.bounding_box();
    let area = b.expand_towards(movement);
    shapes.clear();
    entity_collisions(level, e, area, shapes);
    for_each_block_collision(level, area, |p, shape| {
        shapes.push(shape.iter().map(|s| s.offset(p.x as f64, p.y as f64, p.z as f64)));
        true
    });
    if shapes.is_empty() {
        return movement;
    }
    let mut result = Vec3::ZERO;
    let order: [u8; 3] = if movement.x.abs() < movement.z.abs() { [1, 2, 0] } else { [1, 0, 2] };
    for axis in order {
        let d = movement.get(axis);
        if d != 0.0 {
            let v = shapes_collide(axis, b.moved(result), shapes, d);
            result = result.with(axis, v);
        }
    }
    result
}

/// `Level.getEntityCollisions`: the boxes of entities `e` can't pass
/// through (`canCollideWith`), added to `shapes`. They come before the
/// blocks. Of the engine's entities only a minecart collides with others,
/// and only with pushable ones, which are minecarts
/// (`AbstractBoat.canVehicleCollide`); nothing collides with a minecart
/// otherwise, so an item falls through one.
fn entity_collisions(level: &Level, e: &Entity, area: Aabb, shapes: &mut Shapes) {
    let size = (0..3).map(|a| area.max[a] - area.min[a]).sum::<f64>() / 3.0;
    if size < 1.0E-7 || !matches!(e.data, Data::Minecart(_)) {
        return;
    }
    let area = area.inflate(1.0E-7, 1.0E-7, 1.0E-7);
    for o in &level.entities {
        if !o.removed && o.id != e.id && matches!(o.data, Data::Minecart(_)) {
            let ob = o.bounding_box();
            if ob.intersects(&area) {
                shapes.push([ob]);
            }
        }
    }
}

/// `Shapes.collide`.
fn shapes_collide(axis: u8, b: Aabb, shapes: &Shapes, mut dist: f64) -> f64 {
    for shape in shapes.iter() {
        if dist.abs() < 1.0E-7 {
            return 0.0;
        }
        dist = shape_collide(axis, b, shape, dist);
    }
    dist
}

/// `VoxelShape.collide`, box by box.
fn shape_collide(axis: u8, b: Aabb, shape: &[Aabb], mut dist: f64) -> f64 {
    if dist.abs() < 1.0E-7 {
        return 0.0;
    }
    let others = match axis {
        0 => [1, 2],
        1 => [0, 2],
        _ => [0, 1],
    };
    let a = axis as usize;
    for s in shape {
        let overlaps = others.iter().all(|&o| s.max[o] > b.min[o] + 1.0E-7 && s.min[o] <= b.max[o] - 1.0E-7);
        if !overlaps {
            continue;
        }
        if dist > 0.0 {
            if s.min[a] > b.max[a] - 1.0E-7 {
                dist = dist.min(s.min[a] - b.max[a]);
            }
        } else if s.max[a] < b.min[a] + 1.0E-7 {
            dist = dist.max(s.max[a] - b.min[a]);
        }
    }
    dist
}

/// What snapshots need: entities whose box meets `[min, max + 1)`.
pub fn entities_in(level: &Level, min: Pos, max: Pos) -> Vec<&Entity> {
    let b =
        Aabb::new(min.x as f64, min.y as f64, min.z as f64, max.x as f64 + 1.0, max.y as f64 + 1.0, max.z as f64 + 1.0);
    let mut v: Vec<&Entity> = level.entities.iter().filter(|e| !e.removed && e.bounding_box().intersects(&b)).collect();
    v.sort_by_key(|e| e.id);
    v
}

/// `BlockState.canBeReplaced()`: the `replaceable` property.
pub fn replaceable(level: &Level, state: StateId) -> bool {
    matches!(level.kind(state), Kind::Air | Kind::Water)
}

/// `FallingBlock.isFree`.
pub fn is_free(level: &Level, state: StateId) -> bool {
    replaceable(level, state) || level.reg.name(state) == "minecraft:fire" || level.reg.name(state) == "minecraft:lava"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::Rules;

    /// No capture can show this, because the entities the engine simulates
    /// make one move before they feel blocks, and a later leg of that move
    /// keeps the height of the leg that missed. An entity whose moves this
    /// tick pass over a string, then through it, must still press it.
    #[test]
    fn a_string_missed_by_one_move_is_pressed_by_the_next() {
        let mut level = Level::new(Rules::for_version("26.3").unwrap());
        let string = level.reg.parse("tripwire[attached=false,disarmed=false,powered=false]").unwrap();
        let pos = Pos::new(0, 0, 0);
        level.set_block(pos, string, crate::level::flags::CLIENTS);
        let item = Data::Item(item::Item::new(Stack::new("stone", 1)));
        let e = Entity::new(&mut level, item, Vec3::new(0.5, 0.3, 0.5));
        level.add_entity(e);
        let mut e = level.take_entity(0);
        // Over the string's half-block shape, then down through it.
        let over = Movement { from: Vec3::new(0.5, 0.9, 0.5), to: Vec3::new(0.5, 0.8, 0.5), asked: None };
        let through = Movement { from: over.to, to: e.pos, asked: None };
        e.final_movements = vec![over, through];
        apply_effects_for_last_movements(&mut level, &mut e);
        level.put_entity(e);
        assert!(level.flag(level.get(pos), "powered"));
    }

    fn try_summon(entity: &str, nbt: &str) -> Result<usize, String> {
        let mut level = Level::new(Rules::for_version("26.3").unwrap());
        level.simulate_entities = true;
        summon(&mut level, entity, Vec3::new(0.5, 100.0, 0.5), None, Some(nbt)).map_err(|e| e.to_string())?;
        Ok(level.entities.len())
    }

    #[test]
    fn item_stacks_with_components_are_refused_by_name() {
        assert_eq!(try_summon("item", r#"{Item:{id:"ender_pearl",count:10}}"#), Ok(1));
        // An empty patch is the same as none.
        assert_eq!(try_summon("item", r#"{Item:{id:"ender_pearl",count:10,components:{}}}"#), Ok(1));
        assert_eq!(
            try_summon("item", r#"{Item:{id:"ender_pearl",count:10,components:{"minecraft:max_stack_size":64}}}"#),
            Err("the item component minecraft:max_stack_size in item data (Item) is not supported by the engine yet"
                .to_string())
        );
        assert_eq!(
            try_summon("item", r#"{Item:{id:"stone",components:{"minecraft:custom_name":"Rock"}}}"#),
            Err("the item component minecraft:custom_name in item data (Item) is not supported by the engine yet"
                .to_string())
        );
        assert_eq!(
            try_summon("item", r#"{Item:{id:"stone",Count:2b}}"#),
            Err("item data Item.Count is not supported by the engine yet".to_string())
        );
        assert_eq!(try_summon("chest_minecart", r#"{Items:[{Slot:0b,id:"stone",count:3}]}"#), Ok(1));
        assert_eq!(
            try_summon(
                "hopper_minecart",
                r#"{Items:[{Slot:0b,id:"stone",count:3},{Slot:1b,id:"snowball",count:4,components:{"minecraft:max_stack_size":64}}]}"#
            ),
            Err("the item component minecraft:max_stack_size in hopper_minecart data (Items[1]) is not supported by the engine yet"
                .to_string())
        );
    }
}
