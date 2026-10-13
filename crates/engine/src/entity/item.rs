//! `ItemEntity`: a dropped stack.

use super::{DRAG, Data, Entity, apply_gravity, move_entity, no_block_collision};
use crate::level::Level;
use crate::physics::Vec3;

pub use crate::blocks::container::Stack;

/// `pickupDelay` that never counts down.
const INFINITE_PICKUP_DELAY: i32 = 32767;
/// `age` that never counts up.
const INFINITE_LIFETIME: i32 = -32768;
const LIFETIME: i32 = 6000;

#[derive(Clone, Debug)]
pub struct Item {
    pub stack: Stack,
    pub pickup_delay: i32,
    pub age: i32,
    pub health: i32,
}

impl Item {
    pub fn new(stack: Stack) -> Item {
        let id = stack.id.strip_prefix("minecraft:").unwrap_or(&stack.id).to_string();
        Item { stack: Stack::new(&id, stack.count), pickup_delay: 0, age: 0, health: 5 }
    }

    fn mergable(&self) -> bool {
        self.pickup_delay != INFINITE_PICKUP_DELAY
            && self.age != INFINITE_LIFETIME
            && self.age < LIFETIME
            && self.stack.count < crate::items::max_stack_size(&self.stack.id)
    }
}

fn item(e: &mut Entity) -> &mut Item {
    match &mut e.data {
        Data::Item(i) => i,
        _ => unreachable!("not an item"),
    }
}

/// `new ItemEntity(level, x, y, z, stack)`: a random sideways toss and a
/// small hop, drawn from the item's own random source.
pub fn new_tossed(level: &mut Level, pos: Vec3, stack: Stack) -> Entity {
    let mut e = Entity::new(level, Data::Item(Item::new(stack)), pos);
    let dx = e.random.next_double() * 0.2 - 0.1;
    let dz = e.random.next_double() * 0.2 - 0.1;
    e.motion = Vec3::new(dx, 0.2, dz);
    e
}

/// `Entity.spawnAtLocation(level, stack, yOffset)`.
pub fn spawn_at_location(level: &mut Level, at: Vec3, stack: Stack) {
    if stack.is_empty() {
        return;
    }
    let mut e = new_tossed(level, at, stack);
    item(&mut e).pickup_delay = 10;
    level.add_entity(e);
}

/// `Block.popResource`: a drop from a block, somewhere near its centre.
pub fn pop_resource(level: &mut Level, block: crate::pos::Pos, stack: Stack) {
    let half = (0.25f32 / 2.0) as f64;
    let x = block.x as f64 + 0.5 + next_double_between(level, -0.25, 0.25);
    let y = block.y as f64 + 0.5 + next_double_between(level, -0.25, 0.25) - half;
    let z = block.z as f64 + 0.5 + next_double_between(level, -0.25, 0.25);
    spawn_at_location(level, Vec3::new(x, y, z), stack);
}

/// `Mth.nextDouble(random, min, max)`.
fn next_double_between(level: &mut Level, min: f64, max: f64) -> f64 {
    if min >= max { min } else { level.random.next_double() * (max - min) + min }
}

/// `ItemEntity.tick`.
pub fn tick(level: &mut Level, e: &mut Entity) {
    if item(e).stack.is_empty() {
        e.discard();
        return;
    }
    super::base_tick(level, e);
    {
        let i = item(e);
        if i.pickup_delay > 0 && i.pickup_delay != INFINITE_PICKUP_DELAY {
            i.pickup_delay -= 1;
        }
    }
    let old = e.pos;
    if e.was_touching_water && e.fluid_height > 0.1f32 as f64 {
        // `setUnderwaterMovement`.
        let m = e.motion;
        let f = 0.99f32 as f64;
        let lift = if m.y < 0.06f32 as f64 { 5.0E-4f32 as f64 } else { 0.0 };
        e.motion = Vec3::new(m.x * f, m.y + lift, m.z * f);
    } else {
        apply_gravity(e, 0.04);
    }
    // An item inside a block heads out of it, through blocks, this tick.
    e.no_physics = !no_block_collision(level, e.bounding_box().deflate(1.0E-7));
    if e.no_physics {
        let b = e.bounding_box();
        super::move_towards_closest_space(level, e, e.pos.x, (b.min[1] + b.max[1]) / 2.0, e.pos.z);
    }
    if !e.on_ground || e.motion.horizontal_distance_sqr() > 1.0E-5f32 as f64 || (e.tick_count + e.id) % 4 == 0 {
        move_entity(level, e, e.motion);
        super::apply_effects_from_blocks(level, e);
        let mut f = 0.98f32;
        if e.on_ground {
            let below = level.get(e.pos_below_affecting_movement(level));
            f = level.physics().factors(level.block(below)).friction * 0.98;
        }
        e.motion = e.motion.multiply(f as f64, DRAG, f as f64);
        if e.on_ground && e.motion.y < 0.0 {
            e.motion = e.motion.multiply(1.0, -0.5, 1.0);
        }
    } else {
        // A resting item feels the blocks of its last moves again, and
        // the block it stands on (`applyEffectsFromBlocksForLastMovements`).
        super::apply_effects_for_last_movements(level, e);
    }
    let moved_block = super::floor(old.x) != super::floor(e.pos.x)
        || super::floor(old.y) != super::floor(e.pos.y)
        || super::floor(old.z) != super::floor(e.pos.z);
    let every = if moved_block { 2 } else { 40 };
    if e.tick_count % every == 0 && item(e).mergable() {
        merge_with_neighbours(level, e);
    }
    let i = item(e);
    if i.age != INFINITE_LIFETIME {
        i.age += 1;
    }
    // Water pushes items a second time each tick, after they move.
    super::update_in_water(level, e);
    if item(e).age >= LIFETIME {
        e.discard();
    }
}

/// `ItemEntity.mergeWithNeighbours`.
fn merge_with_neighbours(level: &mut Level, e: &mut Entity) {
    let b = e.bounding_box().inflate(0.5, 0.0, 0.5);
    let mut others: Vec<usize> = (0..level.entities.len())
        .filter(|&k| {
            let o = &level.entities[k];
            !o.removed && matches!(&o.data, Data::Item(i) if i.mergable()) && o.bounding_box().intersects(&b)
        })
        .collect();
    // `getEntitiesOfClass` walks entity sections; within one section, in
    // the order entities were added.
    others.sort_by_key(|&k| level.entities[k].id);
    for k in others {
        let o = &mut level.entities[k];
        let Data::Item(oi) = &mut o.data else { continue };
        if !oi.mergable() {
            continue;
        }
        try_to_merge(e, o);
        if e.removed {
            break;
        }
    }
}

/// `ItemEntity.tryToMerge`.
fn try_to_merge(e: &mut Entity, o: &mut Entity) {
    let (a, b) = (item(e).stack.clone(), item(o).stack.clone());
    if a.id != b.id || a.count + b.count > crate::items::max_stack_size(&b.id) {
        return;
    }
    if b.count < a.count { merge(e, o) } else { merge(o, e) }
}

/// `ItemEntity.merge(dest, destStack, src, srcStack)`.
fn merge(dest: &mut Entity, src: &mut Entity) {
    let (d, s) = (item(dest).clone(), item(src).clone());
    let max = crate::items::max_stack_size(&d.stack.id).min(64);
    let moved = (max - d.stack.count).min(s.stack.count);
    let di = item(dest);
    di.stack.count += moved;
    di.pickup_delay = di.pickup_delay.max(s.pickup_delay);
    di.age = di.age.min(s.age);
    let si = item(src);
    si.stack.count -= moved;
    if si.stack.is_empty() {
        src.discard();
    }
}
