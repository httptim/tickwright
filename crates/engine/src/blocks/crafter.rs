//! `CrafterBlock` and `CrafterBlockEntity`.
//!
//! A rising edge schedules a tick 4 game ticks later. The tick looks the
//! 3x3 grid up in the game's crafting recipes; on a match it lights up for 6
//! ticks, puts the result and then any leftovers (empty buckets) into the
//! container it faces, and uses up one item from every slot. What finds no
//! room flies out as an item entity, in levels that simulate entities.
//! Disabled slots are not modelled: every slot accepts items.

use crate::items::{self, CraftingInput};
use crate::level::{Level, flags};
use crate::pos::{Dir, Pos};
use crate::registry::StateId;

use super::BlockEntity;
use super::container::{self, Stack};

#[derive(Clone, Debug)]
pub struct Crafter {
    pub items: Vec<Stack>,
    crafting_ticks_remaining: i32,
}

impl Crafter {
    pub fn new() -> Crafter {
        Crafter { items: vec![Stack::default(); 9], crafting_ticks_remaining: 0 }
    }
}

impl Default for Crafter {
    fn default() -> Self {
        Crafter::new()
    }
}

fn crafter(level: &Level, pos: Pos) -> Option<&Crafter> {
    match level.block_entities.get(&pos) {
        Some(BlockEntity::Crafter(c)) => Some(c),
        _ => None,
    }
}

fn crafter_mut(level: &mut Level, pos: Pos) -> Option<&mut Crafter> {
    match level.block_entities.get_mut(&pos) {
        Some(BlockEntity::Crafter(c)) => Some(c),
        _ => None,
    }
}

/// The direction the crafter faces: the first half of `orientation`.
fn front(level: &Level, state: StateId) -> Dir {
    let o = level.prop(state, "orientation");
    Dir::from_name(o.split('_').next().unwrap_or("north")).unwrap_or(Dir::North)
}

/// `canPlaceItem`: a slot takes an item unless it is full or a later slot
/// holds a smaller stack of the same item (or nothing), so items spread out.
pub fn can_place_item(level: &Level, pos: Pos, slot: usize) -> bool {
    let Some(c) = crafter(level, pos) else { return false };
    let current = &c.items[slot];
    if current.count >= current.max_stack_size() {
        return false;
    }
    if current.is_empty() {
        return true;
    }
    // `smallerStackExist`, against the stack already in the slot.
    let smaller = c.items[slot + 1..].iter().any(|s| s.is_empty() || (s.count < current.count && s.same_item(current)));
    !smaller
}

/// `getRedstoneSignal`: how many slots hold something.
pub fn analog_output(level: &Level, pos: Pos) -> i32 {
    crafter(level, pos).map_or(0, |c| c.items.iter().filter(|s| !s.is_empty()).count() as i32)
}

pub fn neighbor_changed(level: &mut Level, pos: Pos, state: StateId) {
    let powered = level.has_neighbor_signal(pos);
    let triggered = level.flag(state, "triggered");
    if powered && !triggered {
        level.schedule_tick(pos, level.block(state), 4, 0);
        level.set_block(pos, level.with_bool(state, "triggered", true), flags::CLIENTS);
    } else if !powered && triggered {
        // A falling edge also puts out the crafting light at once.
        let off = level.with_bool(level.with_bool(state, "triggered", false), "crafting", false);
        level.set_block(pos, off, flags::CLIENTS);
    }
}

/// `tick`, which is `dispenseFrom`.
pub fn tick(level: &mut Level, pos: Pos, state: StateId) {
    let Some(c) = crafter(level, pos) else { return };
    let grid: Vec<Option<String>> = c.items.iter().map(|s| (!s.is_empty()).then(|| s.id.clone())).collect();
    let input = CraftingInput::of(3, 3, &grid);
    // No match is only a sound and particles.
    let Some(recipe) = items::find_recipe(&input) else { return };
    let (id, count) = recipe.result();
    let remainders: Vec<Stack> =
        grid.iter().flatten().filter_map(|id| items::crafting_remainder(id)).map(|r| Stack::new(r, 1)).collect();
    crafter_mut(level, pos).unwrap().crafting_ticks_remaining = 6;
    level.set_block(pos, level.with_bool(state, "crafting", true), flags::CLIENTS);
    dispense_item(level, pos, state, Stack::new(id, count));
    for r in remainders {
        dispense_item(level, pos, state, r);
    }
    for s in crafter_mut(level, pos).unwrap().items.iter_mut() {
        if !s.is_empty() {
            s.count -= 1;
            if s.is_empty() {
                *s = Stack::default();
            }
        }
    }
    container::set_changed(level, pos);
}

/// `dispenseItem`: into the container in front, one at a time into another
/// crafter or when the stack is too big for the container, whole otherwise;
/// what is left flies out as an item.
fn dispense_item(level: &mut Level, pos: Pos, state: StateId, stack: Stack) {
    let facing = front(level, state);
    let mut stack = stack;
    if let Some(dest) = container::container_at(level, pos.offset(facing)) {
        let one_by_one = matches!(dest, container::Inv::Block(d) if level.kind_at(d) == super::Kind::Crafter)
            || stack.count > stack.max_stack_size().min(99);
        if one_by_one {
            while !stack.is_empty() {
                // `copyWithCount(1)`, components and all.
                let left = container::add_item(level, Some(pos.into()), dest, Stack { count: 1, ..stack.clone() });
                if !left.is_empty() {
                    break;
                }
                stack.count -= 1;
            }
        } else {
            while !stack.is_empty() {
                let n = stack.count;
                stack = container::add_item(level, Some(pos.into()), dest, stack);
                if stack.count == n {
                    break;
                }
            }
        }
    }
    // `spawnItem` from 0.7 of a block out of the front face; the sound and
    // the recipe advancement change nothing.
    super::dispenser::spawn_item(level, stack, facing, super::dispenser::dispense_position(pos, facing));
}

/// `serverTick`: the crafting light goes out 6 ticks after a craft.
pub fn tick_entity(level: &mut Level, pos: Pos) {
    let Some(c) = crafter_mut(level, pos) else { return };
    let left = c.crafting_ticks_remaining - 1;
    if left >= 0 {
        c.crafting_ticks_remaining = left;
        if left == 0 {
            let state = level.get(pos);
            level.set_block(pos, level.with_bool(state, "crafting", false), flags::ALL);
        }
    }
}
