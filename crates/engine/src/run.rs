//! Running a fixture the way the capture harness runs it in the game.
//!
//! The harness clears the fixture's box, places each block with
//! `setBlock(pos, state, UPDATE_CLIENTS | UPDATE_KNOWN_SHAPE)` in y, z, x
//! order, snapshots the box, and then for each tick applies that tick's
//! stimuli before running the world tick. Placement still runs each block's
//! `onPlace`, which can schedule ticks and queue block events that land
//! during the recording, so the engine places blocks the same way.

use anyhow::{Context, Result, bail};
use tickwright_trace::{Action, Level as TraceLevel, ResolvedFixture, TRACE_FORMAT, Trace};

use crate::blocks::{self, Kind};
use crate::level::{Level, flags};
use crate::pos::Pos;
use crate::record::Recorder;
use crate::registry::Registry;

/// How the harness places fixture blocks.
const QUIET: i32 = flags::CLIENTS | flags::KNOWN_SHAPE;

pub fn run(fixture: &ResolvedFixture, mc_version: &str) -> Result<Trace> {
    let reg = Registry::get(mc_version)?;
    let mut level = Level::new(reg);
    let [ox, oy, oz] = fixture.origin;
    let origin = Pos::new(ox, oy, oz);
    let m = fixture.margin;
    let [sx, sy, sz] = fixture.size;
    let min = Pos::new(ox - m, oy - m, oz - m);
    let max = Pos::new(ox + sx - 1 + m, oy + sy - 1 + m, oz + sz - 1 + m);

    let supported = |text: &str| -> Result<crate::registry::StateId> {
        let state = reg.parse(text).with_context(|| format!("{}: block {text}", fixture.id))?;
        if Kind::of(reg.name(state)) == Kind::Other {
            bail!("{}: {} is not supported by the engine yet", fixture.id, reg.name(state));
        }
        Ok(state)
    };
    for s in fixture.stimuli.iter().filter_map(|s| s.state.as_deref()) {
        supported(s)?;
    }
    for (x, y, z, s) in &fixture.blocks {
        let state = supported(s)?;
        level.set_block(Pos::new(ox + x, oy + y, oz + z), state, QUIET);
    }

    let mut initial: Vec<(i32, i32, i32, String)> = level
        .non_air()
        .filter(|(p, _)| {
            (min.x..=max.x).contains(&p.x) && (min.y..=max.y).contains(&p.y) && (min.z..=max.z).contains(&p.z)
        })
        .map(|(p, s)| (p.x - ox, p.y - oy, p.z - oz, reg.format(s)))
        .collect();
    initial.sort();

    level.recorder = Some(Recorder::new(origin, min, max));
    for t in 1..=fixture.ticks {
        level.recorder.as_mut().unwrap().tick = t;
        for s in fixture.stimuli.iter().filter(|s| s.tick == t) {
            let [x, y, z] = s.pos;
            let pos = Pos::new(ox + x, oy + y, oz + z);
            match s.action {
                Action::Use => {
                    let state = level.get(pos);
                    blocks::use_without_item(&mut level, pos, state);
                }
                Action::Set => {
                    let text = s.state.as_deref().context("set stimulus without a state")?;
                    level.set_block(pos, reg.parse(text)?, flags::ALL);
                }
                Action::Destroy => {
                    level.destroy_block(pos, crate::level::MAX_RECURSION);
                }
            }
        }
        level.tick();
    }

    let entries = level.recorder.take().unwrap().entries;
    if fixture.format != 1 {
        bail!("resolved fixture format {} is not supported", fixture.format);
    }
    Ok(Trace {
        format: TRACE_FORMAT,
        source: format!("tickwright {}", env!("CARGO_PKG_VERSION")),
        mc_version: mc_version.to_string(),
        fixture: fixture.id.clone(),
        origin: fixture.origin,
        ticks: fixture.ticks,
        levels: vec![TraceLevel::State, TraceLevel::Phase, TraceLevel::Order],
        initial,
        entries,
    })
}

#[cfg(test)]
mod tests {
    use tickwright_trace::{FixtureSource, Transform};

    #[test]
    fn unsupported_blocks_fail_by_name() {
        let source = FixtureSource::from_json(
            r##"{"name": "t", "ticks": 1, "palette": {"#": "stone", "h": "hopper"}, "layers": [["#h"]], "stimuli": []}"##,
        )
        .unwrap();
        let fixture = source.resolve(Transform::IDENTITY).unwrap();
        let err = super::run(&fixture, "26.3").unwrap_err().to_string();
        assert!(err.contains("minecraft:hopper is not supported"), "{err}");
    }
}
