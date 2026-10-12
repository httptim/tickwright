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
use crate::entity;
use crate::level::{Level, flags};
use crate::physics::Vec3;
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
    let mut entities = Vec::new();
    let mut rng = Vec::new();
    let mut numbers: Vec<i32> = Vec::new();
    if fixture.entities {
        rng.push((0, level.random.internal()));
    }
    for t in 1..=fixture.ticks {
        level.recorder.as_mut().unwrap().tick = t;
        apply_stimuli(&mut level, fixture, t)?;
        level.tick();
        if let Some(what) = &level.unsupported {
            bail!("{}: {what} is not simulated by the engine yet", fixture.id);
        }
        if fixture.entities {
            snapshot_entities(&level, origin, min, max, t, &mut numbers, &mut entities);
            rng.push((t, level.random.internal()));
        }
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
        entities,
        rng,
    })
}

/// What the harness records after each tick: every entity whose box meets
/// the fixture's, by id, numbered in the order they first appeared.
fn snapshot_entities(
    level: &Level,
    origin: Pos,
    min: Pos,
    max: Pos,
    tick: u32,
    numbers: &mut Vec<i32>,
    out: &mut Vec<tickwright_trace::EntityRow>,
) {
    for e in entity::entities_in(level, min, max) {
        let n = match numbers.iter().position(|&id| id == e.id) {
            Some(n) => n,
            None => {
                numbers.push(e.id);
                numbers.len() - 1
            }
        };
        out.push(tickwright_trace::EntityRow {
            tick,
            n: n as u32,
            id: e.id,
            kind: e.type_name().to_string(),
            pos: [e.pos.x - origin.x as f64, e.pos.y - origin.y as f64, e.pos.z - origin.z as f64],
            motion: [e.motion.x, e.motion.y, e.motion.z],
            detail: e.detail(level),
        });
    }
}

/// `String.hashCode`.
fn java_string_hash(s: &str) -> i32 {
    s.encode_utf16().fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32))
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
pub(crate) fn place(fixture: &ResolvedFixture, rules: Rules) -> Result<Level> {
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
    // Without a frozen clock the day time is whatever the game's world had.
    let uses_day_time = |s: &str| s.starts_with("daylight_detector") || s.starts_with("minecraft:daylight_detector");
    if fixture.day_time.is_none() && fixture.blocks.iter().any(|b| uses_day_time(&b.3)) {
        bail!("{}: a daylight detector needs the fixture's day_time", fixture.id);
    }
    if !fixture.entities && fixture.stimuli.iter().any(|s| s.action == Action::Summon) {
        bail!("{}: summoning needs \"entities\": true", fixture.id);
    }
    let mut level = Level::new(rules);
    if fixture.entities {
        // The harness seeds the level's random source with the fixture's id
        // and numbers entities from 1.
        level.simulate_entities = true;
        level.random.set_seed(java_string_hash(&fixture.id) as i64);
        level.entity_counter = 0;
    }
    if let Some(t) = fixture.day_time {
        level.day_time = t;
    }
    for e in &fixture.experiments {
        match e.as_str() {
            "redstone_experiments" => level.experimental_redstone = true,
            other => bail!("{}: the {other} experiment is not supported by the engine yet", fixture.id),
        }
    }
    if let Some(n) = fixture.max_chained_updates {
        level.max_chained_updates = n as usize;
    }
    for (pos, state) in blocks? {
        level.set_block(pos, state, QUIET);
    }
    // The harness fills containers with `Container.setItem`, slot by slot.
    for c in &fixture.containers {
        let pos = Pos::new(ox + c.pos[0], oy + c.pos[1], oz + c.pos[2]);
        if !matches!(
            level.kind_at(pos),
            Kind::Chest | Kind::Barrel | Kind::Hopper | Kind::Dropper | Kind::Dispenser | Kind::Crafter | Kind::Shelf
        ) {
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
    for c in &fixture.comparators {
        let pos = Pos::new(ox + c.pos[0], oy + c.pos[1], oz + c.pos[2]);
        match level.block_entities.get_mut(&pos) {
            Some(blocks::BlockEntity::Comparator(output)) => *output = c.output,
            _ => bail!("{}: no comparator at {:?}", fixture.id, c.pos),
        }
    }
    // Books on lecterns: `setBook`, then `setPage`.
    for l in &fixture.lecterns {
        let pos = Pos::new(ox + l.pos[0], oy + l.pos[1], oz + l.pos[2]);
        if level.kind_at(pos) != Kind::Lectern {
            bail!("{}: no lectern at {:?}", fixture.id, l.pos);
        }
        blocks::lectern::set_book(&mut level, pos, l.pages as i32);
        blocks::lectern::set_page(&mut level, pos, l.page as i32);
    }
    // Entities that come with the build: the ones the engine simulates are
    // created before the first tick; the rest are carried in the fixture
    // only.
    for e in &fixture.start_entities {
        let at = Vec3::new(ox as f64 + e.at[0], oy as f64 + e.at[1], oz as f64 + e.at[2]);
        let motion = e.motion.map(|m| Vec3::new(m[0], m[1], m[2]));
        entity::place_start_entity(&mut level, &e.entity, at, motion, e.nbt.as_deref())
            .with_context(|| format!("{}: entity {}", fixture.id, e.entity))?;
    }
    Ok(level)
}

/// A resolved place stimulus as a click on the block at `pos`.
pub fn click(pos: Pos, p: &tickwright_trace::fixture::PlaceInput) -> Result<blocks::placement::Click> {
    let dir = |s: &str| crate::pos::Dir::from_name(s).with_context(|| format!("{s} is not a direction"));
    Ok(blocks::placement::Click {
        clicked: pos,
        face: dir(&p.face)?,
        hit: p.hit_or_default(),
        look: dir(&p.look)?,
        pitch: p.pitch,
        sneak: p.sneak,
    })
}

/// Apply the stimuli for game tick `t`, before the world tick runs.
fn apply_stimuli(level: &mut Level, fixture: &ResolvedFixture, t: u32) -> Result<()> {
    let [ox, oy, oz] = fixture.origin;
    for s in fixture.stimuli.iter().filter(|s| s.tick == t) {
        let [x, y, z] = s.pos;
        let pos = Pos::new(ox + x, oy + y, oz + z);
        if matches!(s.action, Action::Use | Action::Place) {
            level.new_clicking_player();
        }
        match s.action {
            Action::Use => {
                let state = level.get(pos);
                blocks::use_without_item(level, pos, state);
            }
            Action::Set => {
                let text = s.state.as_deref().context("set stimulus without a state")?;
                let how = if s.quiet { flags::CLIENTS | flags::KNOWN_SHAPE } else { flags::ALL };
                level.set_block(pos, level.reg.parse(text)?, how);
            }
            Action::Destroy => {
                level.destroy_block(pos, false, crate::level::MAX_RECURSION);
            }
            Action::Place => {
                let p = s.place.as_ref().context("place stimulus without item, face and look")?;
                blocks::placement::place(level, &p.item, click(pos, p)?).with_context(|| fixture.id.clone())?;
            }
            Action::Page => {
                blocks::lectern::set_page(level, pos, s.page.context("page stimulus without a page")? as i32)
            }
            Action::Time => level.day_time = s.time.context("time stimulus without a time")?,
            Action::Summon => {
                let sm = s.summon.as_ref().context("summon stimulus without an entity")?;
                let at = Vec3::new(ox as f64 + sm.at[0], oy as f64 + sm.at[1], oz as f64 + sm.at[2]);
                let motion = sm.motion.map(|m| Vec3::new(m[0], m[1], m[2]));
                entity::summon(level, &sm.entity, at, motion, sm.nbt.as_deref()).with_context(|| fixture.id.clone())?;
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
