//! Entities: items, falling blocks, primed TNT and minecarts, moved the way the game's
//! `Entity.move` moves them, against the blocks' collision boxes.
//!
//! The level keeps its entities in the order they were added, which is the
//! order the game ticks them in. Entities added while entities are ticking
//! wait for the next tick, as in `EntityTickList`.

pub mod explosion;
mod falling;
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
    /// A slot whose entity is being ticked, or one that was removed.
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
            Data::Minecart(ref m) if m.furnace => "minecraft:furnace_minecart",
            Data::Minecart(_) => "minecraft:minecart",
            Data::Gone => "minecraft:none",
        }
    }

    /// `EntityDimensions`: width and height, as the game's floats.
    fn dimensions(&self) -> (f32, f32) {
        match self.data {
            Data::Item(_) => (0.25, 0.25),
            Data::Minecart(_) => (0.98, 0.7),
            Data::FallingBlock(_) | Data::Tnt(_) | Data::Gone => (0.98, 0.98),
        }
    }

    /// `getBoundingBox`, from `EntityDimensions.makeBoundingBox`.
    pub fn bounding_box(&self) -> Aabb {
        let (w, h) = self.dimensions();
        let f = (w / 2.0) as f64;
        let h = h as f64;
        let p = self.pos;
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
            Data::Minecart(_) | Data::Gone => None,
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

    /// `getBlockPosBelowThatAffectsMyMovement`.
    pub fn pos_below_affecting_movement(&self, level: &Level) -> Pos {
        self.on_pos(level, 0.500001)
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
            let mut e = std::mem::take(&mut self.entities[i]);
            e.tick_count += 1;
            tick(self, &mut e);
            self.entities[i] = e;
        }
        self.entities.retain(|e| !e.removed);
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
    match e.data {
        Data::Item(_) => item::tick(level, e),
        Data::FallingBlock(_) => falling::tick(level, e),
        Data::Tnt(_) => tnt::tick(level, e),
        Data::Minecart(_) => minecart::tick(level, e),
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
/// (a saved entity carries its UUID, rotation, air supply and the like).
/// Returns whether the entity was created.
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

/// The summon data keys the engine reads for each type it simulates.
fn known_data(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        "item" => &["Item", "PickupDelay", "Age"],
        "tnt" => &["fuse"],
        "falling_block" => &["BlockState", "Time", "DropItem", "CancelDrop"],
        "minecart" | "furnace_minecart" => &[],
        _ => return None,
    })
}

fn parse_data(nbt: Option<&str>) -> Result<snbt::Tag> {
    Ok(match nbt {
        Some(t) => snbt::parse(t).with_context(|| format!("summon data {t}"))?,
        None => snbt::Tag::Compound(Default::default()),
    })
}

fn spawn(level: &mut Level, name: &str, tag: &snbt::Tag, at: Vec3, motion: Option<Vec3>) -> Result<()> {
    let num = |key: &str| tag.get(key).and_then(|t| t.number());
    let data = match name {
        "item" => {
            let it = tag.get("Item").context("an item needs Item")?;
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
        _ => Data::Minecart(minecart::Minecart { furnace: name == "furnace_minecart" }),
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

/// `Mth.equal(double, double)`.
fn mth_equal(a: f64, b: f64) -> bool {
    (b - a).abs() < 1.0E-5f32 as f64
}

/// `Entity.applyEffectsFromBlocks(from, to)`: every block the entity
/// passed through this tick, from where it started to where it is now,
/// feels it once. Blocks are visited in the order the move reaches them.
/// The entity being ticked is out of the level's list, so it goes back on
/// the end while blocks see it, and comes back changed.
pub fn apply_effects_from_blocks(level: &mut Level, e: &mut Entity, from: Vec3) {
    for p in blocks_between(from, e.pos, e.bounding_box().deflate(1.0E-5f32 as f64)) {
        let state = level.get(p);
        if !level.is_air(state) && !e.removed {
            level.entities.push(std::mem::take(e));
            crate::blocks::entity_inside(level, p, state);
            *e = level.entities.pop().unwrap();
        }
    }
}

/// `BlockGetter.forEachBlockIntersectedBetween`, as far as 26.3 captures
/// show it: the blocks the box ends up in, in the order the move reaches
/// them. Blocks the box starts in come first; then the walk steps through
/// the blocks its corner crosses.
fn blocks_between(from: Vec3, to: Vec3, target: Aabb) -> Vec<Pos> {
    let lo = [floor(target.min[0]), floor(target.min[1]), floor(target.min[2])];
    let hi = [floor(target.max[0]), floor(target.max[1]), floor(target.max[2])];
    let mut out: Vec<Pos> = Vec::new();
    let visit = |out: &mut Vec<Pos>, from: [i32; 3], to: [i32; 3]| {
        for x in from[0].max(lo[0])..=to[0].min(hi[0]) {
            for y in from[1].max(lo[1])..=to[1].min(hi[1]) {
                for z in from[2].max(lo[2])..=to[2].min(hi[2]) {
                    let p = Pos::new(x, y, z);
                    if !out.contains(&p) {
                        out.push(p);
                    }
                }
            }
        }
    };
    let travel = to - from;
    if travel.length_sqr() >= (1.0E-5f32 as f64).powi(2) {
        let size = [target.max[0] - target.min[0], target.max[1] - target.min[1], target.max[2] - target.min[2]];
        let end = [target.min[0], target.min[1], target.min[2]];
        let start = [end[0] - travel.x, end[1] - travel.y, end[2] - travel.z];
        let d = [travel.x, travel.y, travel.z];
        let mut cell = [floor(start[0]), floor(start[1]), floor(start[2])];
        let corner = |cell: [i32; 3], hit: [f64; 3]| -> [i32; 3] {
            let mut c = [0; 3];
            for a in 0..3 {
                let h = hit[a].clamp(cell[a] as f64 + 1.0E-5f32 as f64, cell[a] as f64 + 1.0 - 1.0E-5f32 as f64);
                c[a] = floor(h + size[a]);
            }
            c
        };
        visit(&mut out, cell, corner(cell, start));
        let mut step = [0i32; 3];
        let mut t_delta = [f64::MAX; 3];
        let mut t = [f64::MAX; 3];
        for a in 0..3 {
            step[a] = if d[a] > 0.0 {
                1
            } else if d[a] < 0.0 {
                -1
            } else {
                0
            };
            if step[a] != 0 {
                t_delta[a] = step[a] as f64 / d[a];
                let frac = start[a] - start[a].floor();
                t[a] = t_delta[a] * if step[a] > 0 { 1.0 - frac } else { frac };
            }
        }
        while t[0] <= 1.0 || t[1] <= 1.0 || t[2] <= 1.0 {
            let a = if t[0] < t[1] {
                if t[0] < t[2] { 0 } else { 2 }
            } else if t[1] < t[2] {
                1
            } else {
                2
            };
            cell[a] += step[a];
            t[a] += t_delta[a];
            let cell_box = Aabb::new(
                cell[0] as f64,
                cell[1] as f64,
                cell[2] as f64,
                cell[0] as f64 + 1.0,
                cell[1] as f64 + 1.0,
                cell[2] as f64 + 1.0,
            );
            if let Some(hit) = cell_box.clip(start, end) {
                visit(&mut out, cell, corner(cell, hit));
            }
        }
    }
    // `BlockPos.betweenClosed`: x fastest, then y, then z.
    for z in lo[2]..=hi[2] {
        for y in lo[1]..=hi[1] {
            for x in lo[0]..=hi[0] {
                let p = Pos::new(x, y, z);
                if !out.contains(&p) {
                    out.push(p);
                }
            }
        }
    }
    out
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
    if e.stuck_speed_multiplier.length_sqr() > 1.0E-7 {
        let s = e.stuck_speed_multiplier;
        movement = movement.multiply(s.x, s.y, s.z);
        e.stuck_speed_multiplier = Vec3::ZERO;
        e.motion = Vec3::ZERO;
    }
    let moved = collide(level, e, movement);
    let d = moved.length_sqr();
    if d > 1.0E-7 || movement.length_sqr() - d < 1.0E-7 {
        e.pos = e.pos + moved;
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
            Kind::Slime => level.unsupported("an entity landed on a slime block"),
            _ => e.motion = e.motion.multiply(1.0, 0.0, 1.0),
        }
    }
    let f = block_speed_factor(level, e);
    if f != 1.0 {
        e.motion = e.motion.multiply(f as f64, 1.0, f as f64);
    }
}

/// `getBlockSpeedFactor`.
fn block_speed_factor(level: &Level, e: &Entity) -> f32 {
    let state = level.get(e.block_position());
    let f = level.physics().factors(level.block(state)).speed;
    if level.kind(state) == Kind::Water {
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
    for (p, _) in block_collisions(level, b) {
        let c = Vec3::new(p.x as f64 + 0.5, p.y as f64 + 0.5, p.z as f64 + 0.5);
        let dist = c.distance_to_sqr(e.pos);
        let greater = |q: Pos| (p.y, p.z, p.x) > (q.y, q.z, q.x);
        if dist < d || dist == d && best.is_none_or(greater) {
            best = Some(p);
            d = dist;
        }
    }
    best
}

/// `BlockCollisions`: the collision boxes, in world coordinates, of each
/// block whose shape touches `b`, in the game's search order.
pub fn block_collisions(level: &Level, b: Aabb) -> Vec<(Pos, Vec<Aabb>)> {
    let physics = level.physics();
    let x0 = floor(b.min[0] - 1.0E-7) - 1;
    let x1 = floor(b.max[0] + 1.0E-7) + 1;
    let y0 = floor(b.min[1] - 1.0E-7) - 1;
    let y1 = floor(b.max[1] + 1.0E-7) + 1;
    let z0 = floor(b.min[2] - 1.0E-7) - 1;
    let z1 = floor(b.max[2] + 1.0E-7) + 1;
    let mut out = Vec::new();
    for z in z0..=z1 {
        for y in y0..=y1 {
            for x in x0..=x1 {
                let edges = [x == x0 || x == x1, y == y0 || y == y1, z == z0 || z == z1].iter().filter(|e| **e).count();
                if edges == 3 {
                    continue;
                }
                let p = Pos::new(x, y, z);
                let state = level.get(p);
                if edges == 1 && !physics.has_large_shape(state)
                    || edges == 2 && level.kind(state) != Kind::MovingPiston
                {
                    continue;
                }
                let shape = collision_shape(level, p, state);
                if shape.is_empty() {
                    continue;
                }
                let moved: Vec<Aabb> = shape.iter().map(|s| s.offset(x as f64, y as f64, z as f64)).collect();
                if moved.iter().any(|s| s.intersects(&b)) {
                    out.push((p, moved));
                }
            }
        }
    }
    out
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

/// `noCollision(entity, box)` against blocks.
pub fn no_block_collision(level: &Level, b: Aabb) -> bool {
    block_collisions(level, b).is_empty()
}

/// `Entity.collide`, for entities that can't step up.
fn collide(level: &Level, e: &Entity, movement: Vec3) -> Vec3 {
    if movement.length_sqr() == 0.0 {
        return movement;
    }
    let b = e.bounding_box();
    let shapes = block_collisions(level, b.expand_towards(movement));
    if shapes.is_empty() {
        return movement;
    }
    let mut result = Vec3::ZERO;
    let order: [u8; 3] = if movement.x.abs() < movement.z.abs() { [1, 2, 0] } else { [1, 0, 2] };
    for axis in order {
        let d = movement.get(axis);
        if d != 0.0 {
            let v = shapes_collide(axis, b.moved(result), &shapes, d);
            result = result.with(axis, v);
        }
    }
    result
}

/// `Shapes.collide`.
fn shapes_collide(axis: u8, b: Aabb, shapes: &[(Pos, Vec<Aabb>)], mut dist: f64) -> f64 {
    for (_, shape) in shapes {
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
    let others: Vec<usize> = (0..3).filter(|a| *a != axis as usize).collect();
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
