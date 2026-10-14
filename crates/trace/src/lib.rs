//! Fixture and trace formats for Tickwright's ground-truth tests.
//!
//! A [fixture] is a small build plus the inputs applied to it. The
//! capture harness runs each fixture in the real game and writes a
//! [trace]; the engine runs the same fixture and writes its own; and
//! [`diff::compare`] decides whether they match. "Accurate" in Tickwright
//! means: matches every captured trace.

pub mod diff;
pub mod fixture;
pub mod state;
pub mod trace;
pub mod transform;

pub use diff::{Divergence, compare};
pub use fixture::{Action, FixtureSource, Level, ResolvedFixture, Stimulus};
pub use state::BlockState;
pub use trace::{EntityRow, Entry, Kind, Phase, TRACE_FORMAT, Trace, Unsupported};
pub use transform::Transform;
