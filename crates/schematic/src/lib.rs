//! Schematics in the formats players already use: Litematica's
//! `.litematic`, Sponge `.schem` (versions 2 and 3, as WorldEdit writes
//! them) and the game's own structure `.nbt`.
//!
//! Every format reads into one [`Schematic`]: the blocks of a box with its
//! least corner at the origin, the block entity data that matters to
//! redstone (container contents and comparator outputs), and the entities
//! in the box. [`Schematic::to_fixture`]
//! turns one into a fixture, so a downloaded build can be captured from the
//! game and run in the engine like any other.

pub mod litematic;
pub mod nbt;
pub mod snbt;
pub mod sponge;
pub mod structure;

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use tickwright_trace::fixture::{ComparatorOutput, Container, ItemSlot, SummonInput};
use tickwright_trace::{BlockState, FixtureSource, Level, ResolvedFixture};

use nbt::Tag;

/// The data version 26.3 saves files with (`world_version` in the server
/// jar's `version.json`, recorded in `data/26.3/version.json`), written into
/// exported files.
pub const DATA_VERSION_26_3: i32 = 5023;

/// The largest box a schematic may span, in blocks. Editor builds are far
/// smaller; this stops a stress-test file from exhausting memory.
pub const MAX_VOLUME: u64 = 64 << 20;

const JUKEBOX: &str = "minecraft:jukebox";
/// Where a jukebox keeps its disc, and how far its song has played.
const RECORD_ITEM: &str = "RecordItem";
const SONG_TICKS: &str = "ticks_since_song_started";

/// Refuse a box larger than [`MAX_VOLUME`].
fn check_volume(size: [i32; 3]) -> Result<()> {
    let v = size.iter().map(|&n| n.unsigned_abs() as u64).product::<u64>();
    if v > MAX_VOLUME {
        bail!("{}x{}x{} is too large: the limit is {MAX_VOLUME} blocks", size[0], size[1], size[2]);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Litematic,
    Sponge,
    Structure,
}

impl Format {
    /// The format a file name's extension names.
    pub fn from_name(name: &str) -> Result<Format> {
        let lower = name.to_ascii_lowercase();
        Ok(if lower.ends_with(".litematic") {
            Format::Litematic
        } else if lower.ends_with(".schem") {
            Format::Sponge
        } else if lower.ends_with(".nbt") {
            Format::Structure
        } else {
            bail!("{name}: not a .litematic, .schem or .nbt file")
        })
    }
}

/// A block entity's data, at a position inside the schematic.
#[derive(Clone, Debug, PartialEq)]
pub struct BlockEntity {
    pub pos: [i32; 3],
    /// The block entity's own compound, without its position: `Items`,
    /// `OutputSignal` and so on, plus `id` when the file gave one.
    pub data: Tag,
}

/// An entity in the schematic.
#[derive(Clone, Debug, PartialEq)]
pub struct Entity {
    /// The entity type, such as `minecraft:minecart`.
    pub id: String,
    /// Where it is, in blocks from the schematic's least corner.
    pub pos: [f64; 3],
    /// The rest of its saved data, without `id` and `Pos`.
    pub data: Tag,
}

impl Entity {
    /// An entity from a saved compound holding `id` and `Pos`, with `pos`
    /// moved by `offset` (the region's corner in schematic space).
    fn from_saved(tag: &Tag, offset: [f64; 3]) -> Result<Entity> {
        let id = tag.req("id")?.as_str().context("an entity's `id` is not a string")?.to_string();
        let pos = doubles(tag.req("Pos")?).context("an entity's `Pos` is not three doubles")?;
        Ok(Entity {
            id,
            pos: [pos[0] + offset[0], pos[1] + offset[1], pos[2] + offset[2]],
            data: strip_keys(tag, &["id", "Pos"]),
        })
    }

    /// The compound a file saves: `id`, `Pos`, then the rest.
    fn saved(&self) -> Tag {
        let mut e = vec![
            ("id".to_string(), Tag::String(self.id.clone())),
            ("Pos".to_string(), Tag::List(6, self.pos.iter().map(|&v| Tag::Double(v)).collect())),
        ];
        e.extend(self.data.entries().iter().cloned());
        Tag::Compound(e)
    }

    /// The entity as a fixture's start entity: `Motion` becomes `motion`
    /// and everything else stays in `nbt`.
    pub fn to_summon(&self) -> SummonInput {
        let motion = self.data.get("Motion").and_then(doubles).filter(|m| m.iter().any(|&v| v != 0.0));
        let rest = strip_keys(&self.data, &["Motion"]);
        SummonInput {
            entity: self.id.strip_prefix("minecraft:").unwrap_or(&self.id).to_string(),
            at: self.pos,
            motion,
            nbt: (!rest.entries().is_empty()).then(|| snbt::write(&rest)),
        }
    }

    /// A fixture's start entity, back as saved data.
    pub fn from_summon(e: &SummonInput) -> Result<Entity> {
        let mut data = match &e.nbt {
            Some(text) => strip_keys(&snbt::parse(text)?, &["id", "Pos"]),
            None => Tag::Compound(Vec::new()),
        };
        if let (Some(m), Tag::Compound(entries)) = (e.motion, &mut data) {
            entries.retain(|(k, _)| k != "Motion");
            entries.insert(0, ("Motion".to_string(), Tag::List(6, m.iter().map(|&v| Tag::Double(v)).collect())));
        }
        let id = if e.entity.contains(':') { e.entity.clone() } else { format!("minecraft:{}", e.entity) };
        Ok(Entity { id, pos: e.at, data })
    }
}

/// Three numbers from a list of doubles (or floats).
fn doubles(tag: &Tag) -> Option<[f64; 3]> {
    let v: Vec<f64> = tag
        .list()
        .iter()
        .map(|t| match t {
            Tag::Double(d) => Some(*d),
            Tag::Float(f) => Some(*f as f64),
            _ => None,
        })
        .collect::<Option<_>>()?;
    v.try_into().ok()
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Schematic {
    /// Width (x), height (y) and length (z).
    pub size: [i32; 3],
    /// Non-air blocks, sorted by y, z, x.
    pub blocks: Vec<([i32; 3], BlockState)>,
    pub block_entities: Vec<BlockEntity>,
    pub entities: Vec<Entity>,
    /// The data version the file was saved in, if it said.
    pub data_version: Option<i32>,
}

impl Schematic {
    /// Read a file in whichever format its contents are.
    pub fn read(bytes: &[u8]) -> Result<Schematic> {
        let (_, root) = nbt::read(bytes)?;
        if root.get("Regions").is_some() {
            litematic::from_nbt(&root)
        } else if root.get("Schematic").is_some() || root.get("BlockData").is_some() {
            sponge::from_nbt(&root)
        } else if root.get("palette").is_some() || root.get("palettes").is_some() {
            structure::from_nbt(&root)
        } else {
            bail!("not a schematic this reader knows: no Regions, Schematic or palette")
        }
    }

    pub fn write(&self, format: Format) -> Vec<u8> {
        match format {
            Format::Litematic => litematic::write(self),
            Format::Sponge => sponge::write(self),
            Format::Structure => structure::write(self),
        }
    }

    /// Sort blocks into y, z, x order, drop air, and size the box to fit.
    fn finish(mut self) -> Schematic {
        self.blocks.retain(|(_, s)| !s.is_air());
        self.blocks.sort_by_key(|(p, _)| (p[1], p[2], p[0]));
        self.block_entities.sort_by_key(|b| (b.pos[1], b.pos[2], b.pos[0]));
        self
    }

    /// The block at `pos`, air if none.
    pub fn block(&self, pos: [i32; 3]) -> BlockState {
        self.blocks
            .binary_search_by_key(&(pos[1], pos[2], pos[0]), |(p, _)| (p[1], p[2], p[0]))
            .map_or_else(|_| BlockState::air(), |i| self.blocks[i].1.clone())
    }

    /// Container contents from the block entities' `Items`, and a jukebox's
    /// disc from its `RecordItem`, as slot 0.
    pub fn containers(&self) -> Vec<Container> {
        let mut out = Vec::new();
        for be in &self.block_entities {
            let stack = |i: &Tag, slot: Option<i64>| {
                let id = i.get("id")?.as_str()?;
                let count = i.get("count").or_else(|| i.get("Count"))?.as_i64()?;
                let slot = slot.or_else(|| i.get("Slot")?.as_i64())?;
                (count > 0).then(|| ItemSlot {
                    slot: slot as u32,
                    id: id.strip_prefix("minecraft:").unwrap_or(id).to_string(),
                    count: count as u32,
                })
            };
            let slots: Vec<ItemSlot> = match (be.data.get("Items"), be.data.get(RECORD_ITEM)) {
                (Some(items), _) => items.list().iter().filter_map(|i| stack(i, None)).collect(),
                (None, Some(record)) => stack(record, Some(0)).into_iter().collect(),
                (None, None) => continue,
            };
            if !slots.is_empty() {
                out.push(Container { pos: be.pos, items: slots });
            }
        }
        out
    }

    /// Comparator outputs other than 0.
    pub fn comparators(&self) -> Vec<ComparatorOutput> {
        self.block_entities
            .iter()
            .filter(|be| self.block(be.pos).id == "minecraft:comparator")
            .filter_map(|be| {
                let output = be.data.get("OutputSignal")?.as_i64()? as i32;
                (output != 0).then_some(ComparatorOutput { pos: be.pos, output })
            })
            .collect()
    }

    /// A jukebox whose disc a fixture can't carry as the game would load it.
    /// A fixture puts a disc in with `setItem`, which starts its song from
    /// the beginning; the game loads a saved jukebox part way through its
    /// song (`ticks_since_song_started`), or not playing at all when that
    /// is missing, as after its song ended.
    fn check_jukeboxes(&self, name: &str) -> Result<()> {
        for be in &self.block_entities {
            if be.data.get(RECORD_ITEM).is_none() {
                continue;
            }
            match be.data.get(SONG_TICKS).and_then(Tag::as_i64) {
                Some(0) => {}
                Some(t) => bail!(
                    "{name}: a jukebox at {:?} part way through its song ({SONG_TICKS} {t}) is not supported yet",
                    be.pos
                ),
                None => {
                    bail!("{name}: a jukebox at {:?} holding a disc it is not playing is not supported yet", be.pos)
                }
            }
        }
        Ok(())
    }

    /// A fixture that places this schematic and records `ticks` ticks.
    /// A jukebox the fixture can't start as the game would is refused.
    pub fn to_fixture(&self, name: &str, ticks: u32) -> Result<FixtureSource> {
        self.check_jukeboxes(name)?;
        const KEYS: &str = "#ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789@$%&*+=?!<>^~";
        let mut keys: BTreeMap<String, char> = BTreeMap::new();
        let mut extra = (0x3B1u32..0x3CA).chain(0x391..0x3AA).chain(0x400..0x4FF).filter_map(char::from_u32);
        let mut ascii = KEYS.chars();
        let [sx, sy, sz] = self.size;
        let mut layers = vec![vec![vec!['.'; sx as usize]; sz as usize]; sy as usize];
        for (p, state) in &self.blocks {
            let text = state.to_string();
            let next = keys.len();
            let c = match keys.get(&text) {
                Some(c) => *c,
                None => {
                    let c = ascii.next().or_else(|| extra.next()).with_context(|| {
                        format!("{name}: more than {next} block states; fixtures cannot key them all")
                    })?;
                    keys.insert(text.clone(), c);
                    c
                }
            };
            layers[p[1] as usize][p[2] as usize][p[0] as usize] = c;
        }
        let palette = keys
            .into_iter()
            .map(|(state, c)| (c.to_string(), state.strip_prefix("minecraft:").unwrap_or(&state).to_string()))
            .collect();
        let layers = layers.into_iter().map(|l| l.into_iter().map(|row| row.into_iter().collect()).collect()).collect();
        let fixture = FixtureSource {
            name: name.to_string(),
            description: String::new(),
            ticks,
            levels: vec![Level::State, Level::Phase, Level::Order],
            transforms: None,
            seeds: 0,
            origin: None,
            tags: Vec::new(),
            palette,
            layers,
            stimuli: Vec::new(),
            containers: self.containers(),
            comparators: self.comparators(),
            lecterns: Vec::new(),
            experiments: Vec::new(),
            day_time: None,
            max_chained_updates: None,
            entities: false,
            start_entities: self.entities.iter().map(Entity::to_summon).collect(),
        };
        Ok(fixture)
    }

    /// A resolved fixture's blocks, container contents, comparator outputs
    /// and start entities, for export.
    pub fn from_fixture(f: &ResolvedFixture) -> Result<Schematic> {
        let mut blocks = Vec::with_capacity(f.blocks.len());
        for (x, y, z, s) in &f.blocks {
            blocks.push(([*x, *y, *z], BlockState::parse(s).with_context(|| format!("{}: {s}", f.id))?));
        }
        // A jukebox given a disc plays it, and its `has_record` is set, as the
        // harness's `setItem` leaves it.
        for c in &f.containers {
            if let Some((_, state)) = blocks.iter_mut().find(|(p, st)| *p == c.pos && st.id == JUKEBOX) {
                state.props.insert("has_record".into(), "true".into());
            }
        }
        let mut s = Schematic::from_blocks(f.size, blocks);
        for c in &f.containers {
            let stack = |i: &ItemSlot, slot: bool| {
                let id = if i.id.contains(':') { i.id.clone() } else { format!("minecraft:{}", i.id) };
                let mut tags = vec![("id", Tag::String(id)), ("count", Tag::Int(i.count as i32))];
                if slot {
                    tags.insert(0, ("Slot", Tag::Byte(i.slot as i8)));
                }
                nbt::compound(tags)
            };
            let id = s.block(c.pos).id;
            let data = if id == JUKEBOX {
                // `JukeboxBlockEntity.saveAdditional`: the disc, and how far
                // its song has played.
                let disc = c.items.iter().find(|i| i.slot == 0).context("a jukebox holds its disc in slot 0")?;
                nbt::compound(vec![
                    ("id", Tag::String(id)),
                    (RECORD_ITEM, stack(disc, false)),
                    (SONG_TICKS, Tag::Long(0)),
                ])
            } else {
                let items = c.items.iter().map(|i| stack(i, true)).collect();
                nbt::compound(vec![("id", Tag::String(id)), ("Items", Tag::List(10, items))])
            };
            s.block_entities.push(BlockEntity { pos: c.pos, data });
        }
        for c in &f.comparators {
            s.block_entities.push(BlockEntity {
                pos: c.pos,
                data: nbt::compound(vec![
                    ("id", Tag::String("minecraft:comparator".into())),
                    ("OutputSignal", Tag::Int(c.output)),
                ]),
            });
        }
        for e in &f.start_entities {
            s.entities.push(Entity::from_summon(e).with_context(|| format!("{}: entity {}", f.id, e.entity))?);
        }
        s.data_version = Some(DATA_VERSION_26_3);
        Ok(s.finish())
    }

    /// A schematic of a resolved or authored fixture's blocks, for export.
    pub fn from_blocks(size: [i32; 3], blocks: impl IntoIterator<Item = ([i32; 3], BlockState)>) -> Schematic {
        Schematic { size, blocks: blocks.into_iter().collect(), ..Default::default() }.finish()
    }
}

/// `Name` plus `Properties`, as palettes store states.
fn state_from_nbt(tag: &Tag) -> Result<BlockState> {
    let name = tag.req("Name")?.as_str().context("`Name` is not a string")?;
    let id = if name.contains(':') { name.to_string() } else { format!("minecraft:{name}") };
    let mut props = BTreeMap::new();
    if let Some(p) = tag.get("Properties") {
        for (k, v) in p.entries() {
            props.insert(k.clone(), v.as_str().context("a property is not a string")?.to_string());
        }
    }
    Ok(BlockState { id, props })
}

fn state_to_nbt(state: &BlockState) -> Tag {
    let mut entries = vec![("Name".to_string(), Tag::String(state.id.clone()))];
    if !state.props.is_empty() {
        let props = state.props.iter().map(|(k, v)| (k.clone(), Tag::String(v.clone()))).collect();
        entries.push(("Properties".to_string(), Tag::Compound(props)));
    }
    Tag::Compound(entries)
}

/// A block entity compound without the position keys a format adds.
fn strip_keys(tag: &Tag, keys: &[&str]) -> Tag {
    Tag::Compound(tag.entries().iter().filter(|(k, _)| !keys.contains(&k.as_str())).cloned().collect())
}

/// The palette of a schematic's states, air first, and each block's index.
fn palette(s: &Schematic) -> (Vec<BlockState>, Vec<u32>) {
    let [sx, sy, sz] = s.size;
    let mut states = vec![BlockState::air()];
    let mut index: BTreeMap<BlockState, u32> = BTreeMap::from([(BlockState::air(), 0)]);
    let mut cells = vec![0u32; (sx * sy * sz) as usize];
    for (p, st) in &s.blocks {
        let i = *index.entry(st.clone()).or_insert_with(|| {
            states.push(st.clone());
            states.len() as u32 - 1
        });
        cells[((p[1] * sz + p[2]) * sx + p[0]) as usize] = i;
    }
    (states, cells)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn sample() -> Schematic {
        let b = |s: &str| BlockState::parse(s).unwrap();
        let items = Tag::List(
            10,
            vec![nbt::compound(vec![
                ("Slot", Tag::Byte(0)),
                ("id", Tag::String("minecraft:redstone".into())),
                ("count", Tag::Int(64)),
            ])],
        );
        Schematic {
            size: [3, 2, 2],
            blocks: vec![
                ([0, 0, 0], b("stone")),
                ([1, 0, 0], b("barrel[facing=up,open=false]")),
                ([2, 0, 0], b("comparator[facing=west,mode=compare,powered=true]")),
                ([0, 1, 1], b("repeater[delay=2,facing=north,locked=false,powered=false]")),
            ],
            block_entities: vec![
                BlockEntity {
                    pos: [1, 0, 0],
                    data: nbt::compound(vec![("id", Tag::String("minecraft:barrel".into())), ("Items", items)]),
                },
                BlockEntity {
                    pos: [2, 0, 0],
                    data: nbt::compound(vec![
                        ("id", Tag::String("minecraft:comparator".into())),
                        ("OutputSignal", Tag::Int(1)),
                    ]),
                },
            ],
            entities: vec![
                Entity {
                    id: "minecraft:minecart".into(),
                    pos: [1.5, 1.0625, 0.5],
                    data: nbt::compound(vec![
                        ("Motion", Tag::List(6, vec![Tag::Double(0.1), Tag::Double(0.0), Tag::Double(0.0)])),
                        ("Rotation", Tag::List(5, vec![Tag::Float(90.0), Tag::Float(0.0)])),
                    ]),
                },
                Entity {
                    id: "minecraft:item".into(),
                    pos: [0.5, 1.0, 1.5],
                    data: nbt::compound(vec![(
                        "Item",
                        nbt::compound(vec![("id", Tag::String("minecraft:stone".into())), ("count", Tag::Int(1))]),
                    )]),
                },
            ],
            data_version: Some(DATA_VERSION_26_3),
        }
        .finish()
    }

    #[test]
    fn data_version_is_the_games() {
        let text = include_str!("../../../data/26.3/version.json");
        let line = text.lines().find(|l| l.contains("\"data_version\"")).unwrap();
        let n: i32 = line.split(':').nth(1).unwrap().trim().trim_end_matches(',').parse().unwrap();
        assert_eq!(n, DATA_VERSION_26_3);
    }

    #[test]
    fn every_format_round_trips() {
        let s = sample();
        for f in [Format::Litematic, Format::Sponge, Format::Structure] {
            let back = Schematic::read(&s.write(f)).unwrap();
            assert_eq!(back.size, s.size, "{f:?}");
            assert_eq!(back.blocks, s.blocks, "{f:?}");
            assert_eq!(back.containers(), s.containers(), "{f:?}");
            assert_eq!(back.comparators(), s.comparators(), "{f:?}");
            assert_eq!(back.entities, s.entities, "{f:?}");
        }
    }

    #[test]
    fn entities_round_trip_through_fixtures() {
        let s = sample();
        let f = s.to_fixture("t", 10).unwrap();
        assert_eq!(f.start_entities[0].entity, "minecart");
        assert_eq!(f.start_entities[0].motion, Some([0.1, 0.0, 0.0]));
        let text = serde_json::to_string(&f).unwrap();
        let resolved = FixtureSource::from_json(&text).unwrap().resolve("r0".parse().unwrap()).unwrap();
        let back = Schematic::from_fixture(&resolved).unwrap();
        assert_eq!(back.entities, s.entities);
    }

    fn b(s: &str) -> BlockState {
        BlockState::parse(s).unwrap()
    }

    /// A jukebox as 26.3 saves it (`JukeboxBlockEntity.saveAdditional`):
    /// its disc as `RecordItem`, and while it plays, `ticks_since_song_started`.
    fn jukebox(ticks: Option<i64>) -> Schematic {
        let mut data = vec![
            ("id", Tag::String("minecraft:jukebox".into())),
            (
                "RecordItem",
                nbt::compound(vec![("id", Tag::String("minecraft:music_disc_pigstep".into())), ("count", Tag::Int(1))]),
            ),
        ];
        if let Some(t) = ticks {
            data.push(("ticks_since_song_started", Tag::Long(t)));
        }
        Schematic {
            size: [1, 1, 1],
            blocks: vec![([0, 0, 0], b("jukebox[has_record=true]"))],
            block_entities: vec![BlockEntity { pos: [0, 0, 0], data: nbt::compound(data) }],
            data_version: Some(DATA_VERSION_26_3),
            ..Default::default()
        }
    }

    #[test]
    fn a_jukebox_disc_comes_in_from_record_item() {
        let f = jukebox(Some(0)).to_fixture("t", 10).unwrap();
        assert_eq!(
            f.containers,
            vec![Container {
                pos: [0, 0, 0],
                items: vec![ItemSlot { slot: 0, id: "music_disc_pigstep".into(), count: 1 }]
            }]
        );
        // A song part way through, or one that has ended, can't be started
        // as the game would load it.
        let err = jukebox(Some(1234)).to_fixture("t", 10).unwrap_err().to_string();
        assert_eq!(
            err,
            "t: a jukebox at [0, 0, 0] part way through its song (ticks_since_song_started 1234) is not supported yet"
        );
        let err = jukebox(None).to_fixture("t", 10).unwrap_err().to_string();
        assert_eq!(err, "t: a jukebox at [0, 0, 0] holding a disc it is not playing is not supported yet");
    }

    #[test]
    fn a_jukebox_disc_goes_out_as_record_item() {
        let text = serde_json::to_string(&jukebox(Some(0)).to_fixture("t", 10).unwrap()).unwrap();
        let mut source = FixtureSource::from_json(&text).unwrap();
        // A fixture can give a jukebox its disc with has_record unset.
        source.palette.values_mut().for_each(|s| *s = s.replace("has_record=true", "has_record=false"));
        let resolved = source.resolve("r0".parse().unwrap()).unwrap();
        let s = Schematic::from_fixture(&resolved).unwrap();
        assert_eq!(s.blocks[0].1, b("jukebox[has_record=true]"));
        let data = &s.block_entities[0].data;
        assert!(data.get("Items").is_none());
        assert_eq!(data.get("RecordItem").and_then(|r| r.get("id")?.as_str()), Some("minecraft:music_disc_pigstep"));
        assert_eq!(data.get("ticks_since_song_started").and_then(Tag::as_i64), Some(0));
        for f in [Format::Litematic, Format::Sponge, Format::Structure] {
            let back = Schematic::read(&s.write(f)).unwrap();
            assert_eq!(back.containers(), jukebox(Some(0)).containers(), "{f:?}");
            assert_eq!(back.blocks, s.blocks, "{f:?}");
        }
    }

    #[test]
    fn fixtures_carry_contents_and_comparators() {
        let f = sample().to_fixture("t", 10).unwrap();
        assert_eq!(f.layers.len(), 2);
        assert_eq!(f.containers[0].items[0].id, "redstone");
        assert_eq!(f.comparators[0].output, 1);
        FixtureSource::from_json(&serde_json::to_string(&f).unwrap()).unwrap();
    }
}
