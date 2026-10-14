//! Containers: chests, hoppers and droppers, and the items they move.
//!
//! Follows `HopperBlockEntity`, `DropperBlock` and the containers they
//! work on: container blocks, and chest and hopper minecarts where no
//! container block is. Composters are not simulated yet, and a double
//! chest acts as two single chests.
//!
//! `BlockEntity.setChanged` is what the trace records for items, with the
//! container's contents at that moment. Chest and dropper `setItem` and
//! `removeItem` call it themselves; the hopper's versions do not, which is
//! why a hopper marks itself changed once per transfer.

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::{Context, Result, bail};

use crate::entity::snbt;
use crate::level::{Level, flags};
use crate::physics::{Aabb, Vec3};
use crate::pos::Pos;
use crate::registry::StateId;

use super::{BlockEntity, Kind};
use crate::entity::{Data, floor};

/// A container: a container block's block entity, or a chest or hopper
/// minecart, by its place in the level's entity list.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Inv {
    Block(Pos),
    Cart(usize),
}

impl From<Pos> for Inv {
    fn from(p: Pos) -> Inv {
        Inv::Block(p)
    }
}

fn cart_items(level: &Level, k: usize) -> Option<&Vec<Stack>> {
    match &level.entities[k].data {
        Data::Minecart(m) => m.items.as_ref(),
        _ => None,
    }
}

/// `ItemStack`: `count` of item `id`. A count of zero is `ItemStack.EMPTY`.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Stack {
    pub id: String,
    pub count: i32,
    /// What the stack carries beyond its item: its component patch (a name,
    /// say), or `None` for a plain stack.
    pub data: Option<Arc<ItemData>>,
}

/// A stack's component patch (`DataComponentPatch`), and a pre-1.20.5
/// item `tag` a file may still hold. The engine models no component, but
/// it keeps them, so a save writes them back, and compares them, since
/// stacks only merge with stacks whose components are the same
/// (`ItemStack.isSameItemSameComponents`).
#[derive(Clone, Debug)]
pub struct ItemData {
    /// The patch as it was given, in SNBT.
    pub components: Option<String>,
    /// The legacy tag as it was given, in SNBT.
    pub tag: Option<String>,
    /// The data version of the file they came from, when older than the
    /// game's: they are in that version's format. It doesn't take part in
    /// comparing.
    pub data_version: Option<i32>,
    /// Both in canonical SNBT, which is what compares.
    key: String,
}

impl PartialEq for ItemData {
    fn eq(&self, other: &ItemData) -> bool {
        self.key == other.key
    }
}

impl Eq for ItemData {}

/// Components whose behaviour the engine simulates but doesn't model:
/// `max_stack_size` sets the stack size, and `damage_resistant` keeps an
/// item entity from burning or being blown up. Removing one from an item
/// (`!minecraft:max_stack_size`) counts too.
pub const REFUSED_COMPONENTS: [&str; 2] = ["minecraft:max_stack_size", "minecraft:damage_resistant"];

impl ItemData {
    /// The data for a component patch and a legacy tag, each in SNBT;
    /// `None` when there is neither (an empty patch is no patch).
    pub fn new(
        components: Option<&str>,
        tag: Option<&str>,
        data_version: Option<i32>,
    ) -> Result<Option<Arc<ItemData>>> {
        let patch = match components {
            Some(text) => {
                let t = snbt::parse(text).with_context(|| format!("item components {text}"))?;
                let snbt::Tag::Compound(m) = t else { bail!("item components {text} are not a compound") };
                // `stone` and `minecraft:stone` name the same component.
                let named: BTreeMap<String, snbt::Tag> = m.into_iter().map(|(k, v)| (component_name(&k), v)).collect();
                (!named.is_empty()).then_some(snbt::Tag::Compound(named))
            }
            None => None,
        };
        let legacy = match tag {
            Some(text) => Some(snbt::parse(text).with_context(|| format!("item tag {text}"))?),
            None => None,
        };
        if patch.is_none() && legacy.is_none() {
            return Ok(None);
        }
        let key = format!(
            "{}|{}",
            patch.as_ref().map(snbt::write).unwrap_or_default(),
            legacy.as_ref().map(snbt::write).unwrap_or_default()
        );
        Ok(Some(Arc::new(ItemData {
            components: patch.and(components.map(str::to_string)),
            tag: legacy.and(tag.map(str::to_string)),
            data_version,
            key,
        })))
    }

    /// The data for a component patch read from entity data.
    pub fn from_tags(
        components: Option<&snbt::Tag>,
        tag: Option<&snbt::Tag>,
        data_version: Option<i32>,
    ) -> Result<Option<Arc<ItemData>>> {
        let c = components.map(snbt::write);
        let t = tag.map(snbt::write);
        ItemData::new(c.as_deref(), t.as_deref(), data_version)
    }

    /// Whether the patch sets or removes `max_stack_size`.
    pub fn sets_stack_size(&self) -> bool {
        self.components.as_deref().and_then(|c| snbt::parse(c).ok()).is_some_and(|p| {
            p.keys().into_iter().any(|k| component_name(k).trim_start_matches('!') == REFUSED_COMPONENTS[0])
        })
    }

    /// The first component in the patch that changes behaviour the engine
    /// doesn't model.
    pub fn refused_component(&self) -> Option<&'static str> {
        let patch = snbt::parse(self.components.as_deref()?).ok()?;
        patch
            .keys()
            .into_iter()
            .map(component_name)
            .find_map(|k| REFUSED_COMPONENTS.into_iter().find(|r| k.trim_start_matches('!') == *r))
    }
}

/// A component's full name, `minecraft:` added, keeping a leading `!`.
fn component_name(k: &str) -> String {
    let (bang, name) = match k.strip_prefix('!') {
        Some(n) => ("!", n),
        None => ("", k),
    };
    if name.contains(':') { format!("{bang}{name}") } else { format!("{bang}minecraft:{name}") }
}

impl Stack {
    pub fn new(id: &str, count: i32) -> Stack {
        Stack { id: id.to_string(), count, data: None }
    }

    pub fn with_data(mut self, data: Option<Arc<ItemData>>) -> Stack {
        self.data = data;
        self
    }

    pub fn is_empty(&self) -> bool {
        self.count <= 0
    }

    /// `ItemStack.isSameItemSameComponents`.
    pub fn same_item(&self, other: &Stack) -> bool {
        fn bare(id: &str) -> &str {
            id.strip_prefix("minecraft:").unwrap_or(id)
        }
        bare(&self.id) == bare(&other.id) && self.data == other.data
    }

    /// `ItemStack.split`.
    pub(super) fn split(&mut self, amount: i32) -> Stack {
        let n = amount.min(self.count);
        self.count -= n;
        Stack { id: self.id.clone(), count: n, data: self.data.clone() }
    }

    pub(super) fn max_stack_size(&self) -> i32 {
        if self.is_empty() { 64 } else { max_stack_size(&self.id) }
    }
}

/// `Item.getDefaultMaxStackSize`, from the game's item data.
pub fn max_stack_size(id: &str) -> i32 {
    crate::items::max_stack_size(id)
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

pub fn items(level: &Level, inv: impl Into<Inv>) -> Option<&Vec<Stack>> {
    let pos = match inv.into() {
        Inv::Block(pos) => pos,
        Inv::Cart(k) => return cart_items(level, k),
    };
    match level.block_entities.get(&pos)? {
        BlockEntity::Chest(items) | BlockEntity::Dropper(items) | BlockEntity::ShulkerBox(items) => Some(items),
        BlockEntity::Hopper(h) => Some(&h.items),
        BlockEntity::Crafter(c) => Some(&c.items),
        BlockEntity::Shelf(items) => Some(items),
        _ => None,
    }
}

fn items_mut(level: &mut Level, inv: Inv) -> &mut Vec<Stack> {
    let pos = match inv {
        Inv::Block(pos) => pos,
        Inv::Cart(k) => match &mut level.entities[k].data {
            Data::Minecart(m) => return m.items.as_mut().expect("a container minecart"),
            _ => panic!("no container minecart at {k}"),
        },
    };
    match level.block_entities.get_mut(&pos) {
        Some(BlockEntity::Chest(items) | BlockEntity::Dropper(items) | BlockEntity::ShulkerBox(items)) => items,
        Some(BlockEntity::Hopper(h)) => &mut h.items,
        Some(BlockEntity::Crafter(c)) => &mut c.items,
        Some(BlockEntity::Shelf(items)) => items,
        _ => panic!("no container at {pos:?}"),
    }
}

fn is_hopper(level: &Level, inv: Inv) -> bool {
    matches!(inv, Inv::Block(pos) if matches!(level.block_entities.get(&pos), Some(BlockEntity::Hopper(_))))
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

pub fn size(level: &Level, inv: impl Into<Inv>) -> usize {
    items(level, inv).map_or(0, Vec::len)
}

pub(super) fn get_item(level: &Level, inv: impl Into<Inv>, slot: usize) -> Stack {
    items(level, inv).map(|i| i[slot].clone()).unwrap_or_default()
}

fn is_empty(level: &Level, inv: Inv) -> bool {
    items(level, inv).is_none_or(|i| i.iter().all(Stack::is_empty))
}

/// `Container.setItem`.
pub fn set_item(level: &mut Level, inv: impl Into<Inv>, slot: usize, mut stack: Stack) {
    let inv = inv.into();
    // A stack whose own `max_stack_size` the engine refuses keeps its count,
    // so a refused build saves what it was given.
    let own_size = stack.data.as_ref().is_some_and(|d| d.sets_stack_size());
    if !own_size {
        stack.count = stack.count.min(stack.max_stack_size());
    }
    if stack.is_empty() {
        stack = Stack::default();
    }
    items_mut(level, inv)[slot] = stack;
    if !is_hopper(level, inv) {
        set_changed(level, inv);
    }
}

/// `Container.removeItem` (`ContainerHelper.removeItem`).
fn remove_item(level: &mut Level, inv: Inv, slot: usize, amount: i32) -> Stack {
    let stack = {
        let s = &mut items_mut(level, inv)[slot];
        if s.is_empty() || amount <= 0 { Stack::default() } else { s.split(amount) }
    };
    if !stack.is_empty() && !is_hopper(level, inv) {
        set_changed(level, inv);
    }
    stack
}

pub(super) fn set_count(level: &mut Level, inv: impl Into<Inv>, slot: usize, count: i32) {
    items_mut(level, inv.into())[slot].count = count;
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
/// A minecart's does nothing.
pub fn set_changed(level: &mut Level, inv: impl Into<Inv>) {
    let Inv::Block(pos) = inv.into() else { return };
    if let Some(items) = items(level, pos) {
        let detail = describe(items);
        level.record_items(pos, detail);
    }
    set_changed_static(level, Inv::Block(pos));
}

/// The static `BlockEntity.setChanged(level, pos, state)`.
fn set_changed_static(level: &mut Level, inv: Inv) {
    let Inv::Block(pos) = inv else { return };
    let state = level.get(pos);
    if !level.is_air(state) {
        level.update_neighbour_for_output_signal(pos, level.block(state));
    }
}

/// `AbstractContainerMenu.getRedstoneSignalFromContainer`.
pub fn signal_from_items(items: &[Stack]) -> i32 {
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

/// `HopperBlockEntity.getContainerAt(level, pos)`: the container block at
/// `pos`, or else a container minecart in that block's space.
pub(super) fn container_at(level: &mut Level, pos: Pos) -> Option<Inv> {
    container_at_xyz(level, pos, pos.x as f64 + 0.5, pos.y as f64 + 0.5, pos.z as f64 + 0.5)
}

fn container_at_xyz(level: &mut Level, pos: Pos, x: f64, y: f64, z: f64) -> Option<Inv> {
    block_container(level, pos).or_else(|| entity_container(level, x, y, z))
}

/// `getBlockContainer`: a container block entity at `pos`, if the block
/// there is a container.
fn block_container(level: &Level, pos: Pos) -> Option<Inv> {
    let kind = level.kind_at(pos);
    let ok = matches!(
        kind,
        Kind::Chest | Kind::Barrel | Kind::Hopper | Kind::Dropper | Kind::Dispenser | Kind::ShulkerBox | Kind::Crafter
    );
    (ok && items(level, pos).is_some()).then_some(Inv::Block(pos))
}

/// `getEntityContainer`: a container minecart whose box meets the block
/// space centred on (x, y, z), picked with the level's random source even
/// when there is only one.
fn entity_container(level: &mut Level, x: f64, y: f64, z: f64) -> Option<Inv> {
    let b = Aabb::new(x - 0.5, y - 0.5, z - 0.5, x + 0.5, y + 0.5, z + 0.5);
    let found: Vec<usize> = (0..level.entities.len())
        .filter(|&k| {
            let e = &level.entities[k];
            !e.removed && cart_items(level, k).is_some() && e.bounding_box().intersects(&b)
        })
        .collect();
    if found.is_empty() {
        return None;
    }
    let i = level.random.next_int_bounded(found.len() as i32) as usize;
    Some(Inv::Cart(found[i]))
}

/// `Containers.dropContents` for the container block entity at `pos`, as
/// it is removed: each slot, filled or not, takes a spot from the level's
/// random source, and a filled one flies out from there in stacks of 10 to
/// 30, as item entities in levels that simulate entities.
pub fn drop_contents(level: &mut Level, pos: Pos) {
    let Some(stacks) = items(level, pos).cloned() else { return };
    for stack in stacks {
        drop_item_stack(level, pos, stack);
    }
}

/// `Containers.dropItemStack` at the block `pos`.
fn drop_item_stack(level: &mut Level, pos: Pos, mut stack: Stack) {
    // `EntityTypes.ITEM.getWidth()`: an item is a quarter of a block wide.
    let (range, half) = (1.0 - 0.25, 0.25 / 2.0);
    let r = &mut level.random;
    let x = pos.x as f64 + r.next_double() * range + half;
    let y = pos.y as f64 + r.next_double() * range;
    let z = pos.z as f64 + r.next_double() * range + half;
    while !stack.is_empty() {
        let n = level.random.next_int_bounded(21) + 10;
        let part = stack.split(n);
        let made = level.simulate_entities.then(|| crate::entity::item::new_tossed(level, Vec3::new(x, y, z), part));
        let r = &mut level.random;
        let spread = 0.11485000171139836;
        let motion = Vec3::new(r.triangle(0.0, spread), r.triangle(0.2, spread), r.triangle(0.0, spread));
        if let Some(mut e) = made {
            e.motion = motion;
            level.add_entity(e);
        }
    }
}

/// `isFullContainer`.
fn is_full_container(level: &Level, inv: Inv) -> bool {
    items(level, inv).is_some_and(|i| i.iter().all(|s| s.count >= s.max_stack_size()))
}

// --- moving items ----------------------------------------------------------------

/// `HopperBlockEntity.addItem(source, destination, stack, direction)`.
pub(super) fn add_item(level: &mut Level, source: Option<Inv>, dest: Inv, mut stack: Stack) -> Stack {
    let n = size(level, dest);
    let mut slot = 0;
    while slot < n && !stack.is_empty() {
        stack = try_move_in_item(level, source, dest, stack, slot);
        slot += 1;
    }
    stack
}

/// `BlockItem.canFitInsideContainerItems` is false for shulker boxes, of
/// any colour.
fn is_shulker_box(id: &str) -> bool {
    let name = id.strip_prefix("minecraft:").unwrap_or(id);
    name == "shulker_box" || name.ends_with("_shulker_box")
}

/// `HopperBlockEntity.tryMoveInItem`.
fn try_move_in_item(level: &mut Level, source: Option<Inv>, dest: Inv, mut stack: Stack, slot: usize) -> Stack {
    // `canPlaceItemInContainer`: a crafter spreads items over its slots, and
    // a shulker box refuses shulker boxes
    // (`ShulkerBoxBlockEntity.canPlaceItemThroughFace`).
    if let Inv::Block(d) = dest {
        let refused = match level.kind_at(d) {
            Kind::Crafter => !super::crafter::can_place_item(level, d, slot),
            Kind::ShulkerBox => is_shulker_box(&stack.id),
            _ => false,
        };
        if refused {
            return stack;
        }
    }
    let current = get_item(level, dest, slot);
    let mut changed = false;
    let was_empty = is_empty(level, dest);
    if current.is_empty() {
        set_item(level, dest, slot, stack);
        stack = Stack::default();
        changed = true;
    } else if current.same_item(&stack) && current.count < current.max_stack_size() {
        let n = stack.count.min(stack.max_stack_size() - current.count);
        stack.count -= n;
        set_count(level, dest, slot, current.count + n);
        changed = n > 0;
    }
    if changed {
        if let Inv::Block(d) = dest
            && was_empty
            && is_hopper(level, dest)
            && !hopper(level, d).on_custom_cooldown()
        {
            let mut skip = 0;
            if let Some(Inv::Block(src)) = source
                && is_hopper(level, Inv::Block(src))
                && hopper(level, d).ticked_game_time >= hopper(level, src).ticked_game_time
            {
                skip = 1;
            }
            hopper_mut(level, d).cooldown = 8 - skip;
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
        try_move_items(level, pos, Intake::Above);
    }
}

/// What a hopper takes in once it has tried to eject.
enum Intake {
    /// `suckInItems`: from the container or the items above.
    Above,
    /// `addItem(hopper, itemEntity)` for the item that just entered it, at
    /// this index in the level's list.
    Entity(usize),
}

/// `tryMoveItems`.
fn try_move_items(level: &mut Level, pos: Pos, intake: Intake) -> bool {
    let state = level.get(pos);
    if hopper(level, pos).on_cooldown() || !level.flag(state, "enabled") {
        return false;
    }
    let mut changed = false;
    let inv = Inv::Block(pos);
    if !is_empty(level, inv) {
        changed = eject_items(level, pos, state);
    }
    if !is_full_container(level, inv) {
        changed |= match intake {
            Intake::Above => suck_in_items(level, inv),
            Intake::Entity(k) => add_item_entity(level, inv, k),
        };
    }
    if changed {
        hopper_mut(level, pos).cooldown = 8;
        set_changed_static(level, inv);
    }
    changed
}

/// `Hopper.SUCK_AABB` at the hopper: a full block from 11/16 up to two
/// blocks above its base.
fn suck_box(pos: Pos) -> Aabb {
    Aabb::new(0.0, 11.0 / 16.0, 0.0, 1.0, 2.0, 1.0).offset(pos.x as f64, pos.y as f64, pos.z as f64)
}

/// `HopperBlockEntity.entityInside`: an item that touches the suck box
/// goes in at once.
pub fn hopper_entity_inside(level: &mut Level, k: usize, pos: Pos) {
    let e = &level.entities[k];
    let crate::entity::Data::Item(item) = &e.data else { return };
    if item.stack.is_empty() || !e.bounding_box().intersects(&suck_box(pos)) {
        return;
    }
    try_move_items(level, pos, Intake::Entity(k));
}

/// `addItem(container, itemEntity)`: as much of the stack as fits.
fn add_item_entity(level: &mut Level, dest: Inv, k: usize) -> bool {
    let crate::entity::Data::Item(item) = &level.entities[k].data else { return false };
    // Entities keep bare item ids; containers keep namespaced ones.
    let stack =
        Stack::new(&format!("minecraft:{}", item.stack.id), item.stack.count).with_data(item.stack.data.clone());
    let rest = add_item(level, None, dest, stack);
    let e = &mut level.entities[k];
    let crate::entity::Data::Item(item) = &mut e.data else { unreachable!() };
    item.stack.count = rest.count;
    if rest.is_empty() {
        e.discard();
    }
    rest.is_empty()
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
    let inv = Inv::Block(pos);
    for slot in 0..size(level, pos) {
        let stack = get_item(level, pos, slot);
        if stack.is_empty() {
            continue;
        }
        let count = stack.count;
        let moving = remove_item(level, inv, slot, 1);
        let result = add_item(level, Some(inv), dest, moving);
        if result.is_empty() {
            set_changed(level, dest);
            return true;
        }
        restore(level, inv, slot, stack, count);
    }
    false
}

/// Put back a stack whose item did not fit: `stack.setCount(count)`, and
/// `setItem` if removing emptied the slot.
fn restore(level: &mut Level, inv: Inv, slot: usize, mut stack: Stack, count: i32) {
    if count == 1 {
        stack.count = count;
        set_item(level, inv, slot, stack);
    } else {
        set_count(level, inv, slot, count);
    }
}

/// `HopperBlockEntity.suckInItems`, for a hopper block or a hopper
/// minecart: one item from the container above, or else any item lying in
/// the suck box. A full block on a hopper block stops it taking items.
fn suck_in_items(level: &mut Level, hopper: Inv) -> bool {
    // `getLevelX/Y/Z`: a block's middle, or the cart's feet raised by 0.5.
    let (x, y, z, grid) = match hopper {
        Inv::Block(p) => (p.x as f64 + 0.5, p.y as f64 + 0.5, p.z as f64 + 0.5, true),
        Inv::Cart(k) => {
            let p = level.entities[k].pos;
            (p.x, p.y + 0.5, p.z, false)
        }
    };
    let above = Pos::new(floor(x), floor(y + 1.0), floor(z));
    let Some(source) = container_at_xyz(level, above, x, y + 1.0, z) else {
        if grid && level.physics().is_full_block(level.get(above)) {
            return false;
        }
        let b = Aabb::new(0.0, 11.0 / 16.0, 0.0, 1.0, 2.0, 1.0).offset(x - 0.5, y - 0.5, z - 0.5);
        return items_in(level, b).into_iter().any(|k| add_item_entity(level, hopper, k));
    };
    for slot in 0..size(level, source) {
        let stack = get_item(level, source, slot);
        if stack.is_empty() {
            continue;
        }
        let count = stack.count;
        let moving = remove_item(level, source, slot, 1);
        let result = add_item(level, Some(source), hopper, moving);
        if result.is_empty() {
            set_changed(level, source);
            return true;
        }
        restore(level, source, slot, stack, count);
    }
    false
}

/// Item entities whose boxes meet `b`, oldest first.
fn items_in(level: &Level, b: Aabb) -> Vec<usize> {
    let mut items: Vec<usize> = (0..level.entities.len())
        .filter(|&k| {
            let e = &level.entities[k];
            !e.removed && matches!(e.data, Data::Item(_)) && e.bounding_box().intersects(&b)
        })
        .collect();
    items.sort_by_key(|&k| level.entities[k].id);
    items
}

/// `MinecartHopper.suckInItems` for the cart at `k`: what a hopper takes,
/// or else an item touching the cart.
pub fn hopper_cart_suck(level: &mut Level, k: usize) -> bool {
    let cart = Inv::Cart(k);
    if suck_in_items(level, cart) {
        return true;
    }
    let b = level.entities[k].bounding_box().inflate(0.25, 0.0, 0.25);
    items_in(level, b).into_iter().any(|i| add_item_entity(level, cart, i))
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

/// `DropperBlock.dispenseFrom`: one item from a slot picked at random, into
/// the container in front, or else out into the world.
pub fn dropper_tick(level: &mut Level, pos: Pos, state: StateId) {
    // An empty dropper only clicks.
    let Some(slot) = super::dispenser::random_slot(level, pos) else {
        return;
    };
    let stack = get_item(level, pos, slot);
    let facing = level.dir(state, "facing");
    let remaining = match container_at(level, pos.offset(facing)) {
        None => super::dispenser::eject(level, pos, state, stack),
        Some(dest) => {
            let result = add_item(level, Some(Inv::Block(pos)), dest, Stack { count: 1, ..stack.clone() });
            if result.is_empty() { Stack { count: stack.count - 1, ..stack.clone() } } else { stack.clone() }
        }
    };
    set_item(level, pos, slot, remaining);
}
