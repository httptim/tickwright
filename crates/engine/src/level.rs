//! The world and the machinery every block shares: writes, neighbor and
//! shape updates, scheduled ticks, block events and the tick loop.
//!
//! Each piece follows the game's own structure (`Level.setBlock`,
//! `CollectingNeighborUpdater`, `LevelTicks`, `ServerLevel.runBlockEvents`)
//! closely enough that the same calls happen in the same order, because the
//! order is what the traces check.

use std::cell::Cell;
use std::collections::{HashMap, HashSet, VecDeque};

use crate::blocks::{self, Kind};
use crate::pos::{Dir, Pos, UPDATE_ORDER, UPDATE_SHAPE_ORDER};
use crate::record::{Event, Recorder};
use crate::registry::{BlockId, Registry, StateId};
use crate::rules::Rules;
use crate::world::World;
use tickwright_trace::Phase;

/// `Block.UPDATE_*` flags.
pub mod flags {
    pub const NEIGHBORS: i32 = 1;
    pub const CLIENTS: i32 = 2;
    pub const INVISIBLE: i32 = 4;
    pub const IMMEDIATE: i32 = 8;
    pub const KNOWN_SHAPE: i32 = 16;
    pub const SUPPRESS_DROPS: i32 = 32;
    pub const MOVE_BY_PISTON: i32 = 64;
    pub const SKIP_SHAPE_UPDATE_ON_WIRE: i32 = 128;
    pub const SKIP_BLOCK_ENTITY_SIDEEFFECTS: i32 = 256;
    pub const SKIP_ON_PLACE: i32 = 512;
    pub const ALL: i32 = NEIGHBORS | CLIENTS;
    pub const ALL_IMMEDIATE: i32 = ALL | IMMEDIATE;
}

/// `Level.setBlock`'s default recursion budget for shape updates.
pub const MAX_RECURSION: i32 = 512;

/// `TickPriority`, as its integer value.
pub type Priority = i32;

#[derive(Clone, Copy, Debug)]
struct ScheduledTick {
    block: BlockId,
    pos: Pos,
    trigger: i64,
    priority: Priority,
    sub: u64,
}

/// One `NeighborUpdater` entry.
#[derive(Clone, Debug)]
enum Update {
    /// `MultiNeighborUpdate`: neighbor-changed to each side of `source` in
    /// `UPDATE_ORDER`, except `skip`.
    Multi { source: Pos, block: BlockId, skip: Option<Dir>, idx: usize },
    /// `SimpleNeighborUpdate`: neighbor-changed to one position.
    Simple { pos: Pos, block: BlockId, moved_by_piston: bool },
    /// `ShapeUpdate`.
    Shape { dir: Dir, pos: Pos, neighbor: Pos, neighbor_state: StateId, flags: i32, recursion: i32 },
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct BlockEvent {
    pub pos: Pos,
    pub block: BlockId,
    pub a: i32,
    pub b: i32,
}

pub struct Level {
    pub reg: &'static Registry,
    pub air: StateId,
    kinds: Vec<Kind>,
    blocks: World,
    pub game_time: i64,

    // LevelTicks
    pending_ticks: Vec<ScheduledTick>,
    pending_keys: HashSet<(Pos, BlockId)>,
    running_ticks: HashSet<(Pos, BlockId)>,
    next_sub_tick: u64,

    // ServerLevel.blockEvents: a linked hash set.
    events: VecDeque<BlockEvent>,

    // CollectingNeighborUpdater
    stack: Vec<Update>,
    added_this_layer: Vec<Update>,
    update_count: usize,

    /// Block entities by position, and the order they tick in.
    pub block_entities: HashMap<Pos, blocks::BlockEntity>,
    /// `RebindableTickingBlockEntityWrapper`s: each is a position and
    /// whether it is still bound. A block entity replaced at the same
    /// position keeps its wrapper's place in the list.
    ticker_slots: Vec<(Pos, bool)>,
    ticker_at: HashMap<Pos, usize>,
    tickers: Vec<usize>,
    pending_tickers: Vec<usize>,
    ticking_block_entities: bool,

    /// `RedStoneWireBlock.shouldSignal`: off while dust measures the power
    /// around it, so wires do not count each other.
    pub wire_signals: Cell<bool>,
    /// `RedstoneTorchBlock.RECENT_TOGGLES`: forced turn-offs, oldest first.
    pub torch_toggles: Vec<(Pos, i64)>,

    pub phase: Phase,
    pub recorder: Option<Recorder>,
}

impl Level {
    pub fn new(rules: Rules) -> Level {
        let reg = rules.reg;
        let air = reg.block(reg.block_id("air").unwrap()).default;
        let kinds = rules.into_kinds();
        Level {
            reg,
            air,
            kinds,
            blocks: World::new(air),
            game_time: 0,
            pending_ticks: Vec::new(),
            pending_keys: HashSet::new(),
            running_ticks: HashSet::new(),
            next_sub_tick: 0,
            events: VecDeque::new(),
            stack: Vec::new(),
            added_this_layer: Vec::new(),
            update_count: 0,
            block_entities: HashMap::new(),
            ticker_slots: Vec::new(),
            ticker_at: HashMap::new(),
            tickers: Vec::new(),
            pending_tickers: Vec::new(),
            ticking_block_entities: false,
            wire_signals: Cell::new(true),
            torch_toggles: Vec::new(),
            phase: Phase::Input,
            recorder: None,
        }
    }

    // --- reading -----------------------------------------------------------

    pub fn get(&self, pos: Pos) -> StateId {
        self.blocks.get(pos)
    }

    pub fn block(&self, state: StateId) -> BlockId {
        self.reg.block_of(state)
    }

    pub fn kind(&self, state: StateId) -> Kind {
        self.kinds[self.block(state).0 as usize]
    }

    pub fn kind_at(&self, pos: Pos) -> Kind {
        self.kind(self.get(pos))
    }

    pub fn is(&self, state: StateId, block: BlockId) -> bool {
        self.block(state) == block
    }

    pub fn prop(&self, state: StateId, name: &str) -> &'static str {
        self.reg.value(state, name).unwrap_or_else(|| panic!("{} has no property {name}", self.reg.name(state)))
    }

    pub fn flag(&self, state: StateId, name: &str) -> bool {
        self.prop(state, name) == "true"
    }

    pub fn dir(&self, state: StateId, name: &str) -> Dir {
        Dir::from_name(self.prop(state, name)).unwrap()
    }

    pub fn int(&self, state: StateId, name: &str) -> i32 {
        self.prop(state, name).parse().unwrap()
    }

    pub fn with(&self, state: StateId, name: &str, value: &str) -> StateId {
        self.reg.with(state, name, value)
    }

    pub fn with_bool(&self, state: StateId, name: &str, value: bool) -> StateId {
        self.with(state, name, if value { "true" } else { "false" })
    }

    pub fn with_int(&self, state: StateId, name: &str, value: i32) -> StateId {
        self.with(state, name, &value.to_string())
    }

    pub fn with_dir(&self, state: StateId, name: &str, value: Dir) -> StateId {
        self.with(state, name, value.name())
    }

    pub fn default_state(&self, block: BlockId) -> StateId {
        self.reg.block(block).default
    }

    pub fn block_id(&self, name: &str) -> BlockId {
        self.reg.block_id(name).unwrap_or_else(|| panic!("no block {name}"))
    }

    pub fn is_air(&self, state: StateId) -> bool {
        matches!(self.kind(state), Kind::Air)
    }

    // --- recording ---------------------------------------------------------

    fn record(&mut self, event: Event, pos: Pos) {
        if let Some(r) = self.recorder.as_mut() {
            r.record(self.phase, event, pos);
        }
    }

    // --- writes ------------------------------------------------------------

    /// `Level.setBlock(pos, state, flags)`.
    pub fn set_block(&mut self, pos: Pos, state: StateId, flags: i32) -> bool {
        self.set_block_rec(pos, state, flags, MAX_RECURSION)
    }

    /// `Level.setBlock(pos, state, flags, recursionLeft)`.
    pub fn set_block_rec(&mut self, pos: Pos, state: StateId, flags: i32, recursion: i32) -> bool {
        let Some(old) = self.chunk_set(pos, state, flags) else {
            return false;
        };
        if self.get(pos) != state {
            return true;
        }
        if flags & flags::NEIGHBORS != 0 {
            let old_block = self.block(old);
            self.update_neighbors_at(pos, old_block);
            if blocks::has_analog_output(self, state) {
                self.update_neighbour_for_output_signal(pos, self.block(state));
            }
        }
        if flags & flags::KNOWN_SHAPE == 0 && recursion > 0 {
            let f = flags & !(flags::NEIGHBORS | flags::SUPPRESS_DROPS);
            blocks::update_indirect_neighbour_shapes(self, pos, old, f, recursion - 1);
            self.update_neighbour_shapes(pos, state, f, recursion - 1);
            blocks::update_indirect_neighbour_shapes(self, pos, state, f, recursion - 1);
        }
        true
    }

    /// `LevelChunk.setBlockState`: the write itself, block entity
    /// bookkeeping, `affectNeighborsAfterRemoval` and `onPlace`. Returns the
    /// previous state, or `None` if nothing changed.
    fn chunk_set(&mut self, pos: Pos, state: StateId, flags: i32) -> Option<StateId> {
        let old = self.get(pos);
        if old == state {
            return None;
        }
        if let Some(r) = self.recorder.as_mut() {
            r.record(self.phase, Event::Set(self.reg.format(state)), pos);
        }
        self.blocks.set(pos, state);
        let block_changed = self.block(old) != self.block(state);
        let moved_by_piston = flags & flags::MOVE_BY_PISTON != 0;
        if block_changed && blocks::has_block_entity(self.kind(old)) {
            self.remove_block_entity(pos);
        }
        if block_changed && (flags & flags::NEIGHBORS != 0 || moved_by_piston) {
            blocks::affect_neighbors_after_removal(self, pos, old, moved_by_piston);
        }
        if self.block(self.get(pos)) != self.block(state) {
            return None;
        }
        if flags & flags::SKIP_ON_PLACE == 0 {
            blocks::on_place(self, pos, state, old, moved_by_piston);
        }
        if blocks::has_block_entity(self.kind(state))
            && !self.block_entities.contains_key(&pos)
            && let Some(be) = blocks::new_block_entity(self, pos, state)
        {
            self.add_block_entity(pos, be);
        }
        Some(old)
    }

    /// `Level.destroyBlock(pos, drop)`. Drops are not simulated.
    pub fn destroy_block(&mut self, pos: Pos, recursion: i32) -> bool {
        let state = self.get(pos);
        if self.is_air(state) {
            return false;
        }
        self.set_block_rec(pos, self.air, flags::ALL, recursion)
    }

    pub fn remove_block(&mut self, pos: Pos) -> bool {
        self.set_block(pos, self.air, flags::ALL)
    }

    // --- block entities ----------------------------------------------------

    /// `Level.setBlockEntity`, with `LevelChunk.updateBlockEntityTicker`.
    pub fn add_block_entity(&mut self, pos: Pos, be: blocks::BlockEntity) {
        let ticks = be.ticks();
        self.block_entities.insert(pos, be);
        if !ticks {
            self.unbind_ticker(pos);
        } else if !self.ticker_at.contains_key(&pos) {
            let id = self.ticker_slots.len();
            self.ticker_slots.push((pos, true));
            self.ticker_at.insert(pos, id);
            if self.ticking_block_entities {
                self.pending_tickers.push(id);
            } else {
                self.tickers.push(id);
            }
        }
    }

    pub fn remove_block_entity(&mut self, pos: Pos) {
        self.block_entities.remove(&pos);
        self.unbind_ticker(pos);
    }

    fn unbind_ticker(&mut self, pos: Pos) {
        if let Some(id) = self.ticker_at.remove(&pos) {
            self.ticker_slots[id].1 = false;
        }
    }

    /// `ServerLevel.isHandlingTick`: true while block ticks and block
    /// events run.
    pub fn handling_tick(&self) -> bool {
        matches!(self.phase, Phase::BlockTicks | Phase::BlockEvents)
    }

    // --- neighbor and shape updates ------------------------------------------

    /// `Level.updateNeighborsAt(pos, block)`.
    pub fn update_neighbors_at(&mut self, pos: Pos, block: BlockId) {
        self.add_and_run(Update::Multi { source: pos, block, skip: None, idx: 0 });
    }

    /// `Level.updateNeighborsAtExceptFromFacing(pos, block, skip)`.
    pub fn update_neighbors_except(&mut self, pos: Pos, block: BlockId, skip: Dir) {
        let idx = if UPDATE_ORDER[0] == skip { 1 } else { 0 };
        self.add_and_run(Update::Multi { source: pos, block, skip: Some(skip), idx });
    }

    /// `Level.neighborChanged(pos, block)`: one neighbor-changed update.
    pub fn neighbor_changed(&mut self, pos: Pos, block: BlockId) {
        self.add_and_run(Update::Simple { pos, block, moved_by_piston: false });
    }

    /// `BlockStateBase.updateNeighbourShapes`: tell each neighbor of `pos`,
    /// in `UPDATE_SHAPE_ORDER`, that `state` is now beside it.
    pub fn update_neighbour_shapes(&mut self, pos: Pos, state: StateId, flags: i32, recursion: i32) {
        for dir in UPDATE_SHAPE_ORDER {
            self.neighbor_shape_changed(dir.opposite(), pos.offset(dir), pos, state, flags, recursion);
        }
    }

    /// `Level.neighborShapeChanged`: the block at `pos` hears that the block
    /// at `neighbor`, on its `dir` side, is now `neighbor_state`.
    pub fn neighbor_shape_changed(
        &mut self,
        dir: Dir,
        pos: Pos,
        neighbor: Pos,
        neighbor_state: StateId,
        flags: i32,
        recursion: i32,
    ) {
        self.add_and_run(Update::Shape { dir, pos, neighbor, neighbor_state, flags, recursion });
    }

    /// `Level.updateNeighbourForOutputSignal`: comparators are not in phase 1.
    pub fn update_neighbour_for_output_signal(&mut self, _pos: Pos, _block: BlockId) {}

    /// `CollectingNeighborUpdater.addAndRun`.
    fn add_and_run(&mut self, update: Update) {
        let nested = self.update_count > 0;
        self.update_count += 1;
        if nested {
            self.added_this_layer.push(update);
        } else {
            self.stack.push(update);
            self.run_updates();
        }
    }

    /// `CollectingNeighborUpdater.runUpdates`.
    fn run_updates(&mut self) {
        while !self.stack.is_empty() || !self.added_this_layer.is_empty() {
            while let Some(u) = self.added_this_layer.pop() {
                self.stack.push(u);
            }
            loop {
                if !self.added_this_layer.is_empty() {
                    break;
                }
                let Some(top) = self.stack.last_mut() else { break };
                let (step, more) = next_step(top);
                if !more {
                    self.stack.pop();
                }
                if let Some(step) = step {
                    self.execute(step);
                }
                if !more {
                    break;
                }
            }
        }
        self.update_count = 0;
    }

    fn execute(&mut self, step: Step) {
        match step {
            Step::Neighbor { pos, block, moved_by_piston } => {
                let state = self.get(pos);
                self.record(Event::Neighbor, pos);
                blocks::neighbor_changed(self, pos, state, block, moved_by_piston);
            }
            Step::Shape { dir, pos, neighbor, neighbor_state, flags: f, recursion } => {
                let state = self.get(pos);
                if f & flags::SKIP_SHAPE_UPDATE_ON_WIRE != 0 && self.kind(state) == Kind::RedstoneWire {
                    return;
                }
                let new = self.update_shape(pos, state, dir, neighbor, neighbor_state);
                self.update_or_destroy(state, new, pos, f, recursion);
            }
        }
    }

    /// `BlockStateBase.updateShape`, recorded as the game's hook sees it.
    pub fn update_shape(
        &mut self,
        pos: Pos,
        state: StateId,
        dir: Dir,
        neighbor: Pos,
        neighbor_state: StateId,
    ) -> StateId {
        self.record(Event::Shape, pos);
        blocks::update_shape(self, pos, state, dir, neighbor, neighbor_state)
    }

    /// `Block.updateOrDestroy`.
    pub fn update_or_destroy(&mut self, old: StateId, new: StateId, pos: Pos, f: i32, recursion: i32) {
        if new != old {
            if self.is_air(new) {
                self.destroy_block(pos, recursion);
            } else {
                self.set_block_rec(pos, new, f & !flags::SUPPRESS_DROPS, recursion);
            }
        }
    }

    /// `Block.updateFromNeighbourShapes`: the state `state` would take at
    /// `pos` given everything around it.
    pub fn update_from_neighbour_shapes(&mut self, state: StateId, pos: Pos) -> StateId {
        let mut s = state;
        for dir in UPDATE_SHAPE_ORDER {
            let n = pos.offset(dir);
            let ns = self.get(n);
            s = self.update_shape(pos, s, dir, n, ns);
        }
        s
    }

    // --- scheduled ticks ---------------------------------------------------

    /// `Level.scheduleTick(pos, block, delay, priority)`.
    pub fn schedule_tick(&mut self, pos: Pos, block: BlockId, delay: i32, priority: Priority) {
        let trigger = self.game_time + delay as i64;
        let sub = self.next_sub_tick;
        self.next_sub_tick += 1;
        self.record(Event::Schedule { delay: trigger - self.game_time, priority }, pos);
        if self.pending_keys.insert((pos, block)) {
            self.pending_ticks.push(ScheduledTick { block, pos, trigger, priority, sub });
        }
    }

    /// `LevelTicks.hasScheduledTick`.
    pub fn has_scheduled_tick(&self, pos: Pos, block: BlockId) -> bool {
        self.pending_keys.contains(&(pos, block))
    }

    /// `LevelTicks.willTickThisTick`.
    pub fn will_tick_this_tick(&self, pos: Pos, block: BlockId) -> bool {
        self.running_ticks.contains(&(pos, block))
    }

    fn run_block_ticks(&mut self) {
        let now = self.game_time;
        let mut due: Vec<ScheduledTick> = Vec::new();
        self.pending_ticks.retain(|t| {
            if t.trigger <= now {
                due.push(*t);
                false
            } else {
                true
            }
        });
        due.sort_by_key(|t| (t.trigger, t.priority, t.sub));
        for t in &due {
            self.pending_keys.remove(&(t.pos, t.block));
            self.running_ticks.insert((t.pos, t.block));
        }
        for t in due {
            self.running_ticks.remove(&(t.pos, t.block));
            self.record(Event::Tick, t.pos);
            let state = self.get(t.pos);
            if self.block(state) == t.block {
                blocks::tick(self, t.pos, state);
            }
        }
    }

    // --- block events ------------------------------------------------------

    /// `ServerLevel.blockEvent`.
    pub fn block_event(&mut self, pos: Pos, block: BlockId, a: i32, b: i32) {
        self.record(Event::BlockEventQueued, pos);
        let e = BlockEvent { pos, block, a, b };
        if !self.events.contains(&e) {
            self.events.push_back(e);
        }
    }

    fn run_block_events(&mut self) {
        while let Some(e) = self.events.pop_front() {
            self.record(Event::BlockEvent { a: e.a, b: e.b }, e.pos);
            let state = self.get(e.pos);
            if self.block(state) == e.block {
                blocks::trigger_event(self, e.pos, state, e.a, e.b);
            }
        }
    }

    // --- block entities ----------------------------------------------------

    /// `Level.tickBlockEntities`.
    fn tick_block_entities(&mut self) {
        self.ticking_block_entities = true;
        let pending = std::mem::take(&mut self.pending_tickers);
        self.tickers.extend(pending);
        let mut i = 0;
        while i < self.tickers.len() {
            let (pos, bound) = self.ticker_slots[self.tickers[i]];
            if bound {
                blocks::tick_block_entity(self, pos);
                i += 1;
            } else {
                self.tickers.remove(i);
            }
        }
        self.ticking_block_entities = false;
    }

    // --- the tick loop -----------------------------------------------------

    /// One world tick (`ServerLevel.tick`), after player input for the tick
    /// has been applied.
    pub fn tick(&mut self) {
        self.game_time += 1;
        self.phase = Phase::BlockTicks;
        self.run_block_ticks();
        self.phase = Phase::BlockEvents;
        self.run_block_events();
        self.phase = Phase::BlockEntities;
        self.tick_block_entities();
        self.phase = Phase::Input;
    }

    // --- signals -----------------------------------------------------------

    /// `Level.getSignal(pos, dir)`: the signal the block at `pos` gives
    /// toward `dir` (the side of `pos` that faces the asker is `dir`'s
    /// opposite), including power conducted through it.
    pub fn get_signal(&self, pos: Pos, dir: Dir) -> i32 {
        let state = self.get(pos);
        let s = blocks::get_signal(self, pos, state, dir);
        if blocks::is_conductor(self, pos, state) { s.max(self.get_direct_signal_to(pos)) } else { s }
    }

    /// `Level.hasNeighborSignal`.
    pub fn has_neighbor_signal(&self, pos: Pos) -> bool {
        Dir::ALL.iter().any(|&d| self.get_signal(pos.offset(d), d) > 0)
    }

    /// `Level.getBestNeighborSignal`.
    pub fn best_neighbor_signal(&self, pos: Pos) -> i32 {
        let mut best = 0;
        for d in Dir::ALL {
            let s = self.get_signal(pos.offset(d), d);
            if s >= 15 {
                return 15;
            }
            best = best.max(s);
        }
        best
    }

    /// `Level.getDirectSignal(pos, dir)`.
    pub fn get_direct_signal(&self, pos: Pos, dir: Dir) -> i32 {
        let state = self.get(pos);
        blocks::get_direct_signal(self, pos, state, dir)
    }

    /// `Level.getDirectSignalTo(pos)`: the strongest strong power into `pos`.
    pub fn get_direct_signal_to(&self, pos: Pos) -> i32 {
        let mut best = 0;
        for d in Dir::ALL {
            best = best.max(self.get_direct_signal(pos.offset(d), d));
            if best >= 15 {
                return best;
            }
        }
        best
    }

    /// `Level.hasSignal(pos, dir)`.
    pub fn has_signal(&self, pos: Pos, dir: Dir) -> bool {
        self.get_signal(pos, dir) > 0
    }

    pub fn non_air(&self) -> impl Iterator<Item = (Pos, StateId)> + '_ {
        self.blocks.non_air()
    }
}

/// One notification taken off an update entry.
enum Step {
    Neighbor { pos: Pos, block: BlockId, moved_by_piston: bool },
    Shape { dir: Dir, pos: Pos, neighbor: Pos, neighbor_state: StateId, flags: i32, recursion: i32 },
}

/// `NeighborUpdates.runNext`: the next notification and whether more remain.
fn next_step(u: &mut Update) -> (Option<Step>, bool) {
    match u {
        Update::Multi { source, block, skip, idx } => {
            let pos = source.offset(UPDATE_ORDER[*idx]);
            *idx += 1;
            if *idx < 6 && Some(UPDATE_ORDER[*idx]) == *skip {
                *idx += 1;
            }
            (Some(Step::Neighbor { pos, block: *block, moved_by_piston: false }), *idx < 6)
        }
        Update::Simple { pos, block, moved_by_piston } => {
            (Some(Step::Neighbor { pos: *pos, block: *block, moved_by_piston: *moved_by_piston }), false)
        }
        Update::Shape { dir, pos, neighbor, neighbor_state, flags, recursion } => (
            Some(Step::Shape {
                dir: *dir,
                pos: *pos,
                neighbor: *neighbor,
                neighbor_state: *neighbor_state,
                flags: *flags,
                recursion: *recursion,
            }),
            false,
        ),
    }
}
