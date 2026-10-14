//! An interactive simulation for an editor: build, click, step and rewind.
//!
//! A [`Session`] holds a level the editor can step forward one tick at a
//! time, scrub back to any earlier tick, and branch off from there. Edits
//! made between ticks land the way the capture harness applies stimuli:
//! before the next tick runs, recorded in the input phase of that tick.
//!
//! Every event the engine records, anywhere in the world, is kept with the
//! tick it belongs to, which is what the editor's update viewer and
//! timelines read. Positions are absolute.
//!
//! A long run has to fit in a browser tab, so the session doesn't keep a
//! copy of the world after every tick. It keeps one now and then (a
//! checkpoint: at most [`CHECKPOINT_TICKS`] ticks apart, closer when ticks
//! are busy), every edit with the tick it was made at, and every tick's
//! events packed small (see [`crate::record::Packer`]). Any other tick is
//! rebuilt by running the ticks after the checkpoint before it again with
//! the same edits, which gives the same world because the engine is
//! deterministic.

use std::cell::RefCell;

use anyhow::{Result, bail};
use tickwright_trace::fixture::{ComparatorOutput, Container, ItemSlot};
use tickwright_trace::{Entry, ResolvedFixture};

use crate::blocks::{self, Kind};
use crate::fxhash::FxHashMap;
use crate::level::{Level, MAX_RECURSION, flags};
use crate::physics::Vec3;
use crate::pos::{Dir, Pos};
use crate::record::{Packer, Recorder};
use crate::registry::StateId;
use crate::rules::Rules;

/// The most ticks between checkpoints. Rewinding to a tick, or reading
/// one, runs fewer than this many ticks again: at most about 11 ms on the
/// benchmark build in a browser.
pub const CHECKPOINT_TICKS: u32 = 32;

/// Packed event bytes after which a checkpoint comes sooner. The benchmark
/// packs about 3.4 KB of events a tick, so it checkpoints every 32 ticks;
/// a build more than twice as busy checkpoints more often, keeping rewinds
/// as quick at the cost of more copies. A build packing this much in a
/// single tick is copied after every tick, as sessions once did for all.
pub const CHECKPOINT_BYTES: usize = 256 * 1024;

/// How a block is written. Either way a block entity it replaces goes the
/// way `/setblock` removes one, without its side effects: a container's
/// items, a lectern's book and a jukebox's disc go with it instead of
/// spilling out (`Block.UPDATE_SKIP_BLOCK_ENTITY_SIDEEFFECTS`). A fixture
/// stimulus does the same with `"discard_contents": true`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Placement {
    /// As the capture harness places fixture blocks: no neighbor updates,
    /// and the state is taken as given. `onPlace` still runs.
    Quiet,
    /// A normal write that updates neighbors, like a stimulus `set`.
    Updates,
}

/// A pending scheduled tick, for the debug view.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PendingTick {
    pub pos: Pos,
    pub block: String,
    /// The tick it is due to run in.
    pub due: u32,
    pub priority: i32,
}

/// An entity, for the editor's view.
#[derive(Clone, Debug, PartialEq)]
pub struct EntityView {
    pub id: i32,
    /// Its type, such as `minecraft:item`.
    pub kind: String,
    pub pos: [f64; 3],
    /// What else the trace records about it: an item's stack
    /// (`minecraft:stone*3`), a TNT's `fuse=N`, a falling block's state.
    pub detail: Option<String>,
}

/// What a position reads as, for the power views.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Power {
    /// The strongest signal reaching the block from its neighbors.
    pub weak: i32,
    /// The strongest signal driven into the block (strong power).
    pub strong: i32,
}

/// An edit, kept so the ticks after it can be run again.
#[derive(Clone, Debug)]
enum Input {
    Set {
        pos: Pos,
        state: StateId,
        flags: i32,
    },
    Place {
        item: String,
        click: blocks::placement::Click,
    },
    /// A break, with the flags it adds to the write.
    Destroy {
        pos: Pos,
        extra: i32,
    },
    Use(Pos),
    Page {
        pos: Pos,
        page: i32,
    },
    Time(i64),
    Item {
        pos: Pos,
        slot: usize,
        id: String,
        count: i32,
    },
    Summon {
        entity: String,
        at: Vec3,
        motion: Option<Vec3>,
        nbt: Option<String>,
    },
}

/// The flag a write that removes block entities as `/setblock` does adds.
fn discard(discard_contents: bool) -> i32 {
    if discard_contents { flags::SKIP_BLOCK_ENTITY_SIDEEFFECTS } else { 0 }
}

/// Make an edit. Run again, it changes the world exactly as it did the
/// first time, whatever it answered then.
fn apply(level: &mut Level, input: &Input) -> Result<bool> {
    match input {
        Input::Set { pos, state, flags } => {
            level.set_block(*pos, *state, *flags);
        }
        Input::Place { item, click } => {
            level.new_clicking_player();
            return blocks::placement::place(level, item, *click);
        }
        Input::Destroy { pos, extra } => {
            level.destroy_block_with(*pos, false, *extra, MAX_RECURSION);
        }
        Input::Use(pos) => {
            level.new_clicking_player();
            let state = level.get(*pos);
            blocks::use_without_item(level, *pos, state);
        }
        Input::Page { pos, page } => blocks::lectern::set_page(level, *pos, *page),
        Input::Time(time) => level.day_time = *time,
        Input::Item { pos, slot, id, count } => {
            blocks::container::set_item(level, *pos, *slot, blocks::container::Stack::new(id, *count))
        }
        Input::Summon { entity, at, motion, nbt } => {
            crate::entity::summon(level, entity, *at, *motion, nbt.as_deref())?
        }
    }
    Ok(true)
}

/// Run the ticks after `from` up to and including `to`, each after the
/// edits made before it.
fn replay(level: &mut Level, from: u32, to: u32, inputs: &[(u32, Input)]) {
    let mut i = inputs.partition_point(|(u, _)| *u < from);
    for t in from..to {
        while let Some((u, input)) = inputs.get(i)
            && *u == t
        {
            let _ = apply(level, input);
            i += 1;
        }
        level.tick();
    }
}

fn sorted_blocks(level: &Level) -> Vec<(Pos, String)> {
    let mut out: Vec<(Pos, String)> = level.non_air().map(|(p, s)| (p, level.reg.format(s))).collect();
    out.sort_by_key(|(p, _)| (p.y, p.z, p.x));
    out
}

/// Everything a level holds that the editor reads or a later tick depends
/// on, as text, to check a level rebuilt by replaying against the one it
/// should equal.
#[cfg(any(test, debug_assertions))]
fn fingerprint(level: &Level) -> String {
    let mut block_entities: Vec<_> = level.block_entities.iter().map(|(p, e)| format!("{p:?} {e:?}")).collect();
    block_entities.sort();
    format!(
        "time {} {} random {} ids {} {} unsupported {:?} toggles {:?}\nblocks {:?}\npending {:?}\nentities {:?}\nblock entities {block_entities:?}",
        level.game_time,
        level.day_time,
        level.random.internal(),
        level.entity_counter,
        level.unique_seeds,
        level.unsupported,
        level.torch_toggles,
        {
            let mut blocks: Vec<(Pos, StateId)> = level.non_air().collect();
            blocks.sort();
            blocks
        },
        level.pending_block_ticks(),
        level.entities,
    )
}

/// Bytes of packed events are kept in chunks of this size, so a long run
/// never copies them all to grow.
const CHUNK: usize = 256 * 1024;

/// Every tick's events, packed.
struct Log {
    chunks: Vec<Vec<u8>>,
    /// Where each tick's events are: chunk, start and end. Tick 0 (the
    /// build) has none.
    spans: Vec<(u32, u32, u32)>,
    /// Each block's changes as `(tick, state)`, oldest first.
    changes: FxHashMap<Pos, Vec<(u32, StateId)>>,
}

impl Log {
    fn new() -> Log {
        Log { chunks: Vec::new(), spans: vec![(0, 0, 0)], changes: FxHashMap::default() }
    }

    fn last_tick(&self) -> u32 {
        (self.spans.len() - 1) as u32
    }

    /// Tick `t`'s packed events.
    fn of(&self, t: u32) -> &[u8] {
        match self.spans.get(t as usize) {
            Some(&(c, start, end)) if end > start => &self.chunks[c as usize][start as usize..end as usize],
            _ => &[],
        }
    }

    /// Bytes of events in the ticks after `t`.
    fn bytes_after(&self, t: u32) -> usize {
        self.spans[t as usize + 1..].iter().map(|&(_, start, end)| (end - start) as usize).sum()
    }

    /// Add the next tick's events.
    fn push(&mut self, events: &Packer) {
        let t = self.spans.len() as u32;
        let bytes = &events.bytes;
        if self.chunks.last().is_none_or(|c| c.capacity() - c.len() < bytes.len()) {
            self.chunks.push(Vec::with_capacity(CHUNK.max(bytes.len())));
        }
        let c = self.chunks.len() - 1;
        let chunk = &mut self.chunks[c];
        let start = chunk.len();
        chunk.extend_from_slice(bytes);
        self.spans.push((c as u32, start as u32, chunk.len() as u32));
        for &(pos, state) in &events.sets {
            self.changes.entry(pos).or_default().push((t, state));
        }
    }

    /// Drop the ticks after `t`.
    fn truncate(&mut self, t: u32) {
        self.spans.truncate(t as usize + 1);
        let (c, _, end) = self.spans[t as usize];
        self.chunks.truncate(c as usize + 1);
        if let Some(chunk) = self.chunks.get_mut(c as usize) {
            chunk.truncate(end as usize);
        }
        for changes in self.changes.values_mut() {
            changes.truncate(changes.partition_point(|&(u, _)| u <= t));
        }
        self.changes.retain(|_, changes| !changes.is_empty());
    }
}

/// One editor world: the level now, and enough to rebuild any tick since
/// the build.
pub struct Session {
    level: Level,
    /// Copies of the level just after some ticks, oldest first, without a
    /// recorder: always tick 0 (the build), then at most
    /// [`CHECKPOINT_TICKS`] apart.
    checkpoints: Vec<(u32, Level)>,
    /// Every edit with the tick it was made after, oldest first. Edits
    /// made after tick `t` land before tick `t + 1`.
    inputs: Vec<(u32, Input)>,
    log: Log,
    /// The tick last rebuilt to read the past, so reading tick after tick
    /// runs one tick each time rather than all the ticks from a checkpoint.
    scratch: RefCell<Option<(u32, Level)>>,
    /// The checkpoint spacing: the most ticks, and the most event bytes.
    every: u32,
    budget: usize,
}

impl Session {
    /// An empty world with the rules of `version`.
    pub fn new(version: &str) -> Result<Session> {
        Ok(Session::from_level(Level::new(Rules::for_version(version)?)))
    }

    /// A world with a resolved fixture placed the way the harness places it.
    pub fn from_fixture(fixture: &ResolvedFixture, version: &str) -> Result<Session> {
        Ok(Session::from_level(crate::run::place(fixture, Rules::for_version(version)?)?))
    }

    fn from_level(mut level: Level) -> Session {
        level.recorder = None;
        let build = level.clone();
        level.recorder = Some(Recorder::Packed(Packer::default()));
        Session {
            level,
            checkpoints: vec![(0, build)],
            inputs: Vec::new(),
            log: Log::new(),
            scratch: RefCell::new(None),
            every: CHECKPOINT_TICKS,
            budget: CHECKPOINT_BYTES,
        }
    }

    /// The last tick that has run; 0 before the first.
    pub fn tick(&self) -> u32 {
        self.log.last_tick()
    }

    /// The events recorded since the last tick ran: the edits waiting for
    /// the next.
    fn packer(&self) -> &Packer {
        match &self.level.recorder {
            Some(Recorder::Packed(p)) => p,
            _ => unreachable!("a session packs its events"),
        }
    }

    /// Whether anything was edited since the last tick ran.
    fn edited(&self) -> bool {
        self.inputs.last().is_some_and(|(u, _)| *u == self.tick())
    }

    // --- editing -------------------------------------------------------------

    /// Keep an edit and make it.
    fn edit(&mut self, input: Input) -> Result<bool> {
        let answer = apply(&mut self.level, &input);
        self.inputs.push((self.tick(), input));
        answer
    }

    /// Write `state` (such as `repeater[delay=2,facing=north]`) at `pos`.
    /// Blocks the engine does not simulate are refused by name.
    pub fn set_block(&mut self, pos: Pos, state: &str, placement: Placement) -> Result<()> {
        self.set_block_as(pos, state, placement, true)
    }

    /// [`Session::set_block`], choosing whether a block entity it replaces
    /// goes as `/setblock` removes it (`discard_contents`), or spills what
    /// it holds as breaking it in the game does, as a fixture stimulus can.
    pub fn set_block_as(&mut self, pos: Pos, state: &str, placement: Placement, discard_contents: bool) -> Result<()> {
        let id = self.level.reg.parse(state)?;
        if self.level.kind(id) == Kind::Other {
            bail!("{} is not supported by the engine yet", self.level.reg.name(id));
        }
        let flags = match placement {
            Placement::Quiet => flags::CLIENTS | flags::KNOWN_SHAPE,
            Placement::Updates => flags::ALL,
        } | discard(discard_contents);
        self.edit(Input::Set { pos, state: id, flags })?;
        Ok(())
    }

    /// Right-click a face of a block holding `item`, as a player would, so
    /// the block's placement rules pick its state. Returns whether a block
    /// was placed.
    pub fn place(&mut self, item: &str, click: blocks::placement::Click) -> Result<bool> {
        self.edit(Input::Place { item: item.to_string(), click })
    }

    /// Where `place` would put a block and the state it would set, such as
    /// `minecraft:repeater[delay=1,facing=west,…]`, without changing the
    /// world. None when the click would place nothing.
    pub fn preview_place(&self, item: &str, click: blocks::placement::Click) -> Result<Option<(Pos, String)>> {
        let planned = blocks::placement::preview(&self.level, item, click)?;
        Ok(planned.map(|(pos, state)| (pos, self.level.reg.format(state))))
    }

    /// Break the block at `pos` without drops, with normal updates. A block
    /// entity goes without its side effects, as with [`Session::set_block`].
    pub fn destroy(&mut self, pos: Pos) {
        self.destroy_as(pos, true);
    }

    /// [`Session::destroy`], choosing as [`Session::set_block_as`] does.
    pub fn destroy_as(&mut self, pos: Pos, discard_contents: bool) {
        let _ = self.edit(Input::Destroy { pos, extra: discard(discard_contents) });
    }

    /// Right-click the block at `pos` with an empty hand.
    pub fn use_block(&mut self, pos: Pos) {
        let _ = self.edit(Input::Use(pos));
    }

    /// Turn the book on the lectern at `pos` to `page`, as its menu does.
    pub fn turn_page(&mut self, pos: Pos, page: i32) {
        let _ = self.edit(Input::Page { pos, page });
    }

    /// Set the world clock's day time, as `/time set` does.
    pub fn set_day_time(&mut self, time: i64) {
        let _ = self.edit(Input::Time(time));
    }

    /// Put `count` of item `id` in slot `slot` of the container at `pos`.
    pub fn set_item(&mut self, pos: Pos, slot: usize, id: &str, count: i32) -> Result<()> {
        let size = blocks::container::size(&self.level, pos);
        if size == 0 {
            bail!("no container at {pos:?}");
        }
        if slot >= size {
            bail!("slot {slot} is past the container's {size} slots");
        }
        let id = if id.contains(':') { id.to_string() } else { format!("minecraft:{id}") };
        if self.level.kind_at(pos) == Kind::Dispenser && !blocks::dispenser::supported_item(&id) {
            bail!("dispensing {id} is not supported by the engine yet");
        }
        self.edit(Input::Item { pos, slot, id, count })?;
        Ok(())
    }

    /// Summon an entity as `/summon <entity> x y z {nbt}` does, optionally
    /// with a starting motion. Entities the engine can't simulate yet, and
    /// data it doesn't understand, are refused by name.
    pub fn summon(&mut self, entity: &str, at: [f64; 3], motion: Option<[f64; 3]>, nbt: Option<&str>) -> Result<()> {
        let at = Vec3::new(at[0], at[1], at[2]);
        let motion = motion.map(|m| Vec3::new(m[0], m[1], m[2]));
        let input = Input::Summon { entity: entity.to_string(), at, motion, nbt: nbt.map(str::to_string) };
        self.edit(input)?;
        Ok(())
    }

    // --- running -------------------------------------------------------------

    /// Run one game tick. Returns whether anything happened in it: whether
    /// it has any events, the input before it included.
    pub fn step(&mut self) -> bool {
        self.level.tick();
        let Some(Recorder::Packed(events)) = self.level.recorder.as_mut() else {
            unreachable!("a session packs its events")
        };
        let busy = events.count > 0;
        self.log.push(events);
        events.clear();
        let t = self.tick();
        let &(last, _) = self.checkpoints.last().unwrap();
        if t - last >= self.every || self.log.bytes_after(last) >= self.budget {
            let recorder = self.level.recorder.take();
            #[cfg(any(test, debug_assertions))]
            self.check_replay(t);
            self.checkpoints.push((t, self.level.clone()));
            self.level.recorder = recorder;
        }
        busy
    }

    /// Check that the last checkpoint, run again with the edits kept, gives
    /// the world as it is after tick `t`: an edit that changed the level
    /// without going through [`Session::edit`] would be lost on rewinding.
    /// Only in tests and debug builds, as it runs the ticks twice.
    #[cfg(any(test, debug_assertions))]
    fn check_replay(&self, t: u32) {
        let (c, ref checkpoint) = *self.checkpoints.last().unwrap();
        let mut rebuilt = checkpoint.clone();
        replay(&mut rebuilt, c, t, &self.inputs);
        assert!(
            fingerprint(&rebuilt) == fingerprint(&self.level),
            "tick {t} run again from the checkpoint at tick {c} differs from the world: \
             something changed it without Session::edit"
        );
    }

    /// The first thing the run met that the engine can't simulate exactly,
    /// as a sentence such as "an entity in fire is not simulated
    /// by the engine yet". From that tick on the session may differ from the
    /// game. Rewinding to before it clears it.
    pub fn unsupported(&self) -> Option<String> {
        self.level.unsupported.as_ref().map(|what| format!("{what} is not simulated by the engine yet"))
    }

    /// The level just after tick `t`, before the edits made after it, run
    /// from the nearest checkpoint or the last tick rebuilt, whichever is
    /// closer. Everything up to `t` must be kept.
    fn rebuild(&self, t: u32) -> Level {
        let i = self.checkpoints.partition_point(|(c, _)| *c <= t) - 1;
        let (c, ref checkpoint) = self.checkpoints[i];
        let mut scratch = self.scratch.borrow_mut();
        let (from, mut level) = match scratch.take() {
            Some((s, level)) if (c..=t).contains(&s) => (s, level),
            other => {
                *scratch = other;
                (c, checkpoint.clone())
            }
        };
        replay(&mut level, from, t, &self.inputs);
        level
    }

    /// Go back to just after tick `t`, dropping everything after it. Edits
    /// made since the last step are dropped too.
    pub fn rewind(&mut self, t: u32) -> Result<()> {
        if t > self.tick() {
            bail!("tick {t} has not run yet; the last is {}", self.tick());
        }
        if t < self.tick() || self.edited() {
            self.inputs.truncate(self.inputs.partition_point(|(u, _)| *u < t));
            self.checkpoints.truncate(self.checkpoints.partition_point(|(c, _)| *c <= t));
            self.log.truncate(t);
            let scratch = self.scratch.get_mut();
            if scratch.as_ref().is_some_and(|(s, _)| *s > t) {
                *scratch = None;
            }
            self.level = self.rebuild(t);
            self.level.recorder = Some(Recorder::Packed(Packer::default()));
        }
        if let Some(Recorder::Packed(events)) = self.level.recorder.as_mut() {
            events.clear();
        }
        Ok(())
    }

    /// The events of tick `t`, or the edits waiting for the next tick when
    /// `t` is one past the last.
    pub fn events(&self, t: u32) -> Vec<Entry> {
        let bytes = if t == self.tick() + 1 { &self.packer().bytes[..] } else { self.log.of(t) };
        crate::record::unpack(bytes, t, self.level.reg)
    }

    /// Each change of the block at `pos` as `(tick, state)`, oldest first,
    /// including edits waiting for the next tick.
    pub fn history(&self, pos: Pos) -> Vec<(u32, String)> {
        let reg = self.level.reg;
        let kept = self.log.changes.get(&pos).map_or(&[][..], Vec::as_slice).iter().copied();
        let next = self.tick() + 1;
        let waiting = self.packer().sets.iter().filter(|(p, _)| *p == pos).map(|&(_, s)| (next, s));
        kept.chain(waiting).map(|(t, s)| (t, reg.format(s))).collect()
    }

    // --- reading -------------------------------------------------------------

    /// Every non-air block as `(pos, state)`, sorted by y, z, x.
    pub fn blocks(&self) -> Vec<(Pos, String)> {
        sorted_blocks(&self.level)
    }

    /// Every non-air block as it was just after tick `t` (tick 0 is the
    /// build), before any edits made after that tick. `None` when tick `t`
    /// has not run.
    pub fn blocks_at(&self, t: u32) -> Option<Vec<(Pos, String)>> {
        if t > self.tick() {
            return None;
        }
        if t == self.tick() && !self.edited() {
            return Some(sorted_blocks(&self.level));
        }
        if let Ok(i) = self.checkpoints.binary_search_by_key(&t, |(c, _)| *c) {
            return Some(sorted_blocks(&self.checkpoints[i].1));
        }
        let level = self.rebuild(t);
        let out = sorted_blocks(&level);
        *self.scratch.borrow_mut() = Some((t, level));
        Some(out)
    }

    pub fn block(&self, pos: Pos) -> String {
        self.level.reg.format(self.level.get(pos))
    }

    /// The entities in the world now, in the order they tick.
    pub fn entities(&self) -> Vec<EntityView> {
        self.level
            .entities
            .iter()
            .filter(|e| !e.removed)
            .map(|e| EntityView {
                id: e.id,
                kind: e.type_name().to_string(),
                pos: [e.pos.x, e.pos.y, e.pos.z],
                detail: e.detail(&self.level),
            })
            .collect()
    }

    /// The power the block at `pos` receives.
    pub fn power(&self, pos: Pos) -> Power {
        Power { weak: self.level.best_neighbor_signal(pos), strong: self.level.get_direct_signal_to(pos) }
    }

    /// The signal the block at `pos` gives out toward `dir`.
    pub fn signal(&self, pos: Pos, dir: Dir) -> i32 {
        self.level.get_signal(pos, dir)
    }

    /// Scheduled block ticks still to run, in the order they will run.
    pub fn pending_ticks(&self) -> Vec<PendingTick> {
        self.level
            .pending_block_ticks()
            .into_iter()
            .map(|(pos, block, due, priority)| PendingTick {
                pos,
                block: self.level.reg.block(block).name.clone(),
                due: due as u32,
                priority,
            })
            .collect()
    }

    /// The contents of the container at `pos`, as the trace writes them.
    pub fn items(&self, pos: Pos) -> Option<String> {
        blocks::container::items(&self.level, pos).map(|i| blocks::container::describe(i))
    }

    /// Every container that holds something, with its stacks by slot, in
    /// y, z, x order.
    pub fn containers(&self) -> Vec<Container> {
        let mut out: Vec<Container> = self
            .level
            .block_entities
            .keys()
            .filter_map(|&pos| {
                let items: Vec<ItemSlot> = blocks::container::items(&self.level, pos)?
                    .iter()
                    .enumerate()
                    .filter(|(_, s)| !s.is_empty())
                    .map(|(slot, s)| ItemSlot {
                        slot: slot as u32,
                        id: s.id.clone(),
                        count: s.count as u32,
                        components: s.data.as_ref().and_then(|d| d.components.clone()),
                        tag: s.data.as_ref().and_then(|d| d.tag.clone()),
                    })
                    .collect();
                (!items.is_empty()).then_some(Container { pos: [pos.x, pos.y, pos.z], items })
            })
            .collect();
        out.sort_by_key(|c| (c.pos[1], c.pos[2], c.pos[0]));
        out
    }

    /// Every comparator whose stored output isn't 0, in y, z, x order, so a
    /// saved build keeps what its comparators remember.
    pub fn comparators(&self) -> Vec<ComparatorOutput> {
        let mut out: Vec<ComparatorOutput> = self
            .level
            .block_entities
            .iter()
            .filter_map(|(pos, e)| match e {
                blocks::BlockEntity::Comparator(output) if *output != 0 => {
                    Some(ComparatorOutput { pos: [pos.x, pos.y, pos.z], output: *output })
                }
                _ => None,
            })
            .collect();
        out.sort_by_key(|c| (c.pos[1], c.pos[2], c.pos[0]));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_lever_lights_a_lamp_and_rewinding_undoes_it() {
        let mut s = Session::new("26.3").unwrap();
        s.set_block(Pos::new(0, 0, 0), "stone", Placement::Quiet).unwrap();
        s.set_block(Pos::new(0, 1, 0), "lever[face=floor,facing=north,powered=false]", Placement::Quiet).unwrap();
        s.set_block(Pos::new(1, 1, 0), "redstone_lamp[lit=false]", Placement::Quiet).unwrap();
        s.step();
        s.use_block(Pos::new(0, 1, 0));
        assert!(!s.events(2).is_empty(), "the click is waiting for tick 2");
        s.step();
        assert_eq!(s.block(Pos::new(1, 1, 0)), "minecraft:redstone_lamp[lit=true]");
        assert_eq!(s.power(Pos::new(1, 1, 0)).weak, 15);
        s.rewind(1).unwrap();
        assert_eq!(s.block(Pos::new(1, 1, 0)), "minecraft:redstone_lamp[lit=false]");
        assert_eq!(s.tick(), 1);
        let lamp = |blocks: Vec<(Pos, String)>| blocks.into_iter().find(|(p, _)| *p == Pos::new(1, 1, 0)).unwrap().1;
        s.use_block(Pos::new(0, 1, 0));
        s.step();
        assert_eq!(lamp(s.blocks_at(1).unwrap()), "minecraft:redstone_lamp[lit=false]");
        assert_eq!(lamp(s.blocks_at(2).unwrap()), "minecraft:redstone_lamp[lit=true]");
        assert!(s.blocks_at(3).is_none());
    }

    #[test]
    fn a_summoned_item_falls_and_rewinding_brings_it_back() {
        let mut s = Session::new("26.3").unwrap();
        s.set_block(Pos::new(0, 0, 0), "stone", Placement::Quiet).unwrap();
        s.summon("item", [0.5, 3.0, 0.5], None, Some(r#"{Item:{id:"minecraft:redstone",count:2}}"#)).unwrap();
        s.step();
        let e = s.entities();
        assert_eq!(e.len(), 1);
        assert_eq!((e[0].id, e[0].kind.as_str()), (1, "minecraft:item"));
        assert_eq!(e[0].detail.as_deref(), Some("minecraft:redstone*2"));
        assert!(e[0].pos[1] < 3.0, "it fell");
        for _ in 0..40 {
            s.step();
        }
        assert_eq!(s.entities()[0].pos[1], 1.0, "it rests on the stone");
        s.rewind(0).unwrap();
        assert!(s.entities().is_empty(), "the summon was input to tick 1");
        assert!(s.summon("zombie", [0.5, 1.0, 0.5], None, None).is_err());
    }

    #[test]
    fn an_item_that_primes_tnt_from_a_plate_stays_an_item() {
        let mut s = Session::new("26.3").unwrap();
        s.set_block(Pos::new(0, 0, 0), "stone", Placement::Quiet).unwrap();
        s.set_block(Pos::new(1, 0, 0), "stone", Placement::Quiet).unwrap();
        s.set_block(Pos::new(0, 1, 0), "oak_pressure_plate[powered=false]", Placement::Quiet).unwrap();
        s.set_block(Pos::new(1, 1, 0), "tnt[unstable=false]", Placement::Quiet).unwrap();
        s.summon("item", [0.5, 2.0, 0.5], None, Some(r#"{Item:{id:"minecraft:stone",count:1}}"#)).unwrap();
        for _ in 0..20 {
            s.step();
        }
        assert_eq!(s.block(Pos::new(0, 1, 0)), "minecraft:oak_pressure_plate[powered=true]");
        assert_eq!(s.block(Pos::new(1, 1, 0)), "minecraft:air");
        let e = s.entities();
        let kinds: Vec<(i32, &str)> = e.iter().map(|e| (e.id, e.kind.as_str())).collect();
        assert_eq!(
            kinds,
            [(1, "minecraft:item"), (2, "minecraft:tnt")],
            "the item stays first, the TNT it lit after it"
        );
        assert_eq!(e[0].detail.as_deref(), Some("minecraft:stone*1"));
        assert_eq!(e[0].pos[1], 1.0, "the item lies on the plate");
    }

    /// The entities' ids and types in the order they tick, which must be
    /// the order they were added.
    fn tick_order(s: &Session) -> Vec<(i32, String)> {
        let e: Vec<(i32, String)> = s.entities().into_iter().map(|e| (e.id, e.kind)).collect();
        assert!(e.windows(2).all(|w| w[0].0 < w[1].0), "entities tick in the order they were added: {e:?}");
        e
    }

    fn stone_under(s: &mut Session, blocks: &[Pos]) {
        for p in blocks {
            s.set_block(p.below(), "stone", Placement::Quiet).unwrap();
        }
    }

    #[test]
    fn a_minecart_that_primes_tnt_from_a_detector_rail_stays_a_minecart() {
        let mut s = Session::new("26.3").unwrap();
        stone_under(&mut s, &[Pos::new(0, 1, 0), Pos::new(0, 1, 1)]);
        s.set_block(Pos::new(0, 1, 0), "detector_rail[powered=false,shape=east_west]", Placement::Quiet).unwrap();
        s.set_block(Pos::new(0, 1, 1), "tnt[unstable=false]", Placement::Quiet).unwrap();
        s.summon("minecart", [0.5, 1.0625, 0.5], None, None).unwrap();
        for _ in 0..5 {
            s.step();
        }
        assert_eq!(s.block(Pos::new(0, 1, 1)), "minecraft:air", "the rail lit the TNT");
        assert_eq!(tick_order(&s), [(1, "minecraft:minecart".into()), (2, "minecraft:tnt".into())]);
    }

    #[test]
    fn an_item_a_piston_pushes_onto_a_plate_by_tnt_keeps_its_place() {
        let mut s = Session::new("26.3").unwrap();
        let row: Vec<Pos> = (0..5).flat_map(|x| [Pos::new(x, 1, 0), Pos::new(x, 1, 1)]).collect();
        stone_under(&mut s, &row);
        s.set_block(Pos::new(1, 1, 0), "piston[extended=false,facing=east]", Placement::Quiet).unwrap();
        s.set_block(Pos::new(3, 1, 0), "oak_pressure_plate[powered=false]", Placement::Quiet).unwrap();
        s.set_block(Pos::new(3, 1, 1), "tnt[unstable=false]", Placement::Quiet).unwrap();
        s.summon("item", [2.5, 1.0, 0.5], None, Some(r#"{Item:{id:"minecraft:stone",count:1},PickupDelay:40}"#))
            .unwrap();
        for _ in 0..5 {
            s.step();
        }
        assert_eq!(s.block(Pos::new(3, 1, 0)), "minecraft:oak_pressure_plate[powered=false]");
        s.set_block(Pos::new(0, 1, 0), "redstone_block", Placement::Updates).unwrap();
        for _ in 0..5 {
            s.step();
        }
        assert_eq!(s.block(Pos::new(3, 1, 0)), "minecraft:oak_pressure_plate[powered=true]");
        assert_eq!(s.block(Pos::new(3, 1, 1)), "minecraft:air", "the plate lit the TNT");
        assert_eq!(tick_order(&s), [(1, "minecraft:item".into()), (2, "minecraft:tnt".into())]);
    }

    #[test]
    fn an_arrow_that_primes_tnt_from_a_button_stays_an_arrow() {
        let mut s = Session::new("26.3").unwrap();
        stone_under(&mut s, &[Pos::new(7, 1, 1)]);
        s.set_block(Pos::new(8, 1, 0), "stone", Placement::Quiet).unwrap();
        s.set_block(Pos::new(7, 1, 0), "oak_button[face=wall,facing=west,powered=false]", Placement::Quiet).unwrap();
        s.set_block(Pos::new(7, 1, 1), "tnt[unstable=false]", Placement::Quiet).unwrap();
        s.summon("arrow", [1.5, 1.75, 0.5], Some([1.5, 0.0, 0.0]), None).unwrap();
        for _ in 0..10 {
            s.step();
        }
        assert_eq!(s.block(Pos::new(7, 1, 0)), "minecraft:oak_button[face=wall,facing=west,powered=true]");
        assert_eq!(s.block(Pos::new(7, 1, 1)), "minecraft:air", "the button lit the TNT");
        assert_eq!(tick_order(&s), [(1, "minecraft:arrow".into()), (2, "minecraft:tnt".into())]);
    }

    /// Each change to the block at `at`, on stone, as an item summoned
    /// `drop` above its base falls in, over 20 ticks.
    fn changes_under_a_falling_item(at: Pos, block: &str, drop: f64) -> Vec<(u32, String)> {
        let mut s = Session::new("26.3").unwrap();
        s.set_block(at.below(), "stone", Placement::Quiet).unwrap();
        s.set_block(at, block, Placement::Quiet).unwrap();
        let item = Some(r#"{Item:{id:"minecraft:stone",count:1}}"#);
        s.summon("item", [at.x as f64 + 0.5, at.y as f64 + drop, at.z as f64 + 0.5], None, item).unwrap();
        let mut changes = Vec::new();
        let mut last = s.block(at);
        for _ in 0..20 {
            s.step();
            let now = s.block(at);
            if now != last {
                changes.push((s.tick(), now.clone()));
                last = now;
            }
        }
        changes
    }

    // While an entity ticks, its slot in the list is empty, and the empty
    // entity there sits at the origin. Blocks there must not feel it.

    #[test]
    fn a_weighted_plate_at_the_origin_feels_only_real_entities() {
        let plate = "light_weighted_pressure_plate[power=0]";
        let origin = changes_under_a_falling_item(Pos::new(0, 0, 0), plate, 0.5);
        let elsewhere = changes_under_a_falling_item(Pos::new(3, 0, 3), plate, 0.5);
        assert!(!elsewhere.is_empty() && elsewhere[0].0 > 1, "the item lands after a few ticks: {elsewhere:?}");
        assert_eq!(origin, elsewhere);
    }

    #[test]
    fn a_tripwire_at_the_origin_feels_only_real_entities() {
        let string =
            "tripwire[attached=false,disarmed=false,east=false,north=false,powered=false,south=false,west=false]";
        let origin = changes_under_a_falling_item(Pos::new(0, 0, 0), string, 0.9);
        let elsewhere = changes_under_a_falling_item(Pos::new(3, 0, 3), string, 0.9);
        assert!(
            !elsewhere.is_empty() && elsewhere[0].0 > 1,
            "the item reaches the string after a few ticks: {elsewhere:?}"
        );
        assert_eq!(origin, elsewhere);
    }

    #[test]
    fn a_build_starts_with_its_simulated_entities() {
        use tickwright_trace::fixture::SummonInput;
        let summon = |entity: &str, nbt: Option<&str>| SummonInput {
            entity: entity.into(),
            at: [0.5, 1.0, 0.5],
            motion: None,
            nbt: nbt.map(str::to_string),
        };
        let fixture = ResolvedFixture {
            format: 1,
            id: "t@r0".into(),
            name: "t".into(),
            transform: "r0".into(),
            ticks: 1,
            levels: Vec::new(),
            origin: [0, 0, 0],
            size: [1, 2, 1],
            margin: 0,
            blocks: vec![(0, 0, 0, "minecraft:stone".into())],
            stimuli: Vec::new(),
            containers: Vec::new(),
            comparators: Vec::new(),
            lecterns: Vec::new(),
            experiments: Vec::new(),
            day_time: None,
            max_chained_updates: None,
            entities: false,
            start_entities: vec![
                summon("armor_stand", Some("{Rotation:[90.0f,0.0f]}")),
                summon("item", Some(r#"{Item:{id:"minecraft:stone",count:1},UUID:[I;1,2,3,4],OnGround:1b}"#)),
                summon("falling_block", Some(r#"{BlockState:{Name:"minecraft:gravel"},Time:1}"#)),
            ],
        };
        let s = Session::from_fixture(&fixture, "26.3").unwrap();
        let kinds: Vec<String> = s.entities().into_iter().map(|e| e.kind).collect();
        assert_eq!(kinds, ["minecraft:item", "minecraft:falling_block"]);
    }

    #[test]
    fn clicks_number_entities_as_fixture_runs_do() {
        use tickwright_trace::fixture::{Action, Stimulus, SummonInput};
        let stimulus = |tick: u32, action: Action, summon: Option<SummonInput>| Stimulus {
            tick,
            action,
            pos: [0, 1, 0],
            state: None,
            quiet: false,
            discard_contents: false,
            page: None,
            time: None,
            place: None,
            summon,
        };
        let item = SummonInput {
            entity: "item".into(),
            at: [2.5, 1.0, 0.5],
            motion: None,
            nbt: Some(r#"{Item:{id:"minecraft:stone",count:1}}"#.into()),
        };
        let fixture = ResolvedFixture {
            format: 1,
            id: "t@r0".into(),
            name: "t".into(),
            transform: "r0".into(),
            ticks: 2,
            levels: Vec::new(),
            origin: [0, 0, 0],
            size: [3, 2, 1],
            margin: 0,
            blocks: vec![
                (0, 0, 0, "minecraft:stone".into()),
                (1, 0, 0, "minecraft:stone".into()),
                (2, 0, 0, "minecraft:stone".into()),
                (0, 1, 0, "minecraft:lever[face=floor,facing=north,powered=false]".into()),
            ],
            stimuli: vec![stimulus(1, Action::Use, None), stimulus(2, Action::Summon, Some(item.clone()))],
            containers: Vec::new(),
            comparators: Vec::new(),
            lecterns: Vec::new(),
            experiments: Vec::new(),
            day_time: None,
            max_chained_updates: None,
            entities: true,
            start_entities: Vec::new(),
        };
        let trace = crate::run::run(&fixture, "26.3").unwrap();
        let cloud = trace.entities.iter().rfind(|r| r.tick == 2).unwrap();

        let mut s = Session::from_fixture(&fixture, "26.3").unwrap();
        s.use_block(Pos::new(0, 1, 0));
        s.step();
        s.summon("item", item.at, None, item.nbt.as_deref()).unwrap();
        s.step();
        let local = &s.entities()[0];
        assert_eq!((local.id, local.pos), (cloud.id, cloud.pos));
        assert_eq!(s.level.random.internal(), trace.rng.last().unwrap().1);
    }

    #[test]
    fn a_step_reports_what_it_could_not_simulate() {
        let mut s = Session::new("26.3").unwrap();
        s.set_block(Pos::new(0, 0, 0), "stone", Placement::Quiet).unwrap();
        s.set_block(Pos::new(0, 1, 0), "fire", Placement::Quiet).unwrap();
        s.summon("item", [0.5, 1.0, 0.5], None, Some(r#"{Item:{id:"stone",count:1}}"#)).unwrap();
        assert_eq!(s.unsupported(), None);
        s.step();
        assert_eq!(s.unsupported().as_deref(), Some("an entity in fire is not simulated by the engine yet"));
        s.rewind(0).unwrap();
        assert_eq!(s.unsupported(), None);
    }

    #[test]
    fn unsupported_blocks_are_refused() {
        let mut s = Session::new("26.3").unwrap();
        let err = s.set_block(Pos::new(0, 0, 0), "furnace", Placement::Quiet).unwrap_err();
        assert!(err.to_string().contains("minecraft:furnace is not supported"), "{err}");
    }

    #[test]
    fn a_repeater_tick_shows_as_pending() {
        let mut s = Session::new("26.3").unwrap();
        s.set_block(Pos::new(0, 0, 0), "stone", Placement::Quiet).unwrap();
        s.set_block(Pos::new(1, 0, 0), "stone", Placement::Quiet).unwrap();
        s.set_block(Pos::new(0, 1, 0), "repeater[delay=4,facing=east,locked=false,powered=false]", Placement::Quiet)
            .unwrap();
        s.set_block(Pos::new(1, 1, 0), "redstone_block", Placement::Updates).unwrap();
        let pending = s.pending_ticks();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].block, "minecraft:repeater");
        assert_eq!(pending[0].due, 8);
    }

    // --- against a session that keeps everything ------------------------------

    use crate::record::TraceRecorder;
    use tickwright_trace::fixture::Action;
    use tickwright_trace::{FixtureSource, Transform};

    /// The session as it was before it kept less: a copy of the level and
    /// every event, as trace entries, after every tick.
    struct Reference {
        level: Level,
        checkpoints: Vec<Level>,
        events: Vec<Vec<Entry>>,
    }

    impl Reference {
        fn new(mut level: Level) -> Reference {
            let lo = Pos::new(i32::MIN, i32::MIN, i32::MIN);
            let hi = Pos::new(i32::MAX, i32::MAX, i32::MAX);
            let mut recorder = TraceRecorder::new(Pos::new(0, 0, 0), lo, hi);
            recorder.tick = 1;
            level.recorder = Some(Recorder::Trace(recorder));
            Reference { checkpoints: vec![level.clone()], level, events: vec![Vec::new()] }
        }

        fn recorder(&mut self) -> &mut TraceRecorder {
            match self.level.recorder.as_mut() {
                Some(Recorder::Trace(r)) => r,
                _ => unreachable!(),
            }
        }

        fn tick(&self) -> u32 {
            (self.checkpoints.len() - 1) as u32
        }

        fn step(&mut self) {
            self.level.tick();
            let t = self.tick() + 1;
            let rec = self.recorder();
            let entries = std::mem::take(&mut rec.entries);
            rec.tick = t + 1;
            self.events.push(entries);
            self.checkpoints.push(self.level.clone());
        }

        fn rewind(&mut self, t: u32) {
            self.checkpoints.truncate(t as usize + 1);
            self.events.truncate(t as usize + 1);
            self.level = self.checkpoints[t as usize].clone();
            let rec = self.recorder();
            rec.entries.clear();
            rec.tick = t + 1;
        }

        fn events(&mut self, t: u32) -> Vec<Entry> {
            if t == self.tick() + 1 {
                return self.recorder().entries.clone();
            }
            self.events.get(t as usize).cloned().unwrap_or_default()
        }

        fn history(&mut self, pos: Pos) -> Vec<(u32, String)> {
            let mut out = Vec::new();
            for t in 0..=self.tick() + 1 {
                for e in self.events(t) {
                    if e.kind == tickwright_trace::Kind::Set && e.pos == [pos.x, pos.y, pos.z] {
                        out.push((t, e.state.unwrap()));
                    }
                }
            }
            out
        }
    }

    /// An edit made to both sessions.
    #[derive(Debug)]
    enum Edit {
        Set(Pos, &'static str, Placement),
        Use(Pos),
        Destroy(Pos),
        Place(&'static str, blocks::placement::Click),
        Summon(&'static str, [f64; 3], &'static str),
        Item(Pos, usize, &'static str, i32),
        Page(Pos, i32),
        Time(i64),
    }

    fn edit_both(s: &mut Session, r: &mut Reference, edit: &Edit) {
        let level = &mut r.level;
        match *edit {
            Edit::Set(pos, state, how) => {
                s.set_block(pos, state, how).unwrap();
                let f = if how == Placement::Quiet { flags::CLIENTS | flags::KNOWN_SHAPE } else { flags::ALL };
                let id = level.reg.parse(state).unwrap();
                level.set_block(pos, id, f | flags::SKIP_BLOCK_ENTITY_SIDEEFFECTS);
            }
            Edit::Use(pos) => {
                s.use_block(pos);
                level.new_clicking_player();
                let state = level.get(pos);
                blocks::use_without_item(level, pos, state);
            }
            Edit::Destroy(pos) => {
                s.destroy(pos);
                level.destroy_block_with(pos, false, flags::SKIP_BLOCK_ENTITY_SIDEEFFECTS, MAX_RECURSION);
            }
            Edit::Place(item, click) => {
                let a = s.place(item, click).ok();
                level.new_clicking_player();
                let b = blocks::placement::place(level, item, click).ok();
                assert_eq!(a, b, "{edit:?}");
            }
            Edit::Summon(entity, at, nbt) => {
                let a = s.summon(entity, at, None, Some(nbt)).is_ok();
                let at = Vec3::new(at[0], at[1], at[2]);
                let b = crate::entity::summon(level, entity, at, None, Some(nbt)).is_ok();
                assert_eq!(a, b, "{edit:?}");
            }
            Edit::Item(pos, slot, id, count) => {
                let a = s.set_item(pos, slot, id, count).is_ok();
                let size = blocks::container::size(level, pos);
                let b = size > 0 && slot < size;
                if b {
                    let id = format!("minecraft:{id}");
                    blocks::container::set_item(level, pos, slot, blocks::container::Stack::new(&id, count));
                }
                assert_eq!(a, b, "{edit:?}");
            }
            Edit::Page(pos, page) => {
                s.turn_page(pos, page);
                blocks::lectern::set_page(level, pos, page);
            }
            Edit::Time(time) => {
                s.set_day_time(time);
                level.day_time = time;
            }
        }
    }

    /// A tiny deterministic random source for the edits.
    struct Dice(u64);

    impl Dice {
        fn below(&mut self, n: u64) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0 % n
        }

        fn pos(&mut self, lo: Pos, size: [i32; 3]) -> Pos {
            let r = |d: &mut Dice, n: i32| d.below(n as u64) as i32;
            Pos::new(lo.x + r(self, size[0]), lo.y + r(self, size[1]), lo.z + r(self, size[2]))
        }
    }

    /// A random edit somewhere in the box from `lo`, of `size`. Item and
    /// page edits mostly go to the containers and lecterns in `level`.
    fn random_edit(dice: &mut Dice, lo: Pos, size: [i32; 3], level: &Level) -> Edit {
        const STATES: [&str; 11] = [
            "air",
            "stone",
            "redstone_block",
            "redstone_wire",
            "repeater[delay=2,facing=east]",
            "lever[face=floor,facing=north,powered=false]",
            "redstone_lamp",
            "observer[facing=up]",
            "chest[facing=north,type=single]",
            "hopper[enabled=true,facing=down]",
            "comparator[facing=west]",
        ];
        let pos = dice.pos(lo, size);
        let holders = |dice: &mut Dice, is: fn(&blocks::BlockEntity) -> bool| {
            let mut at: Vec<Pos> = level.block_entities.iter().filter(|(_, e)| is(e)).map(|(p, _)| *p).collect();
            at.sort_by_key(|p| (p.y, p.z, p.x));
            match at.len() {
                0 => pos,
                n if dice.below(5) > 0 => at[dice.below(n as u64) as usize],
                _ => pos,
            }
        };
        match dice.below(9) {
            6 => {
                let at = holders(dice, |e| {
                    !matches!(e, blocks::BlockEntity::Lectern(_) | blocks::BlockEntity::Comparator(_))
                });
                let slot = dice.below(blocks::container::size(level, at) as u64 + 1) as usize;
                let id = ["redstone", "stone", "glowstone_dust"][dice.below(3) as usize];
                return Edit::Item(at, slot, id, 1 + dice.below(64) as i32);
            }
            7 => {
                let at = holders(dice, |e| matches!(e, blocks::BlockEntity::Lectern(_)));
                return Edit::Page(at, dice.below(12) as i32);
            }
            8 => return Edit::Time([0, 6000, 13000, 18000, 23000][dice.below(5) as usize]),
            _ => {}
        }
        match dice.below(6) {
            0 => {
                let how = if dice.below(3) == 0 { Placement::Quiet } else { Placement::Updates };
                Edit::Set(pos, STATES[dice.below(STATES.len() as u64) as usize], how)
            }
            1 => Edit::Use(pos),
            2 => Edit::Destroy(pos),
            3 => {
                let look = [Dir::North, Dir::East, Dir::South, Dir::West][dice.below(4) as usize];
                let click = blocks::placement::Click {
                    clicked: pos,
                    face: Dir::Up,
                    hit: [8, 16, 8],
                    look,
                    pitch: 40,
                    sneak: false,
                };
                Edit::Place(["repeater", "redstone", "comparator", "stone"][dice.below(4) as usize], click)
            }
            4 => Edit::Summon(
                "item",
                [pos.x as f64 + 0.5, pos.y as f64 + 2.0, pos.z as f64 + 0.5],
                r#"{Item:{id:"minecraft:redstone",count:2}}"#,
            ),
            _ => Edit::Summon("tnt", [pos.x as f64 + 0.5, pos.y as f64 + 1.0, pos.z as f64 + 0.5], "{fuse:12}"),
        }
    }

    /// Check every tick kept: its events, the blocks just after it, and
    /// the history of some positions. Reads ticks in order (each one runs
    /// on from the last read), backwards, and at random.
    fn check_the_past(s: &Session, r: &mut Reference, dice: &mut Dice, lo: Pos, size: [i32; 3], what: &str) {
        let now = s.tick();
        for t in 0..=now + 2 {
            assert_eq!(s.events(t), r.events(t), "{what}: events of tick {t}");
        }
        let blocks = |r: &Reference, t: u32| sorted_blocks(&r.checkpoints[t as usize]);
        for _ in 0..3 {
            let t = now - dice.below(u64::from(now).min(40) + 1) as u32;
            assert_eq!(s.blocks_at(t).unwrap(), blocks(r, t), "{what}: blocks at tick {t}");
        }
        for t in 0..=now {
            assert_eq!(s.blocks_at(t).unwrap(), blocks(r, t), "{what}: blocks at tick {t} reading forwards");
        }
        for t in (0..=now).rev().step_by(3) {
            assert_eq!(s.blocks_at(t).unwrap(), blocks(r, t), "{what}: blocks at tick {t} reading backwards");
        }
        for _ in 0..10 {
            let t = dice.below(u64::from(now) + 1) as u32;
            assert_eq!(fingerprint(&s.rebuild(t)), fingerprint(&r.checkpoints[t as usize]), "{what}: tick {t} rebuilt");
        }
        assert!(s.blocks_at(now + 1).is_none());
        // Some positions, and every block set in the last tick or since.
        let mut positions: Vec<Pos> = (0..20).map(|_| dice.pos(lo, size)).collect();
        let sets = r.events(now).into_iter().chain(r.events(now + 1));
        positions.extend(
            sets.filter(|e| e.kind == tickwright_trace::Kind::Set).map(|e| Pos::new(e.pos[0], e.pos[1], e.pos[2])),
        );
        for pos in positions {
            assert_eq!(s.history(pos), r.history(pos), "{what}: history of {pos:?}");
        }
    }

    /// Run, edit, read the past and rewind a session and a reference side
    /// by side for `rounds` rounds, checking they always agree.
    fn agree_with_reference(fixture: &ResolvedFixture, every: u32, budget: usize, rounds: usize, seed: u64) {
        let what = format!("{} every {every} ticks or {budget} bytes", fixture.id);
        let mut s = Session::from_fixture(fixture, "26.3").unwrap();
        (s.every, s.budget) = (every, budget);
        let mut r = Reference::new(crate::run::place(fixture, Rules::for_version("26.3").unwrap()).unwrap());
        let mut dice = Dice(seed);
        let [ox, oy, oz] = fixture.origin;
        let lo = Pos::new(ox, oy, oz);
        let size = [fixture.size[0], fixture.size[1] + 2, fixture.size[2]];
        for round in 0..rounds {
            let what = format!("{what}, round {round}");
            for _ in 0..20 + dice.below(40) {
                if dice.below(4) == 0 {
                    let edit = random_edit(&mut dice, lo, size, &s.level);
                    edit_both(&mut s, &mut r, &edit);
                }
                let busy = s.step();
                r.step();
                let t = s.tick();
                assert_eq!(busy, !r.events(t).is_empty(), "{what}: whether tick {t} was busy");
            }
            check_the_past(&s, &mut r, &mut dice, lo, size, &what);
            assert_eq!(fingerprint(&s.level), fingerprint(&r.level), "{what}: the world now");
            for _ in 0..dice.below(3) {
                let edit = random_edit(&mut dice, lo, size, &s.level);
                edit_both(&mut s, &mut r, &edit);
            }
            check_the_past(&s, &mut r, &mut dice, lo, size, &format!("{what}, with edits waiting"));
            let t = match dice.below(4) {
                0 => s.tick(),
                _ => dice.below(u64::from(s.tick()) + 1) as u32,
            };
            // Leave the last tick read somewhere a rewind may drop.
            s.blocks_at(dice.below(u64::from(s.tick()) + 1) as u32);
            s.rewind(t).unwrap();
            r.rewind(t);
            assert_eq!(fingerprint(&s.level), fingerprint(&r.level), "{what}: rewound to tick {t}");
            assert_eq!(s.tick(), r.tick());
            assert!(s.events(t + 1).is_empty());
        }
    }

    /// Three rings of the benchmark, and between the first two a chest, a
    /// lectern and a daylight detector, each read by a comparator, so
    /// item, page and time edits change what the redstone does.
    fn busy_build() -> ResolvedFixture {
        use tickwright_trace::fixture::Lectern;
        let mut f = crate::bench::redstone_rings(3, 13, 0);
        f.entities = true;
        f.day_time = Some(6000);
        for x in 1..=10 {
            f.blocks.push((x, 0, 2, "minecraft:stone".into()));
        }
        for (x, state) in [
            (1, "chest[facing=north,type=single]"),
            (2, "comparator[facing=west,mode=compare,powered=false]"),
            (3, "redstone_lamp[lit=false]"),
            (5, "lectern[facing=north,has_book=true,powered=false]"),
            (6, "comparator[facing=west,mode=compare,powered=false]"),
            (7, "redstone_lamp[lit=false]"),
            (9, "daylight_detector[inverted=false,power=0]"),
            (10, "redstone_lamp[lit=false]"),
        ] {
            f.blocks.push((x, 1, 2, format!("minecraft:{state}")));
        }
        f.blocks.sort_by_key(|&(x, y, z, _)| (y, z, x));
        f.containers = vec![Container {
            pos: [1, 1, 2],
            items: vec![ItemSlot { slot: 0, id: "minecraft:redstone".into(), count: 10, ..Default::default() }],
        }];
        f.lecterns = vec![Lectern { pos: [5, 1, 2], pages: 10, page: 0 }];
        f
    }

    #[test]
    fn a_session_keeping_checkpoints_answers_as_one_that_kept_every_tick() {
        let f = busy_build();
        agree_with_reference(&f, CHECKPOINT_TICKS, CHECKPOINT_BYTES, 8, 1);
        agree_with_reference(&f, 1, CHECKPOINT_BYTES, 4, 2);
        agree_with_reference(&f, 5, CHECKPOINT_BYTES, 8, 3);
        agree_with_reference(&f, CHECKPOINT_TICKS, 3000, 8, 4);
    }

    #[test]
    fn checkpoints_are_kept_at_most_k_ticks_apart_and_sooner_when_busy() {
        let mut s = Session::from_fixture(&crate::bench::redstone_rings(2, 9, 0), "26.3").unwrap();
        for _ in 0..100 {
            s.step();
        }
        let ticks: Vec<u32> = s.checkpoints.iter().map(|(t, _)| *t).collect();
        assert_eq!(ticks, [0, 32, 64, 96]);
        assert!(s.checkpoints.iter().all(|(_, l)| l.recorder.is_none()));
        s.rewind(70).unwrap();
        let ticks: Vec<u32> = s.checkpoints.iter().map(|(t, _)| *t).collect();
        assert_eq!(ticks, [0, 32, 64]);
        s.budget = 1000;
        for _ in 0..30 {
            s.step();
        }
        assert!(s.checkpoints.len() > 4, "a busy stretch checkpoints sooner: {:?}", s.checkpoints.len());
    }

    /// Every fixture, played through a session that keeps a checkpoint
    /// every three ticks and through the reference, gives the same events,
    /// and every tick read back or rewound to is the same.
    /// A build of every block entity that holds things, each filled, in a
    /// session that simulates entities as the editor's do.
    fn filled_block_entities() -> Session {
        let source = FixtureSource::from_json(
            r###"{"name": "upload", "ticks": 1, "entities": true,
                "palette": {"o": "stone", "c": "chest[facing=north,type=single,waterlogged=false]",
                    "b": "barrel[facing=up,open=false]", "h": "hopper[enabled=true,facing=down]",
                    "d": "dropper[facing=up,triggered=false]", "D": "dispenser[facing=up,triggered=false]",
                    "C": "crafter[crafting=false,orientation=north_up,triggered=false]",
                    "s": "oak_shelf[facing=south,powered=false,side_chain=unconnected,waterlogged=false]",
                    "l": "lectern[facing=north,has_book=true,powered=false]"},
                "layers": [["oooooooo"], ["cbhdDCsl"]],
                "stimuli": [],
                "containers": [
                    {"pos": [0, 1, 0], "items": [{"slot": 0, "id": "stone", "count": 5}]},
                    {"pos": [1, 1, 0], "items": [{"slot": 3, "id": "stone", "count": 5}]},
                    {"pos": [2, 1, 0], "items": [{"slot": 1, "id": "stone", "count": 5}]},
                    {"pos": [3, 1, 0], "items": [{"slot": 2, "id": "stone", "count": 5}]},
                    {"pos": [4, 1, 0], "items": [{"slot": 4, "id": "stone", "count": 5}]},
                    {"pos": [5, 1, 0], "items": [{"slot": 0, "id": "oak_log", "count": 5}]},
                    {"pos": [6, 1, 0], "items": [{"slot": 1, "id": "stone", "count": 5}]}
                ],
                "lecterns": [{"pos": [7, 1, 0], "pages": 3}]}"###,
        )
        .unwrap();
        let mut s = Session::from_fixture(&source.resolve(Transform::IDENTITY).unwrap(), "26.3").unwrap();
        s.every = 3;
        s
    }

    #[test]
    fn editing_away_filled_block_entities_spills_nothing() {
        // The editor's edits remove a block entity as `/setblock` does, so
        // what it holds goes with it: nothing to refuse, in any edit, made
        // before the first tick or after some have run.
        type Change = fn(&mut Session, Pos);
        let edits: [(&str, Change); 4] = [
            ("set air with updates", |s, p| s.set_block(p, "air", Placement::Updates).unwrap()),
            ("set air quietly", |s, p| s.set_block(p, "air", Placement::Quiet).unwrap()),
            ("paste stone over it", |s, p| s.set_block(p, "stone", Placement::Quiet).unwrap()),
            ("break it", |s, p| s.destroy(p)),
        ];
        for (what, edit) in edits {
            for at in [0, 5] {
                let mut s = filled_block_entities();
                for _ in 0..at {
                    s.step();
                }
                for x in 0..8 {
                    edit(&mut s, Pos::new(x, 101, 0));
                }
                s.step();
                let next = s.blocks();
                for _ in 0..9 {
                    s.step();
                }
                assert_eq!(s.unsupported(), None, "{what} at tick {at}");
                assert!(s.entities().is_empty(), "{what} at tick {at}: nothing spills");
                assert!(s.containers().is_empty(), "{what} at tick {at}");
                // Rebuilt from its checkpoints (every third tick, each
                // checked against a replay of the edits), the past is the
                // same, and so is the future run again.
                let after = s.blocks();
                assert_eq!(s.blocks_at(at + 1), Some(next.clone()), "{what} at tick {at}");
                s.rewind(at + 1).unwrap();
                assert_eq!(s.blocks(), next, "{what} at tick {at}: rewound");
                for _ in 0..9 {
                    s.step();
                }
                assert_eq!(s.blocks(), after, "{what} at tick {at}: run again");
                assert_eq!(s.unsupported(), None, "{what} at tick {at}: run again");
            }
        }
    }

    #[test]
    fn a_dispenser_takes_only_items_it_can_dispense() {
        let mut s = filled_block_entities();
        let err = s.set_item(Pos::new(4, 101, 0), 0, "tnt", 1).unwrap_err().to_string();
        assert_eq!(err, "dispensing minecraft:tnt is not supported by the engine yet");
        s.set_item(Pos::new(4, 101, 0), 0, "cobblestone", 1).unwrap();
        // A dropper throws anything.
        s.set_item(Pos::new(3, 101, 0), 0, "tnt", 1).unwrap();
    }

    #[test]
    fn every_fixture_rewinds_as_a_session_that_kept_every_tick() {
        let mut dirs = vec![std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")];
        let mut checked = 0;
        while let Some(d) = dirs.pop() {
            for entry in std::fs::read_dir(d).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    dirs.push(path);
                    continue;
                }
                if path.extension().is_none_or(|x| x != "json") {
                    continue;
                }
                let source = FixtureSource::from_json(&std::fs::read_to_string(&path).unwrap()).unwrap();
                let fixture = source.resolve(Transform::IDENTITY).unwrap();
                if crate::run::run(&fixture, "26.3").is_err() {
                    continue; // refused
                }
                replay_fixture(&fixture);
                checked += 1;
            }
        }
        assert!(checked > 150, "only {checked} fixtures ran");
    }

    fn replay_fixture(f: &ResolvedFixture) {
        let rules = || Rules::for_version("26.3").unwrap();
        let mut s = Session::from_fixture(f, "26.3").unwrap();
        s.every = 3;
        let mut r = Reference::new(crate::run::place(f, rules()).unwrap());
        let [ox, oy, oz] = f.origin;
        let stimuli = |s: &mut Session, t: u32| {
            for st in f.stimuli.iter().filter(|st| st.tick == t) {
                let pos = Pos::new(ox + st.pos[0], oy + st.pos[1], oz + st.pos[2]);
                match st.action {
                    Action::Use => s.use_block(pos),
                    Action::Set => {
                        let how = if st.quiet { Placement::Quiet } else { Placement::Updates };
                        s.set_block_as(pos, st.state.as_deref().unwrap(), how, st.discard_contents).unwrap()
                    }
                    Action::Destroy => s.destroy_as(pos, st.discard_contents),
                    Action::Place => {
                        let p = st.place.as_ref().unwrap();
                        s.place(&p.item, crate::run::click(pos, p).unwrap()).unwrap();
                    }
                    Action::Page => s.turn_page(pos, st.page.unwrap() as i32),
                    Action::Time => s.set_day_time(st.time.unwrap()),
                    Action::Summon => {
                        let sm = st.summon.as_ref().unwrap();
                        let centre = |v: f64| if v.fract() == 0.0 { v + 0.5 } else { v };
                        let at = [centre(ox as f64 + sm.at[0]), oy as f64 + sm.at[1], centre(oz as f64 + sm.at[2])];
                        s.summon(&sm.entity, at, sm.motion, sm.nbt.as_deref()).unwrap();
                    }
                }
            }
        };
        let run = |s: &mut Session, r: &mut Reference, from: u32| {
            for t in from..=f.ticks {
                stimuli(s, t);
                crate::run::apply_stimuli(&mut r.level, f, t).unwrap();
                s.step();
                r.step();
                assert_eq!(s.events(t), r.events(t), "{}: events of tick {t}", f.id);
            }
        };
        run(&mut s, &mut r, 1);
        let lo = Pos::new(ox - f.margin, oy - f.margin, oz - f.margin);
        let size = f.size.map(|n| n + 2 * f.margin);
        let mut dice = Dice(0x9e37_79b9_7f4a_7c15 ^ f.ticks as u64);
        check_the_past(&s, &mut r, &mut dice, lo, size, &f.id);
        for t in [f.ticks / 2, 1, 0] {
            s.rewind(t).unwrap();
            r.rewind(t);
            assert_eq!(fingerprint(&s.level), fingerprint(&r.level), "{}: rewound to tick {t}", f.id);
            run(&mut s, &mut r, t + 1);
        }
    }
}
