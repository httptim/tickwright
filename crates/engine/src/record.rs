//! Turning what the engine does into a record, the same way the capture
//! harness hooks the game: every write, update, scheduled tick and block
//! event, as it happens.
//!
//! A fixture run keeps the events inside the fixture's box as trace
//! entries ([`TraceRecorder`]). An editor session keeps every event in the
//! world for as long as it runs, so it packs them small instead
//! ([`Packer`]): most events take one byte, and [`unpack`] turns them back
//! into the entries a trace would hold.

use tickwright_trace::{Entry, Kind, Phase};

use crate::pos::Pos;
use crate::registry::{Registry, StateId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    Set(StateId),
    Neighbor,
    Shape,
    Schedule { delay: i64, priority: i32 },
    Tick,
    BlockEventQueued,
    BlockEvent { a: i32, b: i32 },
    Items(String),
}

impl Event {
    /// The trace entry for this event.
    fn entry(self, reg: &Registry, tick: u32, phase: Phase, pos: [i32; 3]) -> Entry {
        let (kind, state, detail) = match self {
            Event::Set(s) => (Kind::Set, Some(reg.format(s)), None),
            Event::Neighbor => (Kind::Neighbor, None, None),
            Event::Shape => (Kind::Shape, None, None),
            Event::Schedule { delay, priority } => {
                (Kind::Schedule, None, Some(format!("delay={delay},priority={priority}")))
            }
            Event::Tick => (Kind::Tick, None, None),
            Event::BlockEventQueued => (Kind::BlockEventQueued, None, None),
            Event::BlockEvent { a, b } => (Kind::BlockEvent, None, Some(format!("{a},{b}"))),
            Event::Items(d) => (Kind::Items, None, Some(d)),
        };
        Entry { tick, phase, kind, pos, state, detail }
    }
}

/// Where a level's events go.
#[derive(Clone)]
pub enum Recorder {
    /// A fixture run's trace.
    Trace(TraceRecorder),
    /// An editor session's events, packed.
    Packed(Packer),
}

impl Recorder {
    pub fn record(&mut self, reg: &Registry, phase: Phase, event: Event, pos: Pos) {
        match self {
            Recorder::Trace(r) => r.record(reg, phase, event, pos),
            Recorder::Packed(p) => p.record(phase, event, pos),
        }
    }
}

/// Trace entries for the events inside a box, with positions relative to
/// an origin.
#[derive(Clone)]
pub struct TraceRecorder {
    pub origin: Pos,
    pub min: Pos,
    pub max: Pos,
    pub tick: u32,
    pub entries: Vec<Entry>,
}

impl TraceRecorder {
    pub fn new(origin: Pos, min: Pos, max: Pos) -> TraceRecorder {
        TraceRecorder { origin, min, max, tick: 0, entries: Vec::new() }
    }

    fn contains(&self, p: Pos) -> bool {
        (self.min.x..=self.max.x).contains(&p.x)
            && (self.min.y..=self.max.y).contains(&p.y)
            && (self.min.z..=self.max.z).contains(&p.z)
    }

    pub fn record(&mut self, reg: &Registry, phase: Phase, event: Event, pos: Pos) {
        if !self.contains(pos) {
            return;
        }
        let rel = [pos.x - self.origin.x, pos.y - self.origin.y, pos.z - self.origin.z];
        self.entries.push(event.entry(reg, self.tick, phase, rel));
    }
}

// --- packing -----------------------------------------------------------------
//
// Each event starts with one byte: its kind in the low three bits and, in
// the high five, how its position differs from the event before it in the
// same tick (the first is measured from 0, 0, 0). Neighbor and shape
// updates, nearly every event of a busy tick, land beside the block that
// caused them, so their step is one of `STEPS` and the event is that byte
// alone. A longer step follows as three zigzag varints. A change of phase
// is its own two bytes, before the first event of the new phase; a tick
// starts in the input phase. What some kinds carry follows the position:
// a set's state, a schedule's delay and priority, a block event's two
// numbers, a container's contents.

const KINDS: [Kind; 8] = [
    Kind::Set,
    Kind::Neighbor,
    Kind::Shape,
    Kind::Schedule,
    Kind::Tick,
    Kind::BlockEventQueued,
    Kind::BlockEvent,
    Kind::Items,
];

const PHASES: [Phase; 9] = [
    Phase::Input,
    Phase::BlockTicks,
    Phase::FluidTicks,
    Phase::Raids,
    Phase::Chunks,
    Phase::BlockEvents,
    Phase::Entities,
    Phase::BlockEntities,
    Phase::Other,
];

/// Steps that fit in an event's first byte: none, one or two blocks along
/// an axis, and one block along each of two axes.
const STEPS: [[i32; 3]; 25] = [
    [0, 0, 0],
    [-1, 0, 0],
    [1, 0, 0],
    [0, -1, 0],
    [0, 1, 0],
    [0, 0, -1],
    [0, 0, 1],
    [-2, 0, 0],
    [2, 0, 0],
    [0, -2, 0],
    [0, 2, 0],
    [0, 0, -2],
    [0, 0, 2],
    [-1, -1, 0],
    [-1, 1, 0],
    [1, -1, 0],
    [1, 1, 0],
    [-1, 0, -1],
    [-1, 0, 1],
    [1, 0, -1],
    [1, 0, 1],
    [0, -1, -1],
    [0, -1, 1],
    [0, 1, -1],
    [0, 1, 1],
];

/// The step follows as three varints.
const FAR: u8 = 30;
/// With kind bits 0, a phase change: the phase's index follows as a byte.
const NEW_PHASE: u8 = 31;
const NONE: u8 = u8::MAX;

/// The code of each step with every component in -2..=2, by
/// `(x + 2) * 25 + (y + 2) * 5 + z + 2`, or `NONE`.
const STEP_CODES: [u8; 125] = {
    let mut codes = [NONE; 125];
    let mut i = 0;
    while i < STEPS.len() {
        let [x, y, z] = STEPS[i];
        codes[((x + 2) * 25 + (y + 2) * 5 + z + 2) as usize] = i as u8;
        i += 1;
    }
    codes
};

fn step_code(d: [i32; 3]) -> u8 {
    if d.iter().all(|c| (-2..=2).contains(c)) {
        STEP_CODES[((d[0] + 2) * 25 + (d[1] + 2) * 5 + d[2] + 2) as usize]
    } else {
        NONE
    }
}

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push(v as u8 | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn put_signed(out: &mut Vec<u8>, v: i64) {
    put_varint(out, ((v << 1) ^ (v >> 63)) as u64);
}

/// One tick's events, packed as they happen.
#[derive(Clone, Default)]
pub struct Packer {
    pub bytes: Vec<u8>,
    /// The blocks set, with their new states, in order.
    pub sets: Vec<(Pos, StateId)>,
    /// How many events there are.
    pub count: usize,
    last: [i32; 3],
    phase: u8,
}

impl Packer {
    pub fn record(&mut self, phase: Phase, event: Event, pos: Pos) {
        let p = PHASES.iter().position(|&x| x == phase).unwrap_or(PHASES.len() - 1) as u8;
        if p != self.phase {
            self.bytes.push(NEW_PHASE << 3);
            self.bytes.push(p);
            self.phase = p;
        }
        let at = [pos.x, pos.y, pos.z];
        let d = [0, 1, 2].map(|i| at[i].wrapping_sub(self.last[i]));
        let code = match step_code(d) {
            NONE => FAR,
            c => c,
        };
        let kind = match &event {
            Event::Set(_) => 0,
            Event::Neighbor => 1,
            Event::Shape => 2,
            Event::Schedule { .. } => 3,
            Event::Tick => 4,
            Event::BlockEventQueued => 5,
            Event::BlockEvent { .. } => 6,
            Event::Items(_) => 7,
        };
        self.bytes.push(code << 3 | kind);
        if code == FAR {
            for c in d {
                put_signed(&mut self.bytes, i64::from(c));
            }
        }
        match event {
            Event::Set(s) => {
                put_varint(&mut self.bytes, u64::from(s.0));
                self.sets.push((pos, s));
            }
            Event::Schedule { delay, priority } => {
                put_signed(&mut self.bytes, delay);
                put_signed(&mut self.bytes, i64::from(priority));
            }
            Event::BlockEvent { a, b } => {
                put_signed(&mut self.bytes, i64::from(a));
                put_signed(&mut self.bytes, i64::from(b));
            }
            Event::Items(d) => {
                put_varint(&mut self.bytes, d.len() as u64);
                self.bytes.extend_from_slice(d.as_bytes());
            }
            _ => {}
        }
        self.last = at;
        self.count += 1;
    }

    /// Start a new tick: no events, the input phase, measured from 0, 0, 0.
    pub fn clear(&mut self) {
        self.bytes.clear();
        self.sets.clear();
        self.count = 0;
        self.last = [0; 3];
        self.phase = 0;
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl Reader<'_> {
    fn byte(&mut self) -> u8 {
        self.at += 1;
        self.bytes[self.at - 1]
    }

    fn varint(&mut self) -> u64 {
        let mut v = 0u64;
        let mut shift = 0;
        loop {
            let b = self.byte();
            v |= u64::from(b & 0x7f) << shift;
            if b < 0x80 {
                return v;
            }
            shift += 7;
        }
    }

    fn signed(&mut self) -> i64 {
        let v = self.varint();
        (v >> 1) as i64 ^ -((v & 1) as i64)
    }
}

/// The events in one tick's packed bytes, as `(phase, event, position)`.
pub fn unpack_events(bytes: &[u8]) -> Vec<(Phase, Event, Pos)> {
    let mut r = Reader { bytes, at: 0 };
    let mut out = Vec::new();
    let mut last = [0i32; 3];
    let mut phase = Phase::Input;
    while r.at < bytes.len() {
        let head = r.byte();
        let (code, kind) = (head >> 3, head & 7);
        if code == NEW_PHASE {
            phase = PHASES[r.byte() as usize];
            continue;
        }
        let d = if code == FAR { [0; 3].map(|_| r.signed() as i32) } else { STEPS[code as usize] };
        last = [0, 1, 2].map(|i| last[i].wrapping_add(d[i]));
        let event = match KINDS[kind as usize] {
            Kind::Set => Event::Set(StateId(r.varint() as u32)),
            Kind::Neighbor => Event::Neighbor,
            Kind::Shape => Event::Shape,
            Kind::Schedule => Event::Schedule { delay: r.signed(), priority: r.signed() as i32 },
            Kind::Tick => Event::Tick,
            Kind::BlockEventQueued => Event::BlockEventQueued,
            Kind::BlockEvent => Event::BlockEvent { a: r.signed() as i32, b: r.signed() as i32 },
            _ => {
                let len = r.varint() as usize;
                let text = String::from_utf8_lossy(&bytes[r.at..r.at + len]).into_owned();
                r.at += len;
                Event::Items(text)
            }
        };
        out.push((phase, event, Pos::new(last[0], last[1], last[2])));
    }
    out
}

/// One tick's packed events as trace entries, at world positions.
pub fn unpack(bytes: &[u8], tick: u32, reg: &Registry) -> Vec<Entry> {
    unpack_events(bytes).into_iter().map(|(phase, event, p)| event.entry(reg, tick, phase, [p.x, p.y, p.z])).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_unpack_as_they_were_packed() {
        let events = vec![
            (Phase::Input, Event::Set(StateId(5)), Pos::new(3, 64, -7)),
            (Phase::Input, Event::Neighbor, Pos::new(2, 64, -7)),
            (Phase::Input, Event::Neighbor, Pos::new(4, 64, -7)),
            (Phase::Input, Event::Shape, Pos::new(4, 64, -7)),
            (Phase::BlockTicks, Event::Tick, Pos::new(i32::MIN, i32::MAX, 0)),
            (Phase::BlockTicks, Event::Schedule { delay: -3, priority: -1 }, Pos::new(i32::MAX, i32::MIN, 1)),
            (Phase::BlockEvents, Event::BlockEventQueued, Pos::new(5, 5, 5)),
            (Phase::BlockEvents, Event::BlockEvent { a: 1, b: -2 }, Pos::new(5, 6, 6)),
            (Phase::BlockEntities, Event::Items("0:minecraft:stone*3 4:minecraft:dirt*1".into()), Pos::new(5, 6, 7)),
            (Phase::Other, Event::Set(StateId(u32::MAX)), Pos::new(4, 7, 7)),
        ];
        let mut p = Packer::default();
        for (phase, e, pos) in &events {
            p.record(*phase, e.clone(), *pos);
        }
        assert_eq!(p.count, events.len());
        assert_eq!(p.sets, [(Pos::new(3, 64, -7), StateId(5)), (Pos::new(4, 7, 7), StateId(u32::MAX))]);
        assert_eq!(unpack_events(&p.bytes), events);
        // A neighbor update next to the last event is one byte.
        let before = p.bytes.len();
        p.record(Phase::Other, Event::Neighbor, Pos::new(4, 7, 6));
        assert_eq!(p.bytes.len(), before + 1);
        p.clear();
        assert!(p.bytes.is_empty() && p.sets.is_empty() && p.count == 0);
        p.record(Phase::Input, Event::Neighbor, Pos::new(0, 0, 1));
        assert_eq!(p.bytes, [6 << 3 | 1]);
    }

    #[test]
    fn every_short_step_has_one_code() {
        for (i, &s) in STEPS.iter().enumerate() {
            assert_eq!(step_code(s), i as u8, "{s:?}");
        }
        assert_eq!(STEP_CODES.iter().filter(|&&c| c != NONE).count(), STEPS.len());
        assert_eq!(step_code([1, 1, 1]), NONE);
        assert_eq!(step_code([3, 0, 0]), NONE);
    }
}
