//! Fixtures: a small build, the inputs applied to it, and how long to watch.
//!
//! Fixtures are written by hand as layered ASCII art with a palette, because
//! that is the easiest way to review a redstone build in a pull request:
//!
//! ```json
//! {
//!   "name": "torch_inverter",
//!   "ticks": 10,
//!   "palette": { "#": "stone", "L": "lever[face=floor,facing=north,powered=false]" },
//!   "layers": [
//!     ["###"],
//!     ["L.."]
//!   ],
//!   "stimuli": [ { "tick": 2, "action": "use", "pos": [0, 1, 0] } ]
//! }
//! ```
//!
//! `layers[y][z]` is a row, and each character in it is one block along x.
//! `.` is air. The capture harness reads the *resolved* form instead: one
//! explicit block list per orientation, so it needs no ASCII parsing and no
//! knowledge of how block states rotate.

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::state::BlockState;
use crate::transform::Transform;

/// Where every fixture is placed in the world unless it says otherwise.
///
/// The absolute position matters: default dust updates its neighbors in an
/// order derived from block coordinates, so a trace is only comparable with
/// a simulation run at the same origin.
pub const DEFAULT_ORIGIN: [i32; 3] = [0, 100, 0];

/// Blocks of air cleared and observed around the fixture, so changes it
/// causes just outside its box (a pushed block, a broken torch) are seen.
pub const DEFAULT_MARGIN: i32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// Every block's state at the end of each game tick.
    State,
    /// Each block's changes within a tick, tagged with the tick phase.
    Phase,
    /// Every update, scheduled tick and block event, in the game's order.
    Order,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Right-click the block with an empty hand, as a player would.
    Use,
    /// Set the block state with normal updates, as a player placing it.
    Set,
    /// Break the block without drops, with normal updates.
    Destroy,
    /// Right-click a face of the block at `pos` holding `item`, as a player
    /// looking `look` (and `pitch` degrees down) would, so the block's own
    /// placement rules choose its state.
    Place,
    /// Turn the book on the lectern at `pos` to `page`, as a player
    /// reading it would (`LecternBlockEntity.setPage`).
    Page,
    /// Set the time of day to `time`, as `/time set` does. `pos` is unused.
    Time,
}

/// One stack in a container: `count` of item `id` in `slot`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemSlot {
    pub slot: u32,
    /// An item id such as `stone`; `minecraft:` is implied.
    pub id: String,
    pub count: u32,
}

/// What a container block (chest, barrel, hopper, dropper, dispenser) holds when the fixture is
/// placed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Container {
    pub pos: [i32; 3],
    pub items: Vec<ItemSlot>,
}

/// A lectern holding a book of `pages` pages, open at `page`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Lectern {
    pub pos: [i32; 3],
    pub pages: u32,
    #[serde(default)]
    pub page: u32,
}

/// A comparator's stored output (`ComparatorBlockEntity.output`), which a
/// build can carry from the world it was made in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComparatorOutput {
    pub pos: [i32; 3],
    pub output: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Stimulus {
    /// Applied just before this game tick's world tick runs, which is where
    /// player input lands in the real game. Tick 1 is the first tick.
    pub tick: u32,
    pub action: Action,
    #[serde(default)]
    pub pos: [i32; 3],
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
    /// The page a `page` stimulus turns to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<u32>,
    /// The time of day a `time` stimulus sets, in ticks (0 is sunrise).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time: Option<i64>,
    #[serde(flatten, default, skip_serializing_if = "Option::is_none")]
    pub place: Option<PlaceInput>,
}

/// How a `place` stimulus is made: what is held, which face of the block at
/// the stimulus position is clicked, and where the player is looking.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlaceInput {
    /// The item held, such as `repeater` or `redstone`.
    pub item: String,
    /// The face clicked: `up`, `down`, `north` and so on.
    pub face: String,
    /// The horizontal direction the player faces.
    pub look: String,
    /// Degrees below the horizon the player looks; negative is up.
    #[serde(default)]
    pub pitch: i32,
    /// Where on the clicked block the click lands, in sixteenths of a block
    /// from its least corner. Defaults to the middle of the clicked face.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hit: Option<[i32; 3]>,
    /// Whether the player is sneaking.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub sneak: bool,
}

const DIRECTIONS: [&str; 6] = ["down", "up", "north", "south", "west", "east"];

impl PlaceInput {
    /// The click point in sixteenths, the middle of the face if not given.
    pub fn hit_or_default(&self) -> [i32; 3] {
        self.hit.unwrap_or(match self.face.as_str() {
            "down" => [8, 0, 8],
            "up" => [8, 16, 8],
            "north" => [8, 8, 0],
            "south" => [8, 8, 16],
            "west" => [0, 8, 8],
            _ => [16, 8, 8],
        })
    }

    fn validate(&self) -> Result<()> {
        ensure!(DIRECTIONS.contains(&self.face.as_str()), "face {:?} is not a direction", self.face);
        ensure!(DIRECTIONS[2..].contains(&self.look.as_str()), "look {:?} is not a horizontal direction", self.look);
        ensure!((-90..=90).contains(&self.pitch), "pitch {} is outside -90..=90", self.pitch);
        if let Some(h) = self.hit {
            ensure!(h.iter().all(|v| (0..=16).contains(v)), "hit {h:?} is outside 0..=16");
        }
        Ok(())
    }

    fn transform(&self, t: Transform) -> PlaceInput {
        PlaceInput {
            item: self.item.clone(),
            face: t.direction(&self.face),
            look: t.direction(&self.look),
            pitch: self.pitch,
            // Resolved fixtures always carry the click point.
            hit: Some(t.sixteenths(self.hit_or_default())),
            sneak: self.sneak,
        }
    }
}

/// A fixture as written by hand.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureSource {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Game ticks to record.
    pub ticks: u32,
    #[serde(default = "default_levels")]
    pub levels: Vec<Level>,
    /// Orientations to capture. Defaults to all eight.
    #[serde(default)]
    pub transforms: Option<Vec<String>>,
    #[serde(default)]
    pub origin: Option<[i32; 3]>,
    #[serde(default)]
    pub tags: Vec<String>,
    pub palette: BTreeMap<String, String>,
    pub layers: Vec<Vec<String>>,
    #[serde(default)]
    pub stimuli: Vec<Stimulus>,
    #[serde(default)]
    pub containers: Vec<Container>,
    #[serde(default)]
    pub comparators: Vec<ComparatorOutput>,
    #[serde(default)]
    pub lecterns: Vec<Lectern>,
    /// Feature flags the world is created with, such as
    /// `redstone_experiments`.
    #[serde(default)]
    pub experiments: Vec<String>,
    /// The time of day when the fixture starts. Fixtures that set it start
    /// on a game time that is a multiple of 20, the period of the daylight
    /// detector's update.
    #[serde(default)]
    pub day_time: Option<i64>,
    /// `max-chained-neighbor-updates`, for fixtures about update skipping.
    #[serde(default)]
    pub max_chained_updates: Option<i32>,
}

fn default_levels() -> Vec<Level> {
    vec![Level::State, Level::Phase]
}

/// A fixture in one orientation, with every block spelled out. This is the
/// file the capture harness and the engine both load.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResolvedFixture {
    pub format: u32,
    /// `<name>@<transform>`, also the trace's file stem.
    pub id: String,
    pub name: String,
    pub transform: String,
    pub ticks: u32,
    pub levels: Vec<Level>,
    pub origin: [i32; 3],
    pub size: [i32; 3],
    pub margin: i32,
    /// Non-air blocks as `[x, y, z, state]`, relative to `origin`.
    pub blocks: Vec<(i32, i32, i32, String)>,
    pub stimuli: Vec<Stimulus>,
    /// Container contents, placed after the blocks. Item ids are written in
    /// full (`minecraft:stone`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub containers: Vec<Container>,
    /// Comparator outputs, set after the containers are filled.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comparators: Vec<ComparatorOutput>,
    /// Books on lecterns, set after the comparator outputs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lecterns: Vec<Lectern>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub experiments: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub day_time: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_chained_updates: Option<i32>,
}

/// Feature flags a fixture may ask for.
pub const EXPERIMENTS: [&str; 1] = ["redstone_experiments"];

pub const RESOLVED_FORMAT: u32 = 1;

impl FixtureSource {
    pub fn from_json(text: &str) -> Result<Self> {
        let f: FixtureSource = serde_json::from_str(text)?;
        f.validate()?;
        Ok(f)
    }

    pub fn size(&self) -> [i32; 3] {
        let depth = self.layers.iter().map(Vec::len).max().unwrap_or(0);
        let width = self.layers.iter().flatten().map(|row| row.chars().count()).max().unwrap_or(0);
        [width as i32, self.layers.len() as i32, depth as i32]
    }

    fn validate(&self) -> Result<()> {
        ensure!(!self.name.is_empty(), "fixture has no name");
        ensure!(
            self.name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "fixture name {:?} must be lowercase letters, digits and underscores",
            self.name
        );
        ensure!(self.ticks > 0, "{}: ticks must be at least 1", self.name);
        ensure!(!self.layers.is_empty(), "{}: no layers", self.name);
        let [w, _, d] = self.size();
        for (y, layer) in self.layers.iter().enumerate() {
            ensure!(layer.len() as i32 == d, "{}: layer {y} has {} rows, expected {d}", self.name, layer.len());
            for (z, row) in layer.iter().enumerate() {
                ensure!(
                    row.chars().count() as i32 == w,
                    "{}: layer {y} row {z} is {} wide, expected {w}",
                    self.name,
                    row.chars().count()
                );
                for c in row.chars() {
                    if c != '.' && !self.palette.contains_key(&c.to_string()) {
                        bail!("{}: layer {y} row {z} uses {c:?}, which is not in the palette", self.name);
                    }
                }
            }
        }
        for (key, state) in &self.palette {
            ensure!(key.chars().count() == 1, "{}: palette key {key:?} must be one character", self.name);
            ensure!(key != ".", "{}: '.' is reserved for air", self.name);
            BlockState::parse(state).with_context(|| format!("{}: palette entry {key:?}", self.name))?;
        }
        let size = self.size();
        for s in &self.stimuli {
            ensure!(
                s.tick >= 1 && s.tick <= self.ticks,
                "{}: stimulus at tick {} is outside 1..={}",
                self.name,
                s.tick,
                self.ticks
            );
            ensure!(
                (0..3).all(|i| (0..size[i]).contains(&s.pos[i])),
                "{}: stimulus position {:?} is outside the fixture",
                self.name,
                s.pos
            );
            match (&s.action, &s.place) {
                (Action::Place, None) => bail!("{}: a place stimulus needs item, face and look", self.name),
                (Action::Place, Some(p)) => p.validate().with_context(|| format!("{}: place stimulus", self.name))?,
                (_, Some(_)) => bail!("{}: only place stimuli take item, face and look", self.name),
                _ => {}
            }
            match (&s.action, s.page, s.time) {
                (Action::Page, None, _) => bail!("{}: a page stimulus needs a page", self.name),
                (Action::Time, _, None) => bail!("{}: a time stimulus needs a time", self.name),
                (Action::Page, _, Some(_)) | (Action::Time, Some(_), _) => {
                    bail!("{}: a stimulus takes a page or a time, not both", self.name)
                }
                (Action::Page | Action::Time, _, _) => {}
                (_, Some(_), _) | (_, _, Some(_)) => {
                    bail!("{}: only page and time stimuli take a page or a time", self.name)
                }
                _ => {}
            }
            match (&s.action, &s.state) {
                (Action::Set, None) => bail!("{}: a set stimulus needs a state", self.name),
                (Action::Set, Some(st)) => {
                    BlockState::parse(st)?;
                }
                (_, Some(_)) => bail!("{}: only set stimuli take a state", self.name),
                _ => {}
            }
        }
        for c in &self.containers {
            ensure!(
                (0..3).all(|i| (0..size[i]).contains(&c.pos[i])),
                "{}: container position {:?} is outside the fixture",
                self.name,
                c.pos
            );
            for item in &c.items {
                ensure!(item.count > 0, "{}: container at {:?} has an empty stack", self.name, c.pos);
            }
        }
        for l in &self.lecterns {
            ensure!(
                (0..3).all(|i| (0..size[i]).contains(&l.pos[i])) && l.pages > 0 && l.page < l.pages,
                "{}: lectern at {:?} must be inside the fixture, with page < pages",
                self.name,
                l.pos
            );
        }
        for e in &self.experiments {
            ensure!(EXPERIMENTS.contains(&e.as_str()), "{}: unknown experiment {e:?}", self.name);
        }
        if let Some(n) = self.max_chained_updates {
            ensure!(n > 0, "{}: max_chained_updates must be positive", self.name);
        }
        for c in &self.comparators {
            ensure!(
                (0..3).all(|i| (0..size[i]).contains(&c.pos[i])) && (0..=15).contains(&c.output),
                "{}: comparator output {:?} = {} is outside the fixture or 0..15",
                self.name,
                c.pos,
                c.output
            );
        }
        self.transform_list()?;
        Ok(())
    }

    pub fn transform_list(&self) -> Result<Vec<Transform>> {
        match &self.transforms {
            None => Ok(Transform::all().to_vec()),
            Some(list) => list.iter().map(|s| s.parse()).collect(),
        }
    }

    /// The blocks in the authored orientation, as `(pos, state)`.
    fn blocks(&self) -> Result<Vec<([i32; 3], BlockState)>> {
        let mut out = Vec::new();
        for (y, layer) in self.layers.iter().enumerate() {
            for (z, row) in layer.iter().enumerate() {
                for (x, c) in row.chars().enumerate() {
                    if c == '.' {
                        continue;
                    }
                    let state = BlockState::parse(&self.palette[&c.to_string()])?;
                    if !state.is_air() {
                        out.push(([x as i32, y as i32, z as i32], state));
                    }
                }
            }
        }
        Ok(out)
    }

    pub fn resolve(&self, t: Transform) -> Result<ResolvedFixture> {
        let size = self.size();
        let mut blocks: Vec<(i32, i32, i32, String)> = self
            .blocks()?
            .into_iter()
            .map(|(p, s)| {
                let [x, y, z] = t.pos(p, size);
                (x, y, z, t.state(&s).to_string())
            })
            .collect();
        // Deterministic order: y, then z, then x.
        blocks.sort_by_key(|b| (b.1, b.2, b.0));
        let stimuli = self
            .stimuli
            .iter()
            .map(|s| Stimulus {
                tick: s.tick,
                action: s.action.clone(),
                pos: t.pos(s.pos, size),
                state: s
                    .state
                    .as_deref()
                    .map(|st| BlockState::parse(st).map(|b| t.state(&b).to_string()))
                    .transpose()
                    .unwrap(),
                place: s.place.as_ref().map(|p| p.transform(t)),
                page: s.page,
                time: s.time,
            })
            .collect();
        let containers = self
            .containers
            .iter()
            .map(|c| Container {
                pos: t.pos(c.pos, size),
                items: c
                    .items
                    .iter()
                    .map(|i| ItemSlot { slot: i.slot, id: full_item_id(&i.id), count: i.count })
                    .collect(),
            })
            .collect();
        let comparators =
            self.comparators.iter().map(|c| ComparatorOutput { pos: t.pos(c.pos, size), output: c.output }).collect();
        Ok(ResolvedFixture {
            format: RESOLVED_FORMAT,
            id: format!("{}@{t}", self.name),
            name: self.name.clone(),
            transform: t.to_string(),
            ticks: self.ticks,
            levels: self.levels.clone(),
            origin: self.origin.unwrap_or(DEFAULT_ORIGIN),
            size: t.size(size),
            margin: DEFAULT_MARGIN,
            blocks,
            stimuli,
            containers,
            comparators,
            lecterns: self
                .lecterns
                .iter()
                .map(|l| Lectern { pos: t.pos(l.pos, size), pages: l.pages, page: l.page })
                .collect(),
            experiments: self.experiments.clone(),
            day_time: self.day_time,
            max_chained_updates: self.max_chained_updates,
        })
    }

    pub fn resolve_all(&self) -> Result<Vec<ResolvedFixture>> {
        self.transform_list()?.into_iter().map(|t| self.resolve(t)).collect()
    }
}

fn full_item_id(id: &str) -> String {
    if id.contains(':') { id.to_string() } else { format!("minecraft:{id}") }
}

#[cfg(test)]
mod tests {
    use super::*;

    const INVERTER: &str = r###"{
        "name": "torch_inverter",
        "ticks": 10,
        "palette": {
            "#": "stone",
            "L": "lever[face=floor,facing=north,powered=false]",
            "T": "redstone_wall_torch[facing=east,lit=true]"
        },
        "layers": [
            ["##."],
            ["L#T"]
        ],
        "stimuli": [ { "tick": 2, "action": "use", "pos": [0, 1, 0] } ]
    }"###;

    #[test]
    fn parses_and_resolves_all_orientations() {
        let f = FixtureSource::from_json(INVERTER).unwrap();
        assert_eq!(f.size(), [3, 2, 1]);
        let all = f.resolve_all().unwrap();
        assert_eq!(all.len(), 8);
        let r0 = &all[0];
        assert_eq!(r0.id, "torch_inverter@r0");
        assert_eq!(r0.blocks.len(), 5);
        assert_eq!(r0.blocks[4], (2, 1, 0, "minecraft:redstone_wall_torch[facing=east,lit=true]".into()));
        let r90 = &all[1];
        assert_eq!(r90.size, [1, 2, 3]);
        assert!(r90.blocks.contains(&(0, 1, 2, "minecraft:redstone_wall_torch[facing=south,lit=true]".into())));
        assert_eq!(r90.stimuli[0].pos, [0, 1, 0]);
    }

    #[test]
    fn rejects_unknown_palette_chars_and_ragged_rows() {
        let bad = INVERTER.replace("\"L#T\"", "\"L#X\"");
        assert!(FixtureSource::from_json(&bad).is_err());
        let ragged = INVERTER.replace("\"##.\"", "\"##\"");
        assert!(FixtureSource::from_json(&ragged).is_err());
    }

    #[test]
    fn rejects_stimuli_outside_the_run() {
        let late = INVERTER.replace("\"tick\": 2", "\"tick\": 11");
        assert!(FixtureSource::from_json(&late).is_err());
    }
}
