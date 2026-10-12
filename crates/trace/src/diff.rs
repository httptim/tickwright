//! Comparing an expected trace (from the game) with an actual one.
//!
//! Each level is stricter than the one before:
//!
//! - **State:** after every tick, every block that has changed holds the
//!   same state in both traces.
//! - **Phase:** within every tick, every block goes through the same states
//!   in the same phases, in the same order.
//! - **Order:** within every tick, every recorded event (state changes,
//!   updates, scheduled ticks, block events) happens in the same sequence.
//!
//! A diff stops at the first tick that differs, because everything after a
//! divergence is noise.

use std::collections::BTreeMap;
use std::fmt;

use crate::fixture::Level;
use crate::trace::{Entry, Kind, Phase, Trace};

#[derive(Debug, Clone, PartialEq)]
pub struct Divergence {
    pub level: Level,
    pub tick: u32,
    pub lines: Vec<String>,
}

impl fmt::Display for Divergence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "first divergence at tick {} ({:?} level):", self.tick, self.level)?;
        for l in &self.lines {
            writeln!(f, "  {l}")?;
        }
        Ok(())
    }
}

/// How many differing lines to show before giving up.
const MAX_LINES: usize = 12;

/// Compare at `level` and every level below it. `Ok(())` means identical.
pub fn compare(expected: &Trace, actual: &Trace, level: Level) -> Result<(), Divergence> {
    compare_world(expected, actual, 0)?;
    compare_entities_at(expected, actual, 0)?;
    let ticks = expected.ticks.max(actual.ticks);
    for tick in 1..=ticks {
        compare_state(expected, actual, tick)?;
        compare_entities_at(expected, actual, tick)?;
        if level >= Level::Phase {
            compare_phase(expected, actual, tick)?;
        }
        if level >= Level::Order {
            compare_order(expected, actual, tick)?;
        }
    }
    Ok(())
}

/// Compare the entities at the end of every tick, exactly: same entities,
/// types, ids, positions, velocities and details.
pub fn compare_entities(expected: &Trace, actual: &Trace) -> Result<(), Divergence> {
    let ticks = expected.ticks.max(actual.ticks);
    for tick in 1..=ticks {
        compare_entities_at(expected, actual, tick)?;
    }
    Ok(())
}

/// The entities and the level random's seed at the end of `tick`. Traces
/// without entity tracking have neither, so this passes for them.
fn compare_entities_at(expected: &Trace, actual: &Trace, tick: u32) -> Result<(), Divergence> {
    let a: Vec<_> = expected.entities.iter().filter(|e| e.tick == tick).collect();
    let b: Vec<_> = actual.entities.iter().filter(|e| e.tick == tick).collect();
    let mut lines = Vec::new();
    if a != b {
        let ns: std::collections::BTreeSet<u32> = a.iter().chain(&b).map(|e| e.n).collect();
        for n in ns {
            let x = a.iter().find(|e| e.n == n);
            let y = b.iter().find(|e| e.n == n);
            if x != y {
                let show = |e: Option<&&crate::trace::EntityRow>| match e {
                    None => "none".to_string(),
                    Some(e) => format!(
                        "{} #{} at {:?} moving {:?}{}",
                        e.kind,
                        e.id,
                        e.pos,
                        e.motion,
                        e.detail.as_deref().map(|d| format!(" ({d})")).unwrap_or_default()
                    ),
                };
                lines.push(format!("entity {n}: expected {}", show(x)));
                lines.push(format!("entity {n}:      got {}", show(y)));
            }
            if lines.len() >= MAX_LINES {
                break;
            }
        }
    }
    let seed = |t: &Trace| t.rng.iter().find(|r| r.0 == tick).map(|r| r.1);
    if !expected.rng.is_empty() && seed(expected) != seed(actual) {
        lines.push(format!("random seed: expected {:?}, got {:?}", seed(expected), seed(actual)));
    }
    finish(Level::State, tick, lines)
}

const AIR: &str = "minecraft:air";

/// The non-air world at the end of `tick` (tick 0 is the initial snapshot).
fn world_at(trace: &Trace, tick: u32) -> BTreeMap<[i32; 3], &str> {
    let mut world: BTreeMap<[i32; 3], &str> =
        trace.initial.iter().map(|(x, y, z, s)| ([*x, *y, *z], s.as_str())).collect();
    for e in trace.entries.iter().filter(|e| e.tick <= tick && e.kind == Kind::Set) {
        match e.state.as_deref().unwrap_or("?") {
            AIR => world.remove(&e.pos),
            s => world.insert(e.pos, s),
        };
    }
    world
}

fn compare_world(expected: &Trace, actual: &Trace, tick: u32) -> Result<(), Divergence> {
    let a = world_at(expected, tick);
    let b = world_at(actual, tick);
    let mut lines = Vec::new();
    let positions: std::collections::BTreeSet<_> = a.keys().chain(b.keys()).collect();
    for pos in positions {
        let (x, y) = (a.get(pos).copied().unwrap_or(AIR), b.get(pos).copied().unwrap_or(AIR));
        if x != y {
            lines.push(format!("{pos:?}: expected {x}, got {y}"));
        }
    }
    finish(Level::State, tick, lines)
}

/// What each container held at the end of `tick`, for those that changed.
fn items_at(trace: &Trace, tick: u32) -> BTreeMap<[i32; 3], &str> {
    trace
        .entries
        .iter()
        .filter(|e| e.tick <= tick && e.kind == Kind::Items)
        .map(|e| (e.pos, e.detail.as_deref().unwrap_or("")))
        .collect()
}

fn compare_state(expected: &Trace, actual: &Trace, tick: u32) -> Result<(), Divergence> {
    compare_world(expected, actual, tick)?;
    let a = items_at(expected, tick);
    let b = items_at(actual, tick);
    let mut lines = Vec::new();
    let positions: std::collections::BTreeSet<_> = a.keys().chain(b.keys()).collect();
    for pos in positions {
        if a.get(pos) != b.get(pos) {
            let show = |v: Option<&&str>| v.map(|s| format!("[{s}]")).unwrap_or_else(|| "no change".into());
            lines.push(format!("{pos:?} items: expected {}, got {}", show(a.get(pos)), show(b.get(pos))));
        }
    }
    finish(Level::State, tick, lines)
}

fn per_block(trace: &Trace, tick: u32) -> BTreeMap<[i32; 3], Vec<(Phase, &str)>> {
    let mut out: BTreeMap<[i32; 3], Vec<(Phase, &str)>> = BTreeMap::new();
    for e in trace.entries.iter().filter(|e| e.tick == tick && e.kind == Kind::Set) {
        out.entry(e.pos).or_default().push((e.phase, e.state.as_deref().unwrap_or("?")));
    }
    out
}

fn compare_phase(expected: &Trace, actual: &Trace, tick: u32) -> Result<(), Divergence> {
    let a = per_block(expected, tick);
    let b = per_block(actual, tick);
    let mut lines = Vec::new();
    let positions: std::collections::BTreeSet<_> = a.keys().chain(b.keys()).collect();
    let show = |v: Option<&Vec<(Phase, &str)>>| match v {
        None => "nothing".to_string(),
        Some(v) => v.iter().map(|(p, s)| format!("{s} in {p:?}")).collect::<Vec<_>>().join(", then "),
    };
    for pos in positions {
        if a.get(pos) != b.get(pos) {
            lines.push(format!("{pos:?}: expected {}; got {}", show(a.get(pos)), show(b.get(pos))));
        }
    }
    finish(Level::Phase, tick, lines)
}

fn describe(e: &Entry) -> String {
    let mut s = format!("{:?} {:?} {:?}", e.phase, e.kind, e.pos);
    if let Some(st) = &e.state {
        s.push_str(&format!(" -> {st}"));
    }
    if let Some(d) = &e.detail {
        s.push_str(&format!(" ({d})"));
    }
    s
}

fn compare_order(expected: &Trace, actual: &Trace, tick: u32) -> Result<(), Divergence> {
    let a: Vec<&Entry> = expected.entries.iter().filter(|e| e.tick == tick).collect();
    let b: Vec<&Entry> = actual.entries.iter().filter(|e| e.tick == tick).collect();
    let first =
        a.iter().zip(&b).position(|(x, y)| x != y).or_else(|| (a.len() != b.len()).then(|| a.len().min(b.len())));
    let Some(i) = first else { return Ok(()) };
    let mut lines = vec![format!("event #{i} of this tick differs")];
    let from = i.saturating_sub(2);
    for j in from..(i + 3) {
        let x = a.get(j).map(|e| describe(e)).unwrap_or_else(|| "(end)".into());
        let y = b.get(j).map(|e| describe(e)).unwrap_or_else(|| "(end)".into());
        let mark = if j == i { ">" } else { " " };
        lines.push(format!("{mark} #{j} expected: {x}"));
        lines.push(format!("{mark} #{j}   actual: {y}"));
    }
    finish(Level::Order, tick, lines)
}

fn finish(level: Level, tick: u32, mut lines: Vec<String>) -> Result<(), Divergence> {
    if lines.is_empty() {
        return Ok(());
    }
    if lines.len() > MAX_LINES {
        let more = lines.len() - MAX_LINES;
        lines.truncate(MAX_LINES);
        lines.push(format!("... and {more} more"));
    }
    Err(Divergence { level, tick, lines })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace::TRACE_FORMAT;

    fn set(tick: u32, phase: Phase, pos: [i32; 3], state: &str) -> Entry {
        Entry { tick, phase, kind: Kind::Set, pos, state: Some(state.into()), detail: None }
    }

    fn nc(tick: u32, pos: [i32; 3]) -> Entry {
        Entry { tick, phase: Phase::BlockTicks, kind: Kind::Neighbor, pos, state: None, detail: None }
    }

    fn trace(entries: Vec<Entry>) -> Trace {
        Trace {
            format: TRACE_FORMAT,
            source: "test".into(),
            mc_version: "26.3".into(),
            fixture: "t@r0".into(),
            origin: [0, 100, 0],
            ticks: 5,
            levels: vec![Level::State, Level::Phase, Level::Order],
            initial: vec![],
            entries,
            entities: Vec::new(),
            rng: Vec::new(),
            unsupported: None,
        }
    }

    #[test]
    fn identical_traces_match_at_every_level() {
        let t = trace(vec![set(2, Phase::BlockTicks, [0, 0, 0], "minecraft:stone"), nc(2, [1, 0, 0])]);
        assert_eq!(compare(&t, &t.clone(), Level::Order), Ok(()));
    }

    #[test]
    fn late_change_is_a_state_divergence_at_the_first_tick() {
        let a = trace(vec![set(2, Phase::BlockTicks, [0, 0, 0], "minecraft:stone")]);
        let b = trace(vec![set(3, Phase::BlockTicks, [0, 0, 0], "minecraft:stone")]);
        let d = compare(&a, &b, Level::State).unwrap_err();
        assert_eq!((d.level, d.tick), (Level::State, 2));
    }

    #[test]
    fn wrong_phase_passes_state_but_fails_phase() {
        let a = trace(vec![set(2, Phase::BlockTicks, [0, 0, 0], "minecraft:stone")]);
        let b = trace(vec![set(2, Phase::BlockEvents, [0, 0, 0], "minecraft:stone")]);
        assert_eq!(compare(&a, &b, Level::State), Ok(()));
        let d = compare(&a, &b, Level::Phase).unwrap_err();
        assert_eq!(d.level, Level::Phase);
    }

    #[test]
    fn transient_flicker_within_a_tick_is_a_phase_divergence() {
        let a = trace(vec![
            set(2, Phase::BlockTicks, [0, 0, 0], "minecraft:stone"),
            set(2, Phase::BlockEvents, [0, 0, 0], "minecraft:air"),
        ]);
        let b = trace(vec![]);
        assert_eq!(compare(&a, &b, Level::State), Ok(()));
        assert!(compare(&a, &b, Level::Phase).is_err());
    }

    #[test]
    fn different_starting_worlds_diverge_at_tick_zero() {
        let a = trace(vec![]);
        let mut b = trace(vec![]);
        b.initial.push((0, 0, 0, "minecraft:stone".into()));
        assert_eq!(compare(&a, &b, Level::State).unwrap_err().tick, 0);
    }

    #[test]
    fn swapped_updates_fail_only_at_order_level() {
        let a = trace(vec![nc(1, [1, 0, 0]), nc(1, [2, 0, 0])]);
        let b = trace(vec![nc(1, [2, 0, 0]), nc(1, [1, 0, 0])]);
        assert_eq!(compare(&a, &b, Level::Phase), Ok(()));
        let d = compare(&a, &b, Level::Order).unwrap_err();
        assert_eq!((d.level, d.tick), (Level::Order, 1));
        assert!(d.lines[0].contains("#0"));
    }
}
