//! Tickwright's simulation engine.
//!
//! [`run::run`] plays a resolved fixture the way the capture harness plays it
//! in the real game and returns a trace that `tickwright_trace::compare` can
//! check against the game's own.

pub mod bench;
pub mod blocks;
pub mod level;
pub mod pos;
pub mod record;
pub mod registry;
pub mod run;

pub use level::Level;
pub use pos::{Dir, Pos};
pub use registry::{BlockId, Registry, StateId};
