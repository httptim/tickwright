//! Running a fixture the way the capture harness runs it in the game.
//!
//! The harness clears the fixture's box, places each block with
//! `setBlock(pos, state, UPDATE_CLIENTS | UPDATE_KNOWN_SHAPE)` in y, z, x
//! order, snapshots the box, and then for each tick applies that tick's
//! stimuli before running the world tick. Placement still runs each block's
//! `onPlace`, which can schedule ticks and queue block events that land
//! during the recording, so the engine places blocks the same way.

use anyhow::{Context, Result, bail};
use tickwright_trace::{Action, Level as TraceLevel, ResolvedFixture, TRACE_FORMAT, Trace, Unsupported};

use crate::blocks::{self, Kind};
use crate::entity;
use crate::level::{Level, flags};
use crate::physics::Vec3;
use crate::pos::Pos;
use crate::record::{Recorder, TraceRecorder};
use crate::rules::Rules;

/// How the harness places fixture blocks.
const QUIET: i32 = flags::CLIENTS | flags::KNOWN_SHAPE;

/// Run a fixture and record its trace. Fails if the build needs something
/// the engine doesn't simulate, so fixture tests never compare a partial
/// trace.
pub fn run(fixture: &ResolvedFixture, mc_version: &str) -> Result<Trace> {
    let trace = run_partial(fixture, mc_version)?;
    if let Some(u) = &trace.unsupported {
        bail!("{}: {}", fixture.id, u.message);
    }
    Ok(trace)
}

/// Like [`run`], but a run that meets something the engine doesn't
/// simulate stops there and still returns its trace: the ticks before that
/// one, with `unsupported` saying which tick and why. A build the engine
/// can't even set up (an unsupported block, a bad stimulus) is still an
/// error.
pub fn run_partial(fixture: &ResolvedFixture, mc_version: &str) -> Result<Trace> {
    if fixture.format != 1 {
        bail!("resolved fixture format {} is not supported", fixture.format);
    }
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

    level.recorder = Some(Recorder::Trace(TraceRecorder::new(origin, min, max)));
    let mut entities = Vec::new();
    let mut rng = Vec::new();
    let mut numbers: Vec<i32> = Vec::new();
    let mut unsupported = None;
    if fixture.entities {
        rng.push((0, level.random.internal()));
    }
    // A build refused as it is placed (an item component the engine doesn't
    // model, say) records no ticks at all.
    if let Some(what) = &level.unsupported {
        unsupported = Some(Unsupported { tick: 1, message: format!("{what} is not simulated by the engine yet") });
    }
    for t in 1..=fixture.ticks {
        if unsupported.is_some() {
            break;
        }
        if let Some(Recorder::Trace(r)) = level.recorder.as_mut() {
            r.tick = t;
        }
        apply_stimuli(&mut level, fixture, t)?;
        level.tick();
        if let Some(what) = &level.unsupported {
            unsupported = Some(Unsupported { tick: t, message: format!("{what} is not simulated by the engine yet") });
            break;
        }
        if fixture.entities {
            snapshot_entities(&level, origin, min, max, t, &mut numbers, &mut entities);
            rng.push((t, level.random.internal()));
        }
    }

    let Some(Recorder::Trace(recorder)) = level.recorder.take() else { unreachable!("a fixture run records a trace") };
    let mut entries = recorder.entries;
    let ticks = unsupported.as_ref().map_or(fixture.ticks, |u| u.tick - 1);
    entries.retain(|e| e.tick <= ticks);
    Ok(Trace {
        format: TRACE_FORMAT,
        source: format!("tickwright {}", env!("CARGO_PKG_VERSION")),
        mc_version: mc_version.to_string(),
        fixture: fixture.id.clone(),
        origin: fixture.origin,
        ticks,
        levels: vec![TraceLevel::State, TraceLevel::Phase, TraceLevel::Order],
        initial,
        entries,
        entities,
        rng,
        unsupported,
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
            Kind::Chest
                | Kind::Barrel
                | Kind::Hopper
                | Kind::Dropper
                | Kind::Dispenser
                | Kind::Crafter
                | Kind::Shelf
                | Kind::ShulkerBox
        ) {
            bail!("{}: no container at {:?}", fixture.id, c.pos);
        }
        for item in &c.items {
            if level.kind_at(pos) == Kind::Dispenser && !blocks::dispenser::supported_item(&item.id) {
                bail!("{}: dispensing {} is not supported by the engine yet", fixture.id, item.id);
            }
            let data =
                blocks::container::ItemData::new(item.components.as_deref(), item.tag.as_deref(), item.data_version)
                    .with_context(|| format!("{}: the item in slot {} at {:?}", fixture.id, item.slot, c.pos))?;
            // Refused the same way as on an entity: the build still opens,
            // marked unsupported from the start.
            if let Some(d) = &data {
                let block = reg.name(level.get(pos));
                let block = block.strip_prefix("minecraft:").unwrap_or(block).replace('_', " ");
                let place = format!("in slot {} of the {block} at {} {} {}", item.slot, c.pos[0], c.pos[1], c.pos[2]);
                if let Some(component) = d.refused_component() {
                    level.unsupported(format!("the item component {component} ({place})"));
                }
                if d.tag.is_some() {
                    level.unsupported(format!("an item tag saved before 1.20.5 ({place})"));
                }
            }
            let stack = blocks::container::Stack::new(&item.id, item.count as i32).with_data(data);
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
        entity::place_start_entity(&mut level, &e.entity, at, motion, e.nbt.as_deref(), e.data_version)
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

/// The update flag a stimulus that discards contents adds.
fn discard(s: &tickwright_trace::fixture::Stimulus) -> i32 {
    if s.discard_contents { flags::SKIP_BLOCK_ENTITY_SIDEEFFECTS } else { 0 }
}

/// Apply the stimuli for game tick `t`, before the world tick runs.
pub(crate) fn apply_stimuli(level: &mut Level, fixture: &ResolvedFixture, t: u32) -> Result<()> {
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
                level.set_block(pos, level.reg.parse(text)?, how | discard(s));
            }
            Action::Destroy => {
                level.destroy_block_with(pos, false, discard(s), crate::level::MAX_RECURSION);
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
                // The harness runs `/summon` with each coordinate written
                // out exactly, and the command centres a whole-number x or z
                // on its block.
                let centre = |v: f64| if v.fract() == 0.0 { v + 0.5 } else { v };
                let at = Vec3::new(centre(ox as f64 + sm.at[0]), oy as f64 + sm.at[1], centre(oz as f64 + sm.at[2]));
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

    #[test]
    fn a_full_shulker_box_is_refused_when_it_drops() {
        let source = FixtureSource::from_json(
            r#"{"name": "t", "ticks": 20, "entities": true,
                "palette": {"o": "obsidian", "s": "shulker_box[facing=up]"},
                "layers": [["ooo"], ["s.."]],
                "containers": [{"pos": [0, 1, 0], "items": [{"slot": 0, "id": "stone", "count": 1}]}],
                "stimuli": [{"tick": 1, "action": "summon", "entity": "tnt", "at": [1.5, 1.0, 0.5], "nbt": "{fuse:5}"}]}"#,
        )
        .unwrap();
        let fixture = source.resolve(Transform::IDENTITY).unwrap();
        let u = super::run_partial(&fixture, "26.3").unwrap().unsupported.unwrap();
        assert_eq!(u.message, "dropping minecraft:shulker_box with what it holds is not simulated by the engine yet");
    }

    #[test]
    fn a_refused_stack_size_keeps_its_count() {
        let source = FixtureSource::from_json(
            r#"{"name": "t", "ticks": 1, "palette": {"c": "chest[facing=north,type=single,waterlogged=false]"},
                "layers": [["c"]],
                "containers": [{"pos": [0, 0, 0], "items": [
                    {"slot": 0, "id": "snowball", "count": 20, "components": "{\"minecraft:max_stack_size\":64}"},
                    {"slot": 1, "id": "snowball", "count": 20}]}]}"#,
        )
        .unwrap();
        let fixture = source.resolve(Transform::IDENTITY).unwrap();
        let level = super::place(&fixture, crate::rules::Rules::for_version("26.3").unwrap()).unwrap();
        let items = crate::blocks::container::items(&level, crate::pos::Pos::new(0, 100, 0)).unwrap();
        assert_eq!((items[0].count, items[1].count), (20, 16));
        assert!(level.unsupported.as_deref().unwrap().contains("max_stack_size"));
    }

    #[test]
    fn a_partial_run_stops_before_the_unsupported_tick() {
        let source = FixtureSource::from_json(
            r###"{"name": "t", "ticks": 10, "entities": true, "palette": {"#": "stone", "f": "fire"},
                "layers": [["##"], ["f."]],
                "stimuli": [
                    {"tick": 3, "action": "summon", "entity": "item", "at": [0.5, 1.0, 0.5],
                     "nbt": "{Item:{id:\"stone\",count:1}}"}
                ]}"###,
        )
        .unwrap();
        let fixture = source.resolve(Transform::IDENTITY).unwrap();
        let trace = super::run_partial(&fixture, "26.3").unwrap();
        let u = trace.unsupported.as_ref().unwrap();
        assert_eq!(u.tick, 3);
        assert_eq!(u.message, "an entity in fire is not simulated by the engine yet");
        assert_eq!(trace.ticks, 2);
        assert!(trace.entries.iter().all(|e| e.tick <= 2));
        assert_eq!(trace.rng.last().map(|r| r.0), Some(2));
        // A full run refuses the same fixture with the same sentence.
        let err = super::run(&fixture, "26.3").unwrap_err().to_string();
        assert_eq!(err, format!("{}: {}", fixture.id, u.message));
    }

    #[test]
    fn a_container_removed_among_entities_spills_what_it_holds() {
        // `preRemoveSideEffects` (`Containers.dropContents`): a filled chest
        // spills its items as entities when it goes, an empty one only draws
        // from the level's random source, and one removed as `/setblock`
        // removes it does neither.
        let run = |items: &str, discard: bool| {
            let json = format!(
                r###"{{"name": "t", "ticks": 4, "entities": true,
                    "palette": {{"o": "stone", "c": "chest[facing=north,type=single,waterlogged=false]"}},
                    "layers": [["oo"], ["c."]],
                    "stimuli": [{{"tick": 2, "action": "set", "pos": [0, 1, 0], "state": "air",
                        "discard_contents": {discard}}}],
                    "containers": [{{"pos": [0, 1, 0], "items": [{items}]}}]}}"###
            );
            let fixture = FixtureSource::from_json(&json).unwrap().resolve(Transform::IDENTITY).unwrap();
            super::run(&fixture, "26.3").unwrap()
        };
        let stone = r#"{"slot": 0, "id": "stone", "count": 64}"#;
        let spilled = run(stone, false);
        let counts: Vec<&str> =
            spilled.entities.iter().filter(|e| e.tick == 2).filter_map(|e| e.detail.as_deref()).collect();
        let total: i32 = counts.iter().map(|d| d.rsplit('*').next().unwrap().parse::<i32>().unwrap()).sum();
        assert_eq!(total, 64, "{counts:?}");
        assert!(counts.len() >= 3, "64 stone fly out in stacks of 10 to 30: {counts:?}");
        let empty = run("", false);
        assert!(empty.entities.is_empty());
        let quiet = run("", true);
        assert!(quiet.entities.is_empty());
        let draws = |t: &tickwright_trace::Trace| t.rng.iter().find(|r| r.0 == 2).unwrap().1;
        assert_ne!(draws(&empty), draws(&quiet), "an empty chest still draws a spot for each slot");
        assert!(run(stone, true).entities.is_empty());
    }

    #[test]
    fn a_lectern_removed_with_its_book_among_entities_is_refused() {
        let source = FixtureSource::from_json(
            r###"{"name": "t", "ticks": 4, "entities": true,
                "palette": {"o": "stone", "l": "lectern[facing=north,has_book=true,powered=false]"},
                "layers": [["oo"], ["l."]],
                "stimuli": [{"tick": 2, "action": "destroy", "pos": [0, 1, 0]}],
                "lecterns": [{"pos": [0, 1, 0], "pages": 3}]}"###,
        )
        .unwrap();
        let trace = super::run_partial(&source.resolve(Transform::IDENTITY).unwrap(), "26.3").unwrap();
        let u = trace.unsupported.as_ref().unwrap();
        assert_eq!(
            (u.tick, u.message.as_str()),
            (2, "removing a lectern with its book is not simulated by the engine yet")
        );
    }

    #[test]
    fn an_item_a_dispenser_cannot_dispense_is_refused_when_fired() {
        // A hopper brings TNT into the dispenser; firing it would prime TNT.
        let source = FixtureSource::from_json(
            r###"{"name": "t", "ticks": 30,
                "palette": {"#": "stone", "h": "hopper[enabled=true,facing=down]",
                    "d": "dispenser[facing=east,triggered=false]"},
                "layers": [["#."], ["d."], ["h."]],
                "stimuli": [{"tick": 20, "action": "set", "pos": [1, 1, 0], "state": "redstone_block"}],
                "containers": [{"pos": [0, 2, 0], "items": [{"slot": 0, "id": "tnt", "count": 1}]}]}"###,
        )
        .unwrap();
        let trace = super::run_partial(&source.resolve(Transform::IDENTITY).unwrap(), "26.3").unwrap();
        let u = trace.unsupported.as_ref().unwrap();
        assert_eq!(u.tick, 23);
        assert_eq!(u.message, "dispensing minecraft:tnt is not simulated by the engine yet");
    }

    #[test]
    fn an_entity_in_powder_snow_is_refused() {
        // An item dropped five blocks onto three blocks of powder snow.
        let source = FixtureSource::from_json(
            r###"{"name": "t", "ticks": 30, "entities": true, "palette": {"#": "stone", "p": "powder_snow"},
                "layers": [["#"], ["p"], ["p"], ["p"], ["."], ["."], ["."], ["."], ["."]],
                "stimuli": [
                    {"tick": 2, "action": "summon", "entity": "item", "at": [0.5, 9.0, 0.5],
                     "nbt": "{Item:{id:\"stone\",count:1}}"}
                ]}"###,
        )
        .unwrap();
        let fixture = source.resolve(Transform::IDENTITY).unwrap();
        let trace = super::run_partial(&fixture, "26.3").unwrap();
        let u = trace.unsupported.as_ref().unwrap();
        assert_eq!(u.message, "an entity in powder snow is not simulated by the engine yet");
    }
}
