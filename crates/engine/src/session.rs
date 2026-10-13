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
/// as quick at the cost of more copies.
pub const CHECKPOINT_BYTES: usize = 256 * 1024;

/// How a block is written.
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
    Set { pos: Pos, state: StateId, flags: i32 },
    Place { item: String, click: blocks::placement::Click },
    Destroy(Pos),
    Use(Pos),
    Page { pos: Pos, page: i32 },
    Time(i64),
    Item { pos: Pos, slot: usize, id: String, count: i32 },
    Summon { entity: String, at: Vec3, motion: Option<Vec3>, nbt: Option<String> },
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
        Input::Destroy(pos) => {
            level.destroy_block(*pos, false, MAX_RECURSION);
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
        let id = self.level.reg.parse(state)?;
        if self.level.kind(id) == Kind::Other {
            bail!("{} is not supported by the engine yet", self.level.reg.name(id));
        }
        let flags = match placement {
            Placement::Quiet => flags::CLIENTS | flags::KNOWN_SHAPE,
            Placement::Updates => flags::ALL,
        };
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

    /// Break the block at `pos` without drops, with normal updates.
    pub fn destroy(&mut self, pos: Pos) {
        let _ = self.edit(Input::Destroy(pos));
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
            self.checkpoints.push((t, self.level.clone()));
            self.level.recorder = recorder;
        }
        busy
    }

    /// The first thing the run met that the engine can't simulate exactly,
    /// as a sentence such as "an item inside a block is not simulated
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
            .filter(|e| !e.removed && !matches!(e.data, crate::entity::Data::Gone))
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
                    .map(|(slot, s)| ItemSlot { slot: slot as u32, id: s.id.clone(), count: s.count as u32 })
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
        s.summon("item", [0.5, 0.5, 0.5], None, Some(r#"{Item:{id:"stone",count:1}}"#)).unwrap();
        assert_eq!(s.unsupported(), None);
        s.step();
        assert_eq!(s.unsupported().as_deref(), Some("an item inside a block is not simulated by the engine yet"));
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

    /// Everything a level holds that the editor reads or a later tick
    /// depends on, as text.
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
            sorted_blocks(level),
            level.pending_block_ticks(),
            level.entities,
        )
    }

    /// An edit made to both sessions.
    #[derive(Debug)]
    enum Edit {
        Set(Pos, &'static str, Placement),
        Use(Pos),
        Destroy(Pos),
        Place(&'static str, blocks::placement::Click),
        Summon(&'static str, [f64; 3], &'static str),
    }

    fn edit_both(s: &mut Session, r: &mut Reference, edit: &Edit) {
        let level = &mut r.level;
        match *edit {
            Edit::Set(pos, state, how) => {
                s.set_block(pos, state, how).unwrap();
                let f = if how == Placement::Quiet { flags::CLIENTS | flags::KNOWN_SHAPE } else { flags::ALL };
                let id = level.reg.parse(state).unwrap();
                level.set_block(pos, id, f);
            }
            Edit::Use(pos) => {
                s.use_block(pos);
                level.new_clicking_player();
                let state = level.get(pos);
                blocks::use_without_item(level, pos, state);
            }
            Edit::Destroy(pos) => {
                s.destroy(pos);
                level.destroy_block(pos, false, MAX_RECURSION);
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

    fn random_edit(dice: &mut Dice, lo: Pos, size: [i32; 3]) -> Edit {
        const STATES: [&str; 8] = [
            "air",
            "stone",
            "redstone_block",
            "redstone_wire",
            "repeater[delay=2,facing=east]",
            "lever[face=floor,facing=north,powered=false]",
            "redstone_lamp",
            "observer[facing=up]",
        ];
        let pos = dice.pos(lo, size);
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
                    edit_both(&mut s, &mut r, &random_edit(&mut dice, lo, size));
                }
                let busy = s.step();
                r.step();
                let t = s.tick();
                assert_eq!(busy, !r.events(t).is_empty(), "{what}: whether tick {t} was busy");
            }
            check_the_past(&s, &mut r, &mut dice, lo, size, &what);
            assert_eq!(fingerprint(&s.level), fingerprint(&r.level), "{what}: the world now");
            for _ in 0..dice.below(3) {
                edit_both(&mut s, &mut r, &random_edit(&mut dice, lo, size));
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

    fn busy_build() -> ResolvedFixture {
        let mut f = crate::bench::redstone_rings(3, 13, 0);
        f.entities = true;
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
                        s.set_block(pos, st.state.as_deref().unwrap(), how).unwrap()
                    }
                    Action::Destroy => s.destroy(pos),
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
