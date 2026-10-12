//! An interactive simulation for an editor: build, click, step and rewind.
//!
//! A [`Session`] holds a level and a checkpoint after every tick, so the
//! editor can step forward one tick at a time, scrub back to any earlier
//! tick, and branch off from there. Edits made between ticks land the way
//! the capture harness applies stimuli: before the next tick runs, recorded
//! in the input phase of that tick.
//!
//! Every event the engine records, anywhere in the world, is kept with the
//! tick it belongs to, which is what the editor's update viewer and
//! timelines read. Positions are absolute.

use anyhow::{Result, bail};
use tickwright_trace::fixture::{Container, ItemSlot};
use tickwright_trace::{Entry, ResolvedFixture};

use crate::blocks::{self, Kind};
use crate::level::{Level, MAX_RECURSION, flags};
use crate::pos::{Dir, Pos};
use crate::record::Recorder;
use crate::rules::Rules;

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

/// What a position reads as, for the power views.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Power {
    /// The strongest signal reaching the block from its neighbors.
    pub weak: i32,
    /// The strongest signal driven into the block (strong power).
    pub strong: i32,
}

pub struct Session {
    level: Level,
    /// `checkpoints[t]` is the level after tick `t`; tick 0 is the build.
    checkpoints: Vec<Level>,
    /// Events by tick: `events[t]` holds tick `t`'s, including the input
    /// applied before it ran.
    events: Vec<Vec<Entry>>,
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
        let lo = Pos::new(i32::MIN, i32::MIN, i32::MIN);
        let hi = Pos::new(i32::MAX, i32::MAX, i32::MAX);
        level.recorder = Some(Recorder::new(Pos::new(0, 0, 0), lo, hi));
        level.recorder.as_mut().unwrap().tick = 1;
        Session { checkpoints: vec![level.clone()], level, events: vec![Vec::new()] }
    }

    /// The last tick that has run; 0 before the first.
    pub fn tick(&self) -> u32 {
        (self.checkpoints.len() - 1) as u32
    }

    // --- editing -------------------------------------------------------------

    /// Write `state` (such as `repeater[delay=2,facing=north]`) at `pos`.
    /// Blocks the engine does not simulate are refused by name.
    pub fn set_block(&mut self, pos: Pos, state: &str, placement: Placement) -> Result<()> {
        let id = self.level.reg.parse(state)?;
        if self.level.kind(id) == Kind::Other {
            bail!("{} is not supported by the engine yet", self.level.reg.name(id));
        }
        let f = match placement {
            Placement::Quiet => flags::CLIENTS | flags::KNOWN_SHAPE,
            Placement::Updates => flags::ALL,
        };
        self.level.set_block(pos, id, f);
        Ok(())
    }

    /// Right-click a face of a block holding `item`, as a player would, so
    /// the block's placement rules pick its state. Returns whether a block
    /// was placed.
    pub fn place(&mut self, item: &str, click: blocks::placement::Click) -> Result<bool> {
        blocks::placement::place(&mut self.level, item, click)
    }

    /// Break the block at `pos` without drops, with normal updates.
    pub fn destroy(&mut self, pos: Pos) {
        self.level.destroy_block(pos, MAX_RECURSION);
    }

    /// Right-click the block at `pos` with an empty hand.
    pub fn use_block(&mut self, pos: Pos) {
        let state = self.level.get(pos);
        blocks::use_without_item(&mut self.level, pos, state);
    }

    /// Turn the book on the lectern at `pos` to `page`, as its menu does.
    pub fn turn_page(&mut self, pos: Pos, page: i32) {
        blocks::lectern::set_page(&mut self.level, pos, page);
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
        blocks::container::set_item(&mut self.level, pos, slot, blocks::container::Stack::new(&id, count));
        Ok(())
    }

    // --- running -------------------------------------------------------------

    /// Run one game tick and return its events, input first.
    pub fn step(&mut self) -> &[Entry] {
        self.level.tick();
        let t = self.tick() + 1;
        let rec = self.level.recorder.as_mut().unwrap();
        let entries = std::mem::take(&mut rec.entries);
        rec.tick = t + 1;
        self.events.push(entries);
        self.checkpoints.push(self.level.clone());
        &self.events[t as usize]
    }

    /// Go back to just after tick `t`, dropping everything after it. Edits
    /// made since the last step are dropped too.
    pub fn rewind(&mut self, t: u32) -> Result<()> {
        if t > self.tick() {
            bail!("tick {t} has not run yet; the last is {}", self.tick());
        }
        self.checkpoints.truncate(t as usize + 1);
        self.events.truncate(t as usize + 1);
        self.level = self.checkpoints[t as usize].clone();
        let rec = self.level.recorder.as_mut().unwrap();
        rec.entries.clear();
        rec.tick = t + 1;
        Ok(())
    }

    /// The events of tick `t`, or the edits waiting for the next tick when
    /// `t` is one past the last.
    pub fn events(&self, t: u32) -> &[Entry] {
        if t == self.tick() + 1 {
            return &self.level.recorder.as_ref().unwrap().entries;
        }
        self.events.get(t as usize).map_or(&[], Vec::as_slice)
    }

    // --- reading -------------------------------------------------------------

    /// Every non-air block as `(pos, state)`, sorted by y, z, x.
    pub fn blocks(&self) -> Vec<(Pos, String)> {
        let mut out: Vec<(Pos, String)> = self.level.non_air().map(|(p, s)| (p, self.level.reg.format(s))).collect();
        out.sort_by_key(|(p, _)| (p.y, p.z, p.x));
        out
    }

    /// Every non-air block as it was just after tick `t` (tick 0 is the
    /// build), before any edits made after that tick. `None` when tick `t`
    /// has not run.
    pub fn blocks_at(&self, t: u32) -> Option<Vec<(Pos, String)>> {
        let level = self.checkpoints.get(t as usize)?;
        let mut out: Vec<(Pos, String)> = level.non_air().map(|(p, s)| (p, level.reg.format(s))).collect();
        out.sort_by_key(|(p, _)| (p.y, p.z, p.x));
        Some(out)
    }

    pub fn block(&self, pos: Pos) -> String {
        self.level.reg.format(self.level.get(pos))
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
}
