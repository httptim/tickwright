//! Schematics in the formats players already use: Litematica's
//! `.litematic`, Sponge `.schem` (versions 2 and 3, as WorldEdit writes
//! them) and the game's own structure `.nbt`.
//!
//! Every format reads into one [`Schematic`]: the blocks of a box with its
//! least corner at the origin, and the block entity data that matters to
//! redstone (container contents and comparator outputs). [`Schematic::to_fixture`]
//! turns one into a fixture, so a downloaded build can be captured from the
//! game and run in the engine like any other.

pub mod litematic;
pub mod nbt;
pub mod sponge;
pub mod structure;

use std::collections::BTreeMap;

use anyhow::{Context, Result, bail};
use tickwright_trace::fixture::{ComparatorOutput, Container, ItemSlot};
use tickwright_trace::{BlockState, FixtureSource, Level, ResolvedFixture};

use nbt::Tag;

/// The data version 26.3 saves files with (`world_version` in the server
/// jar's `version.json`, recorded in `data/26.3/version.json`), written into
/// exported files.
pub const DATA_VERSION_26_3: i32 = 5023;

/// The largest box a schematic may span, in blocks. Editor builds are far
/// smaller; this stops a stress-test file from exhausting memory.
pub const MAX_VOLUME: u64 = 64 << 20;

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

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Schematic {
    /// Width (x), height (y) and length (z).
    pub size: [i32; 3],
    /// Non-air blocks, sorted by y, z, x.
    pub blocks: Vec<([i32; 3], BlockState)>,
    pub block_entities: Vec<BlockEntity>,
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

    /// Container contents from the block entities' `Items`.
    pub fn containers(&self) -> Vec<Container> {
        let mut out = Vec::new();
        for be in &self.block_entities {
            let Some(items) = be.data.get("Items") else { continue };
            let slots: Vec<ItemSlot> = items
                .list()
                .iter()
                .filter_map(|i| {
                    let id = i.get("id")?.as_str()?;
                    let count = i.get("count").or_else(|| i.get("Count"))?.as_i64()?;
                    let slot = i.get("Slot")?.as_i64()?;
                    (count > 0).then(|| ItemSlot {
                        slot: slot as u32,
                        id: id.strip_prefix("minecraft:").unwrap_or(id).to_string(),
                        count: count as u32,
                    })
                })
                .collect();
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

    /// A fixture that places this schematic and records `ticks` ticks.
    pub fn to_fixture(&self, name: &str, ticks: u32) -> Result<FixtureSource> {
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
        };
        Ok(fixture)
    }

    /// A resolved fixture's blocks, container contents and comparator
    /// outputs, for export.
    pub fn from_fixture(f: &ResolvedFixture) -> Result<Schematic> {
        let mut blocks = Vec::with_capacity(f.blocks.len());
        for (x, y, z, s) in &f.blocks {
            blocks.push(([*x, *y, *z], BlockState::parse(s).with_context(|| format!("{}: {s}", f.id))?));
        }
        let mut s = Schematic::from_blocks(f.size, blocks);
        for c in &f.containers {
            let items = c
                .items
                .iter()
                .map(|i| {
                    let id = if i.id.contains(':') { i.id.clone() } else { format!("minecraft:{}", i.id) };
                    nbt::compound(vec![
                        ("Slot", Tag::Byte(i.slot as i8)),
                        ("id", Tag::String(id)),
                        ("count", Tag::Int(i.count as i32)),
                    ])
                })
                .collect();
            let id = s.block(c.pos).id;
            s.block_entities.push(BlockEntity {
                pos: c.pos,
                data: nbt::compound(vec![("id", Tag::String(id)), ("Items", Tag::List(10, items))]),
            });
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
