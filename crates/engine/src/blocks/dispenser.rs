//! Dispensers: `DispenserBlock.dispenseFrom` and the dispense behaviours
//! that change blocks.
//!
//! Buckets place and pick up water and powder snow. Every other item takes
//! the default behaviour and is shot out as an item entity, which is not
//! simulated, so only its slot emptying shows. Items whose own behaviour
//! changes the world (bone meal, shulker boxes, TNT and so on) are refused
//! by [`supported_item`] rather than run wrong.

use crate::level::{Level, MAX_RECURSION, flags};
use crate::pos::Pos;
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
    // `getRandomSlot` picks among the filled slots at random; fixtures fill
    // one, so the first filled slot is the one it would pick.
    let Some(slot) = container::items(level, pos).and_then(|i| i.iter().position(|s| !s.is_empty())) else {
        return;
    };
    let stack = container::get_item(level, pos, slot);
    let name = stack.id.strip_prefix("minecraft:").unwrap_or(&stack.id).to_string();
    let result = match name.as_str() {
        "water_bucket" | "powder_snow_bucket" => dispense_full_bucket(level, pos, state, slot, stack),
        "bucket" => dispense_empty_bucket(level, pos, state, slot, stack),
        _ => eject(stack),
    };
    container::set_item(level, pos, slot, result);
}

/// `DefaultDispenseItemBehavior.execute`: one item leaves as an entity.
fn eject(mut stack: Stack) -> Stack {
    stack.split(1);
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
        consume_with_remainder(level, pos, slot, stack, Stack::new("minecraft:bucket", 1))
    } else {
        eject(stack)
    }
}

/// `SolidBucketItem.emptyContents`.
fn place_powder_snow(level: &mut Level, pos: Pos) -> bool {
    if !level.is_air(level.get(pos)) {
        return false;
    }
    let snow = level.default_state(level.block_id("powder_snow"));
    level.set_block(pos, snow, flags::ALL);
    true
}

/// `BucketItem.emptyContents` for water.
fn empty_water(level: &mut Level, pos: Pos) -> bool {
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
        level.destroy_block(pos, MAX_RECURSION);
    }
    let source = water::legacy_block(level, water::SOURCE);
    level.set_block(pos, source, flags::ALL_IMMEDIATE) || water::fluid_state(level, state).is_source()
}

/// The empty bucket's behaviour: pick up what is in front, or eject.
fn dispense_empty_bucket(level: &mut Level, pos: Pos, state: StateId, slot: usize, stack: Stack) -> Stack {
    let target = pos.offset(level.dir(state, "facing"));
    match pickup_block(level, target) {
        Some(filled) => consume_with_remainder(level, pos, slot, stack, Stack::new(filled, 1)),
        None => eject(stack),
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
/// remainder takes the slot if it emptied, or goes into the dispenser.
fn consume_with_remainder(level: &mut Level, pos: Pos, slot: usize, mut stack: Stack, remainder: Stack) -> Stack {
    stack.split(1);
    if stack.is_empty() {
        return remainder;
    }
    // The slot is written back after this, so the shrunk stack goes in first.
    container::set_count(level, pos, slot, stack.count);
    insert_item(level, pos, remainder);
    stack
}

/// `DispenserBlockEntity.insertItem`; whatever does not fit is ejected.
fn insert_item(level: &mut Level, pos: Pos, mut stack: Stack) {
    let max = stack.max_stack_size();
    for i in 0..container::size(level, pos) {
        let cur = container::get_item(level, pos, i);
        if cur.is_empty() || cur.id == stack.id {
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
}
