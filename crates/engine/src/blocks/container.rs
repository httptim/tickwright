//! Containers: chests, hoppers and droppers, and the items they move.
//!
//! Follows `HopperBlockEntity`, `DropperBlock` and the container block
//! entities they work on. Only container blocks take part: item entities,
//! minecarts and composters are not simulated yet, and a double chest acts
//! as two single chests.
//!
//! `BlockEntity.setChanged` is what the trace records for items, with the
//! container's contents at that moment. Chest and dropper `setItem` and
//! `removeItem` call it themselves; the hopper's versions do not, which is
//! why a hopper marks itself changed once per transfer.

use crate::level::{Level, flags};
use crate::pos::Pos;
use crate::registry::StateId;

use super::{BlockEntity, Kind};

/// `ItemStack`: `count` of item `id`. A count of zero is `ItemStack.EMPTY`.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Stack {
    pub id: String,
    pub count: i32,
}

impl Stack {
    pub fn new(id: &str, count: i32) -> Stack {
        Stack { id: id.to_string(), count }
    }

    pub fn is_empty(&self) -> bool {
        self.count <= 0
    }

    /// `ItemStack.split`.
    pub(super) fn split(&mut self, amount: i32) -> Stack {
        let n = amount.min(self.count);
        self.count -= n;
        Stack { id: self.id.clone(), count: n }
    }

    pub(super) fn max_stack_size(&self) -> i32 {
        if self.is_empty() { 64 } else { max_stack_size(&self.id) }
    }
}

/// `Item.getDefaultMaxStackSize` for the items fixtures are likely to use.
pub fn max_stack_size(id: &str) -> i32 {
    let name = id.strip_prefix("minecraft:").unwrap_or(id);
    let one = ["_sword", "_pickaxe", "_axe", "_shovel", "_hoe", "_helmet", "_chestplate", "_leggings", "_boots"];
    if one.iter().any(|s| name.ends_with(s))
        || name.ends_with("_bucket")
        || matches!(name, "potion" | "splash_potion" | "lingering_potion" | "minecart" | "saddle" | "shears" | "bow")
    {
        1
    } else if name.ends_with("_sign")
        || name.ends_with("_banner")
        || matches!(name, "ender_pearl" | "snowball" | "egg" | "bucket" | "honey_bottle" | "armor_stand")
    {
        16
    } else {
        64
    }
}

/// A hopper's block entity: its items and transfer cooldown.
#[derive(Clone, Debug)]
pub struct Hopper {
    pub items: Vec<Stack>,
    cooldown: i32,
    ticked_game_time: i64,
}

impl Hopper {
    pub fn new() -> Hopper {
        Hopper { items: vec![Stack::default(); 5], cooldown: -1, ticked_game_time: 0 }
    }

    fn on_cooldown(&self) -> bool {
        self.cooldown > 0
    }

    /// `isOnCustomCooldown`.
    fn on_custom_cooldown(&self) -> bool {
        self.cooldown > 8
    }
}

impl Default for Hopper {
    fn default() -> Self {
        Hopper::new()
    }
}

pub fn new_chest() -> BlockEntity {
    BlockEntity::Chest(vec![Stack::default(); 27])
}

pub fn new_dropper() -> BlockEntity {
    BlockEntity::Dropper(vec![Stack::default(); 9])
}

// --- container access ----------------------------------------------------------

pub fn items(level: &Level, pos: Pos) -> Option<&Vec<Stack>> {
    match level.block_entities.get(&pos)? {
        BlockEntity::Chest(items) | BlockEntity::Dropper(items) | BlockEntity::ShulkerBox(items) => Some(items),
        BlockEntity::Hopper(h) => Some(&h.items),
        _ => None,
    }
}

fn items_mut(level: &mut Level, pos: Pos) -> &mut Vec<Stack> {
    match level.block_entities.get_mut(&pos) {
        Some(BlockEntity::Chest(items) | BlockEntity::Dropper(items) | BlockEntity::ShulkerBox(items)) => items,
        Some(BlockEntity::Hopper(h)) => &mut h.items,
        _ => panic!("no container at {pos:?}"),
    }
}

fn is_hopper(level: &Level, pos: Pos) -> bool {
    matches!(level.block_entities.get(&pos), Some(BlockEntity::Hopper(_)))
}

fn hopper(level: &Level, pos: Pos) -> &Hopper {
    match level.block_entities.get(&pos) {
        Some(BlockEntity::Hopper(h)) => h,
        _ => panic!("no hopper at {pos:?}"),
    }
}

fn hopper_mut(level: &mut Level, pos: Pos) -> &mut Hopper {
    match level.block_entities.get_mut(&pos) {
        Some(BlockEntity::Hopper(h)) => h,
        _ => panic!("no hopper at {pos:?}"),
    }
}

pub fn size(level: &Level, pos: Pos) -> usize {
    items(level, pos).map_or(0, Vec::len)
}

pub(super) fn get_item(level: &Level, pos: Pos, slot: usize) -> Stack {
    items(level, pos).map(|i| i[slot].clone()).unwrap_or_default()
}

fn is_empty(level: &Level, pos: Pos) -> bool {
    items(level, pos).is_none_or(|i| i.iter().all(Stack::is_empty))
}

/// `Container.setItem`.
pub fn set_item(level: &mut Level, pos: Pos, slot: usize, mut stack: Stack) {
    let max = stack.max_stack_size();
    stack.count = stack.count.min(max);
    if stack.is_empty() {
        stack = Stack::default();
    }
    items_mut(level, pos)[slot] = stack;
    if !is_hopper(level, pos) {
        set_changed(level, pos);
    }
}

/// `Container.removeItem` (`ContainerHelper.removeItem`).
fn remove_item(level: &mut Level, pos: Pos, slot: usize, amount: i32) -> Stack {
    let stack = {
        let s = &mut items_mut(level, pos)[slot];
        if s.is_empty() || amount <= 0 { Stack::default() } else { s.split(amount) }
    };
    if !stack.is_empty() && !is_hopper(level, pos) {
        set_changed(level, pos);
    }
    stack
}

pub(super) fn set_count(level: &mut Level, pos: Pos, slot: usize, count: i32) {
    items_mut(level, pos)[slot].count = count;
}

/// The contents as the trace writes them: `slot:id*count`, space separated.
pub fn describe(items: &[Stack]) -> String {
    items
        .iter()
        .enumerate()
        .filter(|(_, s)| !s.is_empty())
        .map(|(i, s)| format!("{i}:{}*{}", s.id, s.count))
        .collect::<Vec<_>>()
        .join(" ")
}

/// `BlockEntity.setChanged`: record the contents, then tell comparators.
pub fn set_changed(level: &mut Level, pos: Pos) {
    if let Some(items) = items(level, pos) {
        let detail = describe(items);
        level.record_items(pos, detail);
    }
    set_changed_static(level, pos);
}

/// The static `BlockEntity.setChanged(level, pos, state)`.
fn set_changed_static(level: &mut Level, pos: Pos) {
    let state = level.get(pos);
    if !level.is_air(state) {
        level.update_neighbour_for_output_signal(pos, level.block(state));
    }
}

/// `AbstractContainerMenu.getRedstoneSignalFromContainer`.
fn signal_from_items(items: &[Stack]) -> i32 {
    let mut f = 0.0f32;
    for s in items.iter().filter(|s| !s.is_empty()) {
        f += s.count as f32 / s.max_stack_size().min(99) as f32;
    }
    f /= items.len() as f32;
    (f * 14.0).floor() as i32 + if f > 0.0 { 1 } else { 0 }
}

/// `getAnalogOutputSignal` for container blocks. A chest under a conductor
/// is blocked and reads as empty.
pub fn analog_output(level: &Level, pos: Pos, state: StateId) -> i32 {
    if level.kind(state) == Kind::Chest {
        let above = pos.above();
        if super::is_conductor(level, above, level.get(above)) {
            return 0;
        }
    }
    items(level, pos).map_or(0, |i| signal_from_items(i))
}

/// `HopperBlockEntity.getContainerAt` for blocks: a container block entity
/// at `pos`, if the block there is a container.
fn container_at(level: &Level, pos: Pos) -> Option<Pos> {
    let kind = level.kind_at(pos);
    let ok =
        matches!(kind, Kind::Chest | Kind::Barrel | Kind::Hopper | Kind::Dropper | Kind::Dispenser | Kind::ShulkerBox);
    (ok && items(level, pos).is_some()).then_some(pos)
}

/// `isFullContainer`.
fn is_full_container(level: &Level, pos: Pos) -> bool {
    items(level, pos).is_some_and(|i| i.iter().all(|s| s.count >= s.max_stack_size()))
}

// --- moving items ----------------------------------------------------------------

/// `HopperBlockEntity.addItem(source, destination, stack, direction)`.
fn add_item(level: &mut Level, source: Option<Pos>, dest: Pos, mut stack: Stack) -> Stack {
    let n = size(level, dest);
    let mut slot = 0;
    while slot < n && !stack.is_empty() {
        stack = try_move_in_item(level, source, dest, stack, slot);
        slot += 1;
    }
    stack
}

/// `HopperBlockEntity.tryMoveInItem`.
fn try_move_in_item(level: &mut Level, source: Option<Pos>, dest: Pos, mut stack: Stack, slot: usize) -> Stack {
    let current = get_item(level, dest, slot);
    let mut changed = false;
    let was_empty = is_empty(level, dest);
    if current.is_empty() {
        set_item(level, dest, slot, stack);
        stack = Stack::default();
        changed = true;
    } else if current.id == stack.id && current.count < current.max_stack_size() {
        let n = stack.count.min(stack.max_stack_size() - current.count);
        stack.count -= n;
        set_count(level, dest, slot, current.count + n);
        changed = n > 0;
    }
    if changed {
        if was_empty && is_hopper(level, dest) && !hopper(level, dest).on_custom_cooldown() {
            let mut skip = 0;
            if let Some(src) = source
                && is_hopper(level, src)
                && hopper(level, dest).ticked_game_time >= hopper(level, src).ticked_game_time
            {
                skip = 1;
            }
            hopper_mut(level, dest).cooldown = 8 - skip;
        }
        set_changed(level, dest);
    }
    stack
}

// --- HopperBlockEntity -------------------------------------------------------------

/// `pushItemsTick`, the hopper's block entity ticker.
pub fn push_items_tick(level: &mut Level, pos: Pos) {
    let now = level.game_time;
    let h = hopper_mut(level, pos);
    h.cooldown -= 1;
    h.ticked_game_time = now;
    if !h.on_cooldown() {
        h.cooldown = 0;
        try_move_items(level, pos);
    }
}

/// `tryMoveItems`.
fn try_move_items(level: &mut Level, pos: Pos) -> bool {
    let state = level.get(pos);
    if hopper(level, pos).on_cooldown() || !level.flag(state, "enabled") {
        return false;
    }
    let mut changed = false;
    if !is_empty(level, pos) {
        changed = eject_items(level, pos, state);
    }
    if !is_full_container(level, pos) {
        changed |= suck_in_items(level, pos);
    }
    if changed {
        hopper_mut(level, pos).cooldown = 8;
        set_changed_static(level, pos);
    }
    changed
}

/// `ejectItems`: one item into the container the hopper faces.
fn eject_items(level: &mut Level, pos: Pos, state: StateId) -> bool {
    let facing = level.dir(state, "facing");
    let Some(dest) = container_at(level, pos.offset(facing)) else {
        return false;
    };
    if is_full_container(level, dest) {
        return false;
    }
    for slot in 0..size(level, pos) {
        let stack = get_item(level, pos, slot);
        if stack.is_empty() {
            continue;
        }
        let count = stack.count;
        let moving = remove_item(level, pos, slot, 1);
        let result = add_item(level, Some(pos), dest, moving);
        if result.is_empty() {
            set_changed(level, dest);
            return true;
        }
        restore(level, pos, slot, stack, count);
    }
    false
}

/// Put back a stack whose item did not fit: `stack.setCount(count)`, and
/// `setItem` if removing emptied the slot.
fn restore(level: &mut Level, pos: Pos, slot: usize, mut stack: Stack, count: i32) {
    if count == 1 {
        stack.count = count;
        set_item(level, pos, slot, stack);
    } else {
        set_count(level, pos, slot, count);
    }
}

/// `suckInItems`: one item from the container above.
fn suck_in_items(level: &mut Level, pos: Pos) -> bool {
    let Some(source) = container_at(level, pos.above()) else {
        return false;
    };
    for slot in 0..size(level, source) {
        let stack = get_item(level, source, slot);
        if stack.is_empty() {
            continue;
        }
        let count = stack.count;
        let moving = remove_item(level, source, slot, 1);
        let result = add_item(level, Some(source), pos, moving);
        if result.is_empty() {
            set_changed(level, source);
            return true;
        }
        restore(level, source, slot, stack, count);
    }
    false
}

// --- HopperBlock -------------------------------------------------------------------

/// `checkPoweredState`: a powered hopper is locked.
fn check_powered_state(level: &mut Level, pos: Pos, state: StateId) {
    let enable = !level.has_neighbor_signal(pos);
    if enable != level.flag(state, "enabled") {
        level.set_block(pos, level.with_bool(state, "enabled", enable), flags::CLIENTS);
    }
}

pub fn hopper_on_place(level: &mut Level, pos: Pos, state: StateId, old: StateId) {
    if level.block(old) != level.block(state) {
        check_powered_state(level, pos, state);
    }
}

pub fn hopper_neighbor_changed(level: &mut Level, pos: Pos, state: StateId) {
    check_powered_state(level, pos, state);
}

// --- DropperBlock ------------------------------------------------------------------

pub fn dropper_neighbor_changed(level: &mut Level, pos: Pos, state: StateId) {
    let powered = level.has_neighbor_signal(pos) || level.has_neighbor_signal(pos.above());
    let triggered = level.flag(state, "triggered");
    if powered && !triggered {
        level.schedule_tick(pos, level.block(state), 4, 0);
        level.set_block(pos, level.with_bool(state, "triggered", true), flags::CLIENTS);
    } else if !powered && triggered {
        level.set_block(pos, level.with_bool(state, "triggered", false), flags::CLIENTS);
    }
}

/// `DropperBlock.dispenseFrom`. With several filled slots the game picks
/// one at random; this takes the first, so fixtures fill only one.
pub fn dropper_tick(level: &mut Level, pos: Pos, state: StateId) {
    let Some(slot) = items(level, pos).and_then(|i| i.iter().position(|s| !s.is_empty())) else {
        return;
    };
    let stack = get_item(level, pos, slot);
    let facing = level.dir(state, "facing");
    let remaining = match container_at(level, pos.offset(facing)) {
        // Dropped as an item entity, which is not simulated.
        None => Stack { count: stack.count - 1, ..stack.clone() },
        Some(dest) => {
            let result = add_item(level, Some(pos), dest, Stack { count: 1, ..stack.clone() });
            if result.is_empty() { Stack { count: stack.count - 1, ..stack.clone() } } else { stack.clone() }
        }
    };
    set_item(level, pos, slot, remaining);
}
