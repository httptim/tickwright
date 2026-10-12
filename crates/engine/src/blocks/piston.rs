//! Pistons: `PistonBaseBlock`, `PistonStructureResolver`, `PistonHeadBlock`,
//! `MovingPistonBlock` and `PistonMovingBlockEntity`.

use crate::game_event::GameEvent;
use crate::level::{Level, flags};
use crate::pos::{Dir, Pos, java_hash_set_order};
use crate::registry::StateId;

use super::{BlockEntity, Kind};

/// `PistonStructureResolver.MAX_PUSH_DEPTH`.
const MAX_PUSH: usize = 12;

/// `PushReaction`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PushReaction {
    Normal,
    Destroy,
    Block,
}

fn push_reaction(level: &Level, state: StateId) -> PushReaction {
    match level.kind(state) {
        Kind::Lever
        | Kind::Button
        | Kind::RedstoneWire
        | Kind::RedstoneTorch
        | Kind::RedstoneWallTorch
        | Kind::Repeater
        | Kind::Comparator
        | Kind::Cake
        | Kind::Door
        | Kind::Bell
        | Kind::Leaves
        | Kind::ShulkerBox
        | Kind::TripwireHook
        | Kind::Tripwire
        | Kind::Water => PushReaction::Destroy,
        Kind::MovingPiston | Kind::PistonHead => PushReaction::Block,
        _ => PushReaction::Normal,
    }
}

/// `getDestroySpeed(...) == -1`: blocks that cannot be broken.
fn unbreakable(level: &Level, state: StateId) -> bool {
    level.kind(state) == Kind::MovingPiston || level.reg.name(state) == "minecraft:bedrock"
}

fn is_piston(level: &Level, state: StateId) -> bool {
    matches!(level.kind(state), Kind::Piston | Kind::StickyPiston)
}

fn is_sticky_block(level: &Level, state: StateId) -> bool {
    matches!(level.kind(state), Kind::Slime | Kind::Honey)
}

fn can_stick_to_each_other(level: &Level, a: StateId, b: StateId) -> bool {
    let (ka, kb) = (level.kind(a), level.kind(b));
    if (ka == Kind::Honey && kb == Kind::Slime) || (ka == Kind::Slime && kb == Kind::Honey) {
        return false;
    }
    is_sticky_block(level, a) || is_sticky_block(level, b)
}

/// `PistonBaseBlock.isPushable`.
fn is_pushable(level: &Level, state: StateId, push: Dir, allow_destroy: bool, piston_facing: Dir) -> bool {
    if level.is_air(state) {
        return true;
    }
    if level.reg.name(state) == "minecraft:obsidian" {
        return false;
    }
    if !is_piston(level, state) {
        if unbreakable(level, state) {
            return false;
        }
        match push_reaction(level, state) {
            PushReaction::Block => return false,
            PushReaction::Destroy => return allow_destroy,
            PushReaction::Normal => {}
        }
    } else if level.flag(state, "extended") {
        return false;
    }
    let _ = (push, piston_facing);
    !super::has_block_entity(level.kind(state))
}

/// `PistonStructureResolver`.
struct Resolver {
    piston: Pos,
    push: Dir,
    piston_dir: Dir,
    start: Pos,
    extending: bool,
    to_push: Vec<Pos>,
    to_destroy: Vec<Pos>,
}

impl Resolver {
    fn new(piston: Pos, dir: Dir, extending: bool) -> Resolver {
        let (push, start) =
            if extending { (dir, piston.offset(dir)) } else { (dir.opposite(), piston.offset(dir).offset(dir)) };
        Resolver { piston, push, piston_dir: dir, start, extending, to_push: Vec::new(), to_destroy: Vec::new() }
    }

    fn resolve(&mut self, level: &Level) -> bool {
        self.to_push.clear();
        self.to_destroy.clear();
        let state = level.get(self.start);
        if !is_pushable(level, state, self.push, false, self.piston_dir) {
            if self.extending && push_reaction(level, state) == PushReaction::Destroy {
                self.to_destroy.push(self.start);
                return true;
            }
            return false;
        }
        if !self.add_block_line(level, self.start, self.push) {
            return false;
        }
        let mut i = 0;
        while i < self.to_push.len() {
            let p = self.to_push[i];
            if is_sticky_block(level, level.get(p)) && !self.add_branching_blocks(level, p) {
                return false;
            }
            i += 1;
        }
        true
    }

    fn add_block_line(&mut self, level: &Level, start: Pos, dir: Dir) -> bool {
        let mut state = level.get(start);
        if level.is_air(state)
            || !is_pushable(level, state, self.push, false, dir)
            || start == self.piston
            || self.to_push.contains(&start)
        {
            return true;
        }
        let back = self.push.opposite();
        let mut i = 1;
        if i + self.to_push.len() > MAX_PUSH {
            return false;
        }
        while is_sticky_block(level, state) {
            let p = offset_n(start, back, i);
            let prev = state;
            state = level.get(p);
            if level.is_air(state)
                || !can_stick_to_each_other(level, prev, state)
                || !is_pushable(level, state, self.push, false, back)
                || p == self.piston
            {
                break;
            }
            i += 1;
            if i + self.to_push.len() > MAX_PUSH {
                return false;
            }
        }
        let mut j = 0;
        for k in (0..i).rev() {
            self.to_push.push(offset_n(start, back, k));
            j += 1;
        }
        let mut k = 1;
        loop {
            let p = offset_n(start, self.push, k);
            if let Some(l) = self.to_push.iter().position(|&x| x == p) {
                self.reorder_list_at_collision(j, l);
                for m in 0..=l + j {
                    let q = self.to_push[m];
                    if is_sticky_block(level, level.get(q)) && !self.add_branching_blocks(level, q) {
                        return false;
                    }
                }
                return true;
            }
            let s = level.get(p);
            if level.is_air(s) {
                return true;
            }
            if !is_pushable(level, s, self.push, true, self.push) || p == self.piston {
                return false;
            }
            if push_reaction(level, s) == PushReaction::Destroy {
                self.to_destroy.push(p);
                return true;
            }
            if self.to_push.len() >= MAX_PUSH {
                return false;
            }
            self.to_push.push(p);
            j += 1;
            k += 1;
        }
    }

    fn reorder_list_at_collision(&mut self, i: usize, j: usize) {
        let n = self.to_push.len();
        let mut out = self.to_push[..j].to_vec();
        out.extend_from_slice(&self.to_push[n - i..]);
        out.extend_from_slice(&self.to_push[j..n - i]);
        self.to_push = out;
    }

    fn add_branching_blocks(&mut self, level: &Level, from: Pos) -> bool {
        let state = level.get(from);
        for dir in Dir::ALL {
            if dir.axis() != self.push.axis() {
                let p = from.offset(dir);
                if can_stick_to_each_other(level, level.get(p), state) && !self.add_block_line(level, p, dir) {
                    return false;
                }
            }
        }
        true
    }
}

fn offset_n(p: Pos, d: Dir, n: usize) -> Pos {
    (0..n).fold(p, |p, _| p.offset(d))
}

// --- PistonBaseBlock ---------------------------------------------------------

fn sticky(level: &Level, state: StateId) -> bool {
    level.kind(state) == Kind::StickyPiston
}

/// `getNeighborSignal`, quasi-connectivity included.
fn neighbor_signal(level: &Level, pos: Pos, facing: Dir) -> bool {
    if Dir::ALL.iter().any(|&d| d != facing && level.has_signal(pos.offset(d), d)) {
        return true;
    }
    if level.has_signal(pos, Dir::Down) {
        return true;
    }
    let above = pos.above();
    Dir::ALL.iter().any(|&d| d != Dir::Down && level.has_signal(above.offset(d), d))
}

/// `checkIfExtend`.
pub(super) fn check_if_extend(level: &mut Level, pos: Pos, state: StateId) {
    let dir = level.dir(state, "facing");
    let powered = neighbor_signal(level, pos, dir);
    let extended = level.flag(state, "extended");
    let block = level.block(state);
    if powered && !extended {
        if Resolver::new(pos, dir, true).resolve(level) {
            level.block_event(pos, block, 0, dir as i32);
        }
    } else if !powered && extended {
        let p = pos.offset(dir).offset(dir);
        let s = level.get(p);
        let mut kind = 1;
        if level.kind(s) == Kind::MovingPiston
            && level.dir(s, "facing") == dir
            && let Some(BlockEntity::Piston(be)) = level.block_entities.get(&p)
            && be.extending
            && (be.progress_o < 0.5 || level.game_time == be.last_ticked || level.handling_tick())
        {
            kind = 2;
        }
        level.block_event(pos, block, kind, dir as i32);
    }
}

pub fn neighbor_changed(level: &mut Level, pos: Pos, state: StateId) {
    check_if_extend(level, pos, state);
}

pub fn on_place(level: &mut Level, pos: Pos, state: StateId, old: StateId) {
    if level.block(old) != level.block(state) && !level.block_entities.contains_key(&pos) {
        check_if_extend(level, pos, state);
    }
}

pub fn trigger_event(level: &mut Level, pos: Pos, state: StateId, a: i32, b: i32) -> bool {
    let dir = level.dir(state, "facing");
    let extended = level.with_bool(state, "extended", true);
    let powered = neighbor_signal(level, pos, dir);
    if powered && (a == 1 || a == 2) {
        level.set_block(pos, extended, flags::CLIENTS);
        return false;
    }
    if !powered && a == 0 {
        return false;
    }
    let is_sticky = sticky(level, state);
    if a == 0 {
        if !move_blocks(level, pos, dir, true, is_sticky) {
            return false;
        }
        level.set_block(pos, extended, flags::MOVE_BY_PISTON | flags::ALL);
        level.game_event(GameEvent::BlockActivate, pos);
    } else if a == 1 || a == 2 {
        let front = pos.offset(dir);
        if matches!(level.block_entities.get(&front), Some(BlockEntity::Piston(_))) {
            final_tick(level, front);
        }
        let moving = moving_state(level, dir, is_sticky);
        level.set_block(pos, moving, flags::SKIP_BLOCK_ENTITY_SIDEEFFECTS | flags::KNOWN_SHAPE | flags::INVISIBLE);
        let base = level.with_dir(level.default_state(level.block(state)), "facing", Dir::ALL[(b & 7) as usize]);
        level.add_block_entity(pos, BlockEntity::Piston(Moving::new(base, dir, false, true)));
        level.update_neighbors_at(pos, level.block(moving));
        level.update_neighbour_shapes(pos, moving, flags::CLIENTS, crate::level::MAX_RECURSION);
        if is_sticky {
            let pull = pos.offset(dir).offset(dir);
            let ps = level.get(pull);
            let mut spit = false;
            if level.kind(ps) == Kind::MovingPiston
                && let Some(BlockEntity::Piston(be)) = level.block_entities.get(&pull)
                && be.direction == dir
                && be.extending
            {
                final_tick(level, pull);
                spit = true;
            }
            if !spit {
                let ps = level.get(pull);
                if a != 1
                    || level.is_air(ps)
                    || !is_pushable(level, ps, dir.opposite(), false, dir)
                    || (push_reaction(level, ps) != PushReaction::Normal && !is_piston(level, ps))
                {
                    level.remove_block(pos.offset(dir));
                } else {
                    move_blocks(level, pos, dir, false, is_sticky);
                }
            }
        } else {
            level.remove_block(pos.offset(dir));
        }
        level.game_event(GameEvent::BlockDeactivate, pos);
    }
    true
}

fn moving_state(level: &Level, dir: Dir, sticky: bool) -> StateId {
    let s = level.default_state(level.block_id("moving_piston"));
    let s = level.with_dir(s, "facing", dir);
    level.with(s, "type", if sticky { "sticky" } else { "normal" })
}

/// `moveBlocks`.
fn move_blocks(level: &mut Level, piston: Pos, dir: Dir, extending: bool, is_sticky: bool) -> bool {
    let arm = piston.offset(dir);
    if !extending && level.kind_at(arm) == Kind::PistonHead {
        level.set_block(arm, level.air, flags::SKIP_BLOCK_ENTITY_SIDEEFFECTS | flags::KNOWN_SHAPE | flags::INVISIBLE);
    }
    let mut resolver = Resolver::new(piston, dir, extending);
    if !resolver.resolve(level) {
        return false;
    }
    let to_push = resolver.to_push.clone();
    let to_destroy = resolver.to_destroy.clone();
    // `toRemove` is a HashMap<BlockPos, BlockState>: remember the states and
    // which keys are removed, and iterate in the map's order at the end.
    let shapes: Vec<StateId> = to_push.iter().map(|&p| level.get(p)).collect();
    let mut removed: Vec<Pos> = Vec::new();
    let mut to_update: Vec<StateId> = Vec::new();
    let push_dir = if extending { dir } else { dir.opposite() };

    for &p in to_destroy.iter().rev() {
        let s = level.get(p);
        if level.simulate_entities {
            level.drop_resources(p, s);
        }
        level.set_block(p, level.air, flags::KNOWN_SHAPE | flags::CLIENTS);
        to_update.push(s);
    }
    for i in (0..to_push.len()).rev() {
        let s = level.get(to_push[i]);
        let p = to_push[i].offset(push_dir);
        removed.push(p);
        let moving = level.with_dir(level.default_state(level.block_id("moving_piston")), "facing", dir);
        level.set_block(p, moving, flags::MOVE_BY_PISTON | flags::INVISIBLE);
        level.add_block_entity(p, BlockEntity::Piston(Moving::new(shapes[i], dir, extending, false)));
        to_update.push(s);
    }
    if extending {
        let head = level.default_state(level.block_id("piston_head"));
        let head = level.with_dir(head, "facing", dir);
        let head = level.with(head, "type", if is_sticky { "sticky" } else { "normal" });
        let moving = moving_state(level, dir, is_sticky);
        removed.push(arm);
        level.set_block(arm, moving, flags::MOVE_BY_PISTON | flags::INVISIBLE);
        level.add_block_entity(arm, BlockEntity::Piston(Moving::new(head, dir, true, true)));
    }
    let to_remove: Vec<(Pos, StateId)> = java_hash_set_order(&to_push)
        .into_iter()
        .filter(|p| !removed.contains(p))
        .map(|p| (p, shapes[to_push.iter().position(|&q| q == p).unwrap()]))
        .collect();
    for &(p, _) in &to_remove {
        level.set_block(p, level.air, flags::MOVE_BY_PISTON | flags::KNOWN_SHAPE | flags::CLIENTS);
    }
    for &(p, s) in &to_remove {
        super::update_indirect_neighbour_shapes(level, p, s, flags::CLIENTS, crate::level::MAX_RECURSION);
        level.update_neighbour_shapes(p, level.air, flags::CLIENTS, crate::level::MAX_RECURSION);
    }
    let mut k = 0;
    for &p in to_destroy.iter().rev() {
        let s = to_update[k];
        k += 1;
        super::affect_neighbors_after_removal(level, p, s, false);
        super::update_indirect_neighbour_shapes(level, p, s, flags::CLIENTS, crate::level::MAX_RECURSION);
        level.update_neighbors_at(p, level.block(s));
    }
    for &p in to_push.iter().rev() {
        let s = to_update[k];
        k += 1;
        level.update_neighbors_at(p, level.block(s));
    }
    if extending {
        level.update_neighbors_at(arm, level.block_id("piston_head"));
    }
    true
}

// --- PistonHeadBlock -----------------------------------------------------------

fn is_fitting_base(level: &Level, head: StateId, base: StateId) -> bool {
    let want = if level.prop(head, "type") == "sticky" { Kind::StickyPiston } else { Kind::Piston };
    level.kind(base) == want && level.flag(base, "extended") && level.dir(base, "facing") == level.dir(head, "facing")
}

fn head_can_survive(level: &Level, pos: Pos, state: StateId) -> bool {
    let facing = level.dir(state, "facing");
    let b = level.get(pos.offset(facing.opposite()));
    is_fitting_base(level, state, b) || (level.kind(b) == Kind::MovingPiston && level.dir(b, "facing") == facing)
}

pub fn head_update_shape(level: &Level, pos: Pos, state: StateId, dir: Dir) -> StateId {
    if dir.opposite() == level.dir(state, "facing") && !head_can_survive(level, pos, state) { level.air } else { state }
}

pub fn head_neighbor_changed(level: &mut Level, pos: Pos, state: StateId, source: crate::registry::BlockId) {
    if head_can_survive(level, pos, state) {
        level.neighbor_changed(pos.offset(level.dir(state, "facing").opposite()), source);
    }
}

pub fn head_affect_neighbors_after_removal(level: &mut Level, pos: Pos, state: StateId) {
    let base = pos.offset(level.dir(state, "facing").opposite());
    if is_fitting_base(level, state, level.get(base)) {
        level.destroy_block(base, true, crate::level::MAX_RECURSION);
    }
}

// --- PistonMovingBlockEntity -----------------------------------------------------

#[derive(Clone, Debug)]
pub struct Moving {
    pub moved_state: StateId,
    pub direction: Dir,
    pub extending: bool,
    pub is_source_piston: bool,
    pub progress: f32,
    pub progress_o: f32,
    pub last_ticked: i64,
}

impl Moving {
    fn new(moved_state: StateId, direction: Dir, extending: bool, is_source_piston: bool) -> Moving {
        Moving { moved_state, direction, extending, is_source_piston, progress: 0.0, progress_o: 0.0, last_ticked: 0 }
    }
}

/// `PistonMovingBlockEntity.tick`.
pub fn tick_moving(level: &mut Level, pos: Pos) {
    let now = level.game_time;
    let Some(BlockEntity::Piston(be)) = level.block_entities.get_mut(&pos) else { return };
    be.last_ticked = now;
    be.progress_o = be.progress;
    if be.progress_o >= 1.0 {
        let moved = be.moved_state;
        level.remove_block_entity(pos);
        if level.kind_at(pos) == Kind::MovingPiston {
            let s = level.update_from_neighbour_shapes(moved, pos);
            if level.is_air(s) {
                level.set_block(pos, moved, flags::MOVE_BY_PISTON | flags::KNOWN_SHAPE | flags::INVISIBLE);
                level.update_or_destroy(moved, s, pos, flags::ALL, crate::level::MAX_RECURSION);
            } else {
                level.set_block(pos, s, flags::MOVE_BY_PISTON | flags::ALL);
                level.neighbor_changed(pos, level.block(s));
            }
        }
    } else {
        be.progress = (be.progress + 0.5).min(1.0);
    }
}

/// `PistonMovingBlockEntity.finalTick`.
pub fn final_tick(level: &mut Level, pos: Pos) {
    let Some(BlockEntity::Piston(be)) = level.block_entities.get_mut(&pos) else { return };
    if be.progress_o >= 1.0 {
        return;
    }
    be.progress = 1.0;
    be.progress_o = 1.0;
    let (moved, source) = (be.moved_state, be.is_source_piston);
    level.remove_block_entity(pos);
    if level.kind_at(pos) == Kind::MovingPiston {
        let s = if source { level.air } else { level.update_from_neighbour_shapes(moved, pos) };
        level.set_block(pos, s, flags::ALL);
        level.neighbor_changed(pos, level.block(s));
    }
}

/// `isFaceSturdy` for a moving piston, from
/// `PistonMovingBlockEntity.getCollisionShape`: a retracting piston base
/// keeps its extended base's shape, and a moved block counts only once it
/// has arrived. In between, a partly moved block is never whole on a face.
pub fn moving_face_sturdy(level: &Level, pos: Pos, dir: Dir) -> bool {
    let Some(BlockEntity::Piston(be)) = level.block_entities.get(&pos) else { return false };
    let moved = be.moved_state;
    if !be.extending && be.is_source_piston && is_piston(level, moved) && level.dir(moved, "facing").opposite() == dir {
        return true;
    }
    let offset = if be.extending { be.progress - 1.0 } else { 1.0 - be.progress };
    offset == 0.0 && !be.is_source_piston && super::is_face_sturdy(level, pos, moved, dir)
}
