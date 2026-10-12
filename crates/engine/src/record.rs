//! Turning what the engine does into a trace, the same way the capture
//! harness hooks the game: every write, update, scheduled tick and block
//! event inside the fixture's box, as it happens.

use tickwright_trace::{Entry, Kind, Phase};

use crate::pos::Pos;

pub enum Event {
    Set(String),
    Neighbor,
    Shape,
    Schedule { delay: i64, priority: i32 },
    Tick,
    BlockEventQueued,
    BlockEvent { a: i32, b: i32 },
}

pub struct Recorder {
    pub origin: Pos,
    pub min: Pos,
    pub max: Pos,
    pub tick: u32,
    pub entries: Vec<Entry>,
}

impl Recorder {
    pub fn new(origin: Pos, min: Pos, max: Pos) -> Recorder {
        Recorder { origin, min, max, tick: 0, entries: Vec::new() }
    }

    fn contains(&self, p: Pos) -> bool {
        (self.min.x..=self.max.x).contains(&p.x)
            && (self.min.y..=self.max.y).contains(&p.y)
            && (self.min.z..=self.max.z).contains(&p.z)
    }

    pub fn record(&mut self, phase: Phase, event: Event, pos: Pos) {
        if !self.contains(pos) {
            return;
        }
        let (kind, state, detail) = match event {
            Event::Set(s) => (Kind::Set, Some(s), None),
            Event::Neighbor => (Kind::Neighbor, None, None),
            Event::Shape => (Kind::Shape, None, None),
            Event::Schedule { delay, priority } => {
                (Kind::Schedule, None, Some(format!("delay={delay},priority={priority}")))
            }
            Event::Tick => (Kind::Tick, None, None),
            Event::BlockEventQueued => (Kind::BlockEventQueued, None, None),
            Event::BlockEvent { a, b } => (Kind::BlockEvent, None, Some(format!("{a},{b}"))),
        };
        self.entries.push(Entry {
            tick: self.tick,
            phase,
            kind,
            pos: [pos.x - self.origin.x, pos.y - self.origin.y, pos.z - self.origin.z],
            state,
            detail,
        });
    }
}
