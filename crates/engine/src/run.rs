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
use crate::rules::Rules;

/// How the harness places fixture blocks.
const QUIET: i32 = flags::CLIENTS | flags::KNOWN_SHAPE;

pub fn run(fixture: &ResolvedFixture, mc_version: &str) -> Result<Trace> {
    let mut level = place(fixture, Rules::for_version(mc_version)?)?;
    let reg = level.reg;
    let [ox, oy, oz] = fixture.origin;
    let origin = Pos::new(ox, oy, oz);
    let m = fixture.margin;
    let [sx, sy, sz] = fixture.size;
    let min = Pos::new(ox - m, oy - m, oz - m);
    let max = Pos::new(ox + sx - 1 + m, oy + sy - 1 + m, oz + sz - 1 + m);

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
        apply_stimuli(&mut level, fixture, t)?;
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

/// Run a fixture without recording anything, for benchmarks.
pub fn run_unrecorded(fixture: &ResolvedFixture, mc_version: &str) -> Result<()> {
    let mut level = place(fixture, Rules::for_version(mc_version)?)?;
    for t in 1..=fixture.ticks {
        apply_stimuli(&mut level, fixture, t)?;
        level.tick();
    }
    Ok(())
}

/// A level with the fixture placed the way the harness places it.
fn place(fixture: &ResolvedFixture, rules: Rules) -> Result<Level> {
    let reg = rules.reg;
    let [ox, oy, oz] = fixture.origin;
    let supported = |text: &str| -> Result<crate::registry::StateId> {
        let state = reg.parse(text).with_context(|| format!("{}: block {text}", fixture.id))?;
        if rules.kind_of(state) == Kind::Other {
            bail!("{}: {} is not supported by the engine yet", fixture.id, reg.name(state));
        }
        Ok(state)
    };
    for s in fixture.stimuli.iter().filter_map(|s| s.state.as_deref()) {
        supported(s)?;
    }
    let blocks = fixture.blocks.iter().map(|(x, y, z, s)| Ok((Pos::new(ox + x, oy + y, oz + z), supported(s)?)));
    let blocks: Result<Vec<_>> = blocks.collect();
    let mut level = Level::new(rules);
    for (pos, state) in blocks? {
        level.set_block(pos, state, QUIET);
    }
    // The harness fills containers with `Container.setItem`, slot by slot.
    for c in &fixture.containers {
        let pos = Pos::new(ox + c.pos[0], oy + c.pos[1], oz + c.pos[2]);
        if !matches!(level.kind_at(pos), Kind::Chest | Kind::Barrel | Kind::Hopper | Kind::Dropper | Kind::Dispenser) {
            bail!("{}: no container at {:?}", fixture.id, c.pos);
        }
        for item in &c.items {
            if level.kind_at(pos) == Kind::Dispenser && !blocks::dispenser::supported_item(&item.id) {
                bail!("{}: dispensing {} is not supported by the engine yet", fixture.id, item.id);
            }
            let stack = blocks::container::Stack::new(&item.id, item.count as i32);
            blocks::container::set_item(&mut level, pos, item.slot as usize, stack);
        }
    }
    Ok(level)
}

/// Apply the stimuli for game tick `t`, before the world tick runs.
fn apply_stimuli(level: &mut Level, fixture: &ResolvedFixture, t: u32) -> Result<()> {
    let [ox, oy, oz] = fixture.origin;
    for s in fixture.stimuli.iter().filter(|s| s.tick == t) {
        let [x, y, z] = s.pos;
        let pos = Pos::new(ox + x, oy + y, oz + z);
        match s.action {
            Action::Use => {
                let state = level.get(pos);
                blocks::use_without_item(level, pos, state);
            }
            Action::Set => {
                let text = s.state.as_deref().context("set stimulus without a state")?;
                level.set_block(pos, level.reg.parse(text)?, flags::ALL);
            }
            Action::Destroy => {
                level.destroy_block(pos, crate::level::MAX_RECURSION);
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use tickwright_trace::{FixtureSource, Transform};

    #[test]
    fn unsupported_blocks_fail_by_name() {
        let source = FixtureSource::from_json(
            r##"{"name": "t", "ticks": 1, "palette": {"#": "stone", "h": "furnace"}, "layers": [["#h"]], "stimuli": []}"##,
        )
        .unwrap();
        let fixture = source.resolve(Transform::IDENTITY).unwrap();
        let err = super::run(&fixture, "26.3").unwrap_err().to_string();
        assert!(err.contains("minecraft:furnace is not supported"), "{err}");
    }
}
