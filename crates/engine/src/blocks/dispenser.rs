//! Dispensers and droppers: `DispenserBlock.dispenseFrom`, the slot it
//! picks, and the dispense behaviours that change blocks.
//!
//! Buckets place and pick up water and powder snow. Every other item takes
//! the default behaviour and flies out as an item entity, in levels that
//! simulate entities; elsewhere only its slot emptying shows. Items whose
//! own behaviour changes the world (bone meal, shulker boxes, TNT and so on)
//! are refused by [`supported_item`] rather than run wrong.

use crate::entity::item;
use crate::game_event::GameEvent;
use crate::level::{Level, MAX_RECURSION, flags};
use crate::physics::Vec3;
use crate::pos::{Dir, Pos};
use crate::registry::StateId;

use super::Kind;
use super::container::{self, Stack};
use super::water;

/// Whether the engine knows what a dispenser does with `id`.
pub fn supported_item(id: &str) -> bool {
    let name = id.strip_prefix("minecraft:").unwrap_or(id);
    if matches!(name, "water_bucket" | "powder_snow_bucket" | "bucket") {
        return true;
    }
    // Items with a dispense behaviour of their own, or an equippable or
    // projectile default, which the engine does not simulate yet.
    let special_suffix = [
        "_bucket",
        "_boat",
        "_raft",
        "_minecart",
        "_spawn_egg",
        "shulker_box",
        "_helmet",
        "_chestplate",
        "_leggings",
        "_boots",
        "_horse_armor",
        "_carpet",
        "_head",
        "_skull",
        "_arrow",
        "_harness",
        "_chest_boat",
    ];
    let special = [
        "minecart",
        "arrow",
        "tnt",
        "bone_meal",
        "flint_and_steel",
        "fire_charge",
        "shears",
        "glass_bottle",
        "honeycomb",
        "carved_pumpkin",
        "elytra",
        "saddle",
        "armor_stand",
        "egg",
        "snowball",
        "splash_potion",
        "lingering_potion",
        "experience_bottle",
        "potion",
        "firework_rocket",
        "trident",
        "wind_charge",
        "brush",
        "chest",
        "wolf_armor",
        "respawn_anchor",
        "glowstone",
        "turtle_helmet",
        "shield",
    ];
    !special.contains(&name) && !special_suffix.iter().any(|s| name.ends_with(s))
}

/// `DispenserBlock.tick`: `dispenseFrom`.
pub fn tick(level: &mut Level, pos: Pos, state: StateId) {
    if container::items(level, pos).is_none() {
        return;
    }
    let Some(slot) = random_slot(level, pos) else {
        // `levelEvent(1001)`, the click, is only a sound.
        level.game_event(GameEvent::BlockActivate, pos);
        return;
    };
    let stack = container::get_item(level, pos, slot);
    // A hopper can bring in an item whose behaviour the engine lacks.
    if !supported_item(&stack.id) {
        level.unsupported(format!("dispensing {}", stack.id));
        return;
    }
    let name = stack.id.strip_prefix("minecraft:").unwrap_or(&stack.id).to_string();
    let result = match name.as_str() {
        "water_bucket" | "powder_snow_bucket" => dispense_full_bucket(level, pos, state, slot, stack),
        "bucket" => dispense_empty_bucket(level, pos, state, slot, stack),
        _ => eject(level, pos, state, stack),
    };
    container::set_item(level, pos, slot, result);
}

/// `DispenserBlockEntity.getRandomSlot`: one draw from the level's random
/// source for each filled slot, `nextInt(1)`, `nextInt(2)` and so on; the
/// last filled slot to draw 0 is the one used. `None` if all are empty.
pub(super) fn random_slot(level: &mut Level, pos: Pos) -> Option<usize> {
    let filled: Vec<usize> =
        container::items(level, pos)?.iter().enumerate().filter(|(_, s)| !s.is_empty()).map(|(i, _)| i).collect();
    let mut slot = None;
    for (odds, i) in (1..).zip(filled) {
        if level.random.next_int_bounded(odds) == 0 {
            slot = Some(i);
        }
    }
    slot
}

/// `DispenserBlock.getDispensePosition`: 0.7 of a block out of the face
/// the block points at, from its middle.
pub(super) fn dispense_position(pos: Pos, facing: Dir) -> Vec3 {
    let [sx, sy, sz] = facing.step();
    let centre = Vec3::new(pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5);
    Vec3::new(centre.x + 0.7 * sx as f64, centre.y + 0.7 * sy as f64, centre.z + 0.7 * sz as f64)
}

/// `DefaultDispenseItemBehavior.spawnItem`, with the default accuracy of
/// 6: the stack flies out of `at` toward `facing`, its speed and spread
/// drawn from the level's random source after the item's own random
/// source has given it a toss that this replaces. Only levels that
/// simulate entities make the item; elsewhere it is gone.
pub(super) fn spawn_item(level: &mut Level, stack: Stack, facing: Dir, at: Vec3) {
    if !level.simulate_entities || stack.is_empty() {
        return;
    }
    let y = at.y - if facing.axis() == 1 { 0.125 } else { 0.15625 };
    let mut e = item::new_tossed(level, Vec3::new(at.x, y, at.z), stack);
    let [sx, _, sz] = facing.step();
    let spread = 0.0172275 * 6.0;
    let r = &mut level.random;
    let pow = r.next_double() * 0.1 + 0.2;
    let dx = r.triangle(sx as f64 * pow, spread);
    let dy = r.triangle(0.2, spread);
    let dz = r.triangle(sz as f64 * pow, spread);
    e.motion = Vec3::new(dx, dy, dz);
    level.add_entity(e);
}

/// `DefaultDispenseItemBehavior.execute`: one item flies out. The sound
/// and particles that follow change nothing.
pub(super) fn eject(level: &mut Level, pos: Pos, state: StateId, mut stack: Stack) -> Stack {
    let facing = level.dir(state, "facing");
    let one = stack.split(1);
    spawn_item(level, one, facing, dispense_position(pos, facing));
    stack
}

/// The behaviour for `DispensibleContainerItem`s: empty the bucket in front,
/// or eject it if that fails.
fn dispense_full_bucket(level: &mut Level, pos: Pos, state: StateId, slot: usize, stack: Stack) -> Stack {
    let target = pos.offset(level.dir(state, "facing"));
    let emptied = if stack.id.ends_with("powder_snow_bucket") {
        place_powder_snow(level, target)
    } else {
        empty_water(level, target)
    };
    if emptied {
        consume_with_remainder(level, pos, state, slot, stack, Stack::new("minecraft:bucket", 1))
    } else {
        eject(level, pos, state, stack)
    }
}

/// `SolidBucketItem.emptyContents`.
fn place_powder_snow(level: &mut Level, pos: Pos) -> bool {
    if !level.is_air(level.get(pos)) {
        return false;
    }
    let snow = level.default_state(level.block_id("powder_snow"));
    level.set_block(pos, snow, flags::ALL);
    level.game_event(GameEvent::FluidPlace, pos);
    true
}

/// `BucketItem.emptyContents` for water, with the `FLUID_PLACE` game event
/// of `playEmptySound` when it works.
fn empty_water(level: &mut Level, pos: Pos) -> bool {
    let placed = place_water(level, pos);
    if placed {
        level.game_event(GameEvent::FluidPlace, pos);
    }
    placed
}

fn place_water(level: &mut Level, pos: Pos) -> bool {
    let state = level.get(pos);
    let liquid = level.kind(state) == Kind::Water;
    let replaceable = level.is_air(state) || liquid || !water::is_solid(level, state);
    let container = water::is_container(level, state);
    let can_place = container && !level.flag(state, "waterlogged");
    if !(level.is_air(state) || replaceable || can_place) {
        return false;
    }
    if container {
        water::place_liquid(level, pos, state, water::SOURCE);
        return true;
    }
    if replaceable && !liquid {
        level.destroy_block(pos, true, MAX_RECURSION);
    }
    let source = water::legacy_block(level, water::SOURCE);
    level.set_block(pos, source, flags::ALL_IMMEDIATE) || water::fluid_state(level, state).is_source()
}

/// The empty bucket's behaviour: pick up what is in front, or eject.
fn dispense_empty_bucket(level: &mut Level, pos: Pos, state: StateId, slot: usize, stack: Stack) -> Stack {
    let target = pos.offset(level.dir(state, "facing"));
    match pickup_block(level, target) {
        Some(filled) => {
            level.game_event(GameEvent::FluidPickup, target);
            consume_with_remainder(level, pos, state, slot, stack, Stack::new(filled, 1))
        }
        None => eject(level, pos, state, stack),
    }
}

/// `BucketPickup.pickupBlock`: the bucket it fills, if any.
fn pickup_block(level: &mut Level, pos: Pos) -> Option<&'static str> {
    let state = level.get(pos);
    match level.kind(state) {
        Kind::Water if level.int(state, "level") == 0 => {
            level.set_block(pos, level.air, flags::ALL_IMMEDIATE);
            Some("minecraft:water_bucket")
        }
        Kind::PowderSnow => {
            level.set_block(pos, level.air, flags::ALL_IMMEDIATE);
            Some("minecraft:powder_snow_bucket")
        }
        _ if water::is_container(level, state) && level.flag(state, "waterlogged") => {
            level.set_block(pos, level.with_bool(state, "waterlogged", false), flags::ALL);
            Some("minecraft:water_bucket")
        }
        _ => None,
    }
}

/// `DefaultDispenseItemBehavior.consumeWithRemainder`: use one item; the
/// remainder takes the slot if it emptied, or goes into the dispenser, and
/// what does not fit flies out (`addToInventoryOrDispense`).
fn consume_with_remainder(
    level: &mut Level,
    pos: Pos,
    state: StateId,
    slot: usize,
    mut stack: Stack,
    remainder: Stack,
) -> Stack {
    stack.split(1);
    if stack.is_empty() {
        return remainder;
    }
    // The slot is written back after this, so the shrunk stack goes in first.
    container::set_count(level, pos, slot, stack.count);
    let rest = insert_item(level, pos, remainder);
    if !rest.is_empty() {
        let facing = level.dir(state, "facing");
        spawn_item(level, rest, facing, dispense_position(pos, facing));
    }
    stack
}

/// `DispenserBlockEntity.insertItem`: what does not fit is returned.
fn insert_item(level: &mut Level, pos: Pos, mut stack: Stack) -> Stack {
    let max = stack.max_stack_size();
    for i in 0..container::size(level, pos) {
        let cur = container::get_item(level, pos, i);
        if cur.is_empty() || cur.same_item(&stack) {
            let n = stack.count.min(max - cur.count);
            if n > 0 {
                if cur.is_empty() {
                    let part = stack.split(n);
                    container::set_item(level, pos, i, part);
                } else {
                    stack.count -= n;
                    container::set_count(level, pos, i, cur.count + n);
                }
            }
            if stack.is_empty() {
                break;
            }
        }
    }
    stack
}
