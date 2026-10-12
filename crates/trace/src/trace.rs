//! Traces: what happened, tick by tick, when a fixture ran.
//!
//! The capture harness writes one from the real game and the engine writes
//! one from a simulation; [`crate::diff`] compares them. A trace is a flat,
//! ordered list of entries. Each entry says which tick and phase it happened
//! in and what kind of thing it was, so one list serves all three levels of
//! comparison.

use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};

use crate::fixture::Level;
use crate::state;

pub const TRACE_FORMAT: u32 = 1;

/// Tick phases, in the order 26.3 runs them. Player input lands in `input`,
/// before the world tick. Unknown phases from future captures parse as
/// `Other` rather than failing.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    Input,
    BlockTicks,
    FluidTicks,
    Raids,
    Chunks,
    BlockEvents,
    Entities,
    BlockEntities,
    #[serde(other)]
    Other,
}

/// What an entry records.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    /// A block changed state. `state` is the new state.
    Set,
    /// A neighbor-changed update was delivered to `pos`.
    Neighbor,
    /// A shape update was delivered to `pos`.
    Shape,
    /// A comparator (analog output) update was delivered to `pos`.
    Comparator,
    /// A scheduled tick was requested for `pos`. `detail` holds delay and priority.
    Schedule,
    /// A scheduled tick ran at `pos`.
    Tick,
    /// A block event was queued for `pos`. `detail` holds its id and parameter.
    BlockEventQueued,
    /// A block event ran at `pos`.
    BlockEvent,
    #[serde(other)]
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    pub tick: u32,
    pub phase: Phase,
    pub kind: Kind,
    /// Relative to the fixture origin. May be negative inside the margin.
    pub pos: [i32; 3],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trace {
    pub format: u32,
    /// Who produced it, such as `vanilla` or `tickwright 0.1.0`.
    pub source: String,
    pub mc_version: String,
    /// The resolved fixture id, `<name>@<transform>`.
    pub fixture: String,
    pub origin: [i32; 3],
    pub ticks: u32,
    /// The levels this trace has complete data for.
    pub levels: Vec<Level>,
    /// Every non-air block in the observed box before tick 1, as
    /// `[x, y, z, state]` relative to the origin. Checks that both sides
    /// started from the same world, and gives unchanged blocks their state.
    pub initial: Vec<(i32, i32, i32, String)>,
    pub entries: Vec<Entry>,
}

impl Trace {
    pub fn from_json(text: &str) -> Result<Self> {
        let mut t: Trace = serde_json::from_str(text)?;
        ensure!(t.format == TRACE_FORMAT, "trace format {} is not supported (expected {TRACE_FORMAT})", t.format);
        t.canonicalize()?;
        Ok(t)
    }

    /// Rewrite every state in canonical form so traces from the game (which
    /// prints properties in definition order) compare with hand-written ones.
    pub fn canonicalize(&mut self) -> Result<()> {
        for (i, b) in self.initial.iter_mut().enumerate() {
            b.3 = state::canonical(&b.3).with_context(|| format!("initial block {i}"))?;
        }
        for (i, e) in self.entries.iter_mut().enumerate() {
            if let Some(s) = &e.state {
                e.state = Some(state::canonical(s).with_context(|| format!("entry {i}"))?);
            }
            ensure!(e.tick <= self.ticks, "entry {i} is at tick {} but the trace has {} ticks", e.tick, self.ticks);
        }
        Ok(())
    }

    pub fn has_level(&self, level: Level) -> bool {
        self.levels.contains(&level)
    }
}
