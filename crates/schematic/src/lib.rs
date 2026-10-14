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

/// The first data version whose item stacks keep `count` and `components`
/// where older files kept `Count` and `tag`: the game's
/// `ItemStackComponentizationFix` runs at schema 3818.5.
pub const ITEM_COMPONENTS_DATA_VERSION: i32 = 3819;

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
    /// and everything else stays in `nbt`. Item stacks saved before 1.20.5
    /// (`data_version`) are brought up to date the way the game's data fixer
    /// starts to: `Count` becomes `count`, and their `tag` stays, for the
    /// engine to refuse by name. A newer stack's stray `tag`, which the game
    /// ignores, is left out. The rest of its data is in the file's format,
    /// so an entity from a file older than the game records the file's data
    /// version (or, from a file that doesn't say, the version before item
    /// components when its stacks are that old).
    pub fn to_summon(&self, data_version: Option<i32>) -> SummonInput {
        let motion = self.data.get("Motion").and_then(doubles).filter(|m| m.iter().any(|&v| v != 0.0));
        let mut rest = strip_keys(&self.data, &["Motion"]);
        let mut versioned = data_version.filter(|&v| v < DATA_VERSION_26_3);
        for stack in item_stacks(&mut rest) {
            if let Some(dv) = fix_stack(stack, data_version)
                && data_version.is_none()
            {
                versioned = Some(versioned.map_or(dv, |v| v.min(dv)));
            }
        }
        SummonInput {
            entity: self.id.strip_prefix("minecraft:").unwrap_or(&self.id).to_string(),
            at: self.pos,
            motion,
            nbt: (!rest.entries().is_empty()).then(|| snbt::write(&rest)),
            data_version: versioned,
        }
    }

    /// A fixture's start entity, back as saved data. `legacy` writes its
    /// item stacks as files from before 1.20.5 keep them (`Count`), for a
    /// file of that data version.
    pub fn from_summon(e: &SummonInput, legacy: bool) -> Result<Entity> {
        let mut data = match &e.nbt {
            Some(text) => strip_keys(&snbt::parse(text)?, &["id", "Pos"]),
            None => Tag::Compound(Vec::new()),
        };
        if legacy {
            for stack in item_stacks(&mut data) {
                unfix_stack(stack)?;
            }
        }
        if let (Some(m), Tag::Compound(entries)) = (e.motion, &mut data) {
            entries.retain(|(k, _)| k != "Motion");
            entries.insert(0, ("Motion".to_string(), Tag::List(6, m.iter().map(|&v| Tag::Double(v)).collect())));
        }
        let id = if e.entity.contains(':') { e.entity.clone() } else { format!("minecraft:{}", e.entity) };
        Ok(Entity { id, pos: e.at, data })
    }
}

/// The item stacks in an entity's saved data: an item entity's `Item` and
/// a container minecart's `Items`.
fn item_stacks(data: &mut Tag) -> Vec<&mut Tag> {
    let Tag::Compound(entries) = data else { return Vec::new() };
    let mut out = Vec::new();
    for (k, v) in entries.iter_mut() {
        match (k.as_str(), v) {
            ("Item", item) => out.push(item),
            ("Items", Tag::List(_, items)) => out.extend(items.iter_mut()),
            _ => {}
        }
    }
    out
}

/// Whether an item stack was saved before 1.20.5. A file that gives no
/// data version is judged by the stack: `Count` without `count`.
fn legacy_stack(stack: &Tag, data_version: Option<i32>) -> bool {
    match data_version {
        Some(v) => v < ITEM_COMPONENTS_DATA_VERSION,
        None => stack.get("count").is_none() && stack.get("Count").is_some(),
    }
}

/// The data version to record for a stack whose components or tag are in
/// an older format: the file's, or the last version before components for
/// a legacy stack in a file that doesn't say.
fn stack_version(stack: &Tag, data_version: Option<i32>) -> Option<i32> {
    match data_version {
        Some(v) if v < DATA_VERSION_26_3 => Some(v),
        Some(_) => None,
        None => legacy_stack(stack, None).then_some(ITEM_COMPONENTS_DATA_VERSION - 1),
    }
}

/// `ItemStackComponentizationFix`, as far as the engine needs it: a legacy
/// stack's `Count` (a byte) becomes `count` (an int), and its `tag` stays.
/// A newer stack's `tag` is a key the game doesn't read, and goes. Returns
/// the data version the stack's components or tag are in, if older.
fn fix_stack(stack: &mut Tag, data_version: Option<i32>) -> Option<i32> {
    let version = stack_version(stack, data_version);
    let legacy = legacy_stack(stack, data_version);
    if let Tag::Compound(entries) = stack {
        if legacy {
            for (k, v) in entries.iter_mut() {
                if k == "Count" {
                    *k = "count".to_string();
                    if let Some(n) = v.as_i64() {
                        *v = Tag::Int(n as i32);
                    }
                }
            }
            entries.retain(|(k, _)| k != "components");
        } else {
            entries.retain(|(k, _)| k != "tag");
        }
    }
    let formatted = stack.get("components").is_some_and(|c| !c.entries().is_empty()) || stack.get("tag").is_some();
    version.filter(|_| formatted)
}

/// The way back for a file from before 1.20.5: `count` becomes a `Count`
/// byte. A stack with components can't be written so, nor can a count
/// outside 1 to 99, which the game doesn't read.
fn unfix_stack(stack: &mut Tag) -> Result<()> {
    if stack.get("components").is_some_and(|c| !c.entries().is_empty()) {
        let id = stack.get("id").and_then(Tag::as_str).unwrap_or("an item");
        bail!("{id} has item components, which a file from before 1.20.5 can't hold");
    }
    if let Tag::Compound(entries) = stack {
        entries.retain(|(k, _)| k != "components");
        let mut has_count = false;
        for (k, v) in entries.iter_mut() {
            if k == "count" {
                *k = "Count".to_string();
                *v = Tag::Byte(file_count(v.as_i64().unwrap_or(1))? as i8);
                has_count = true;
            }
        }
        if !has_count {
            entries.push(("Count".to_string(), Tag::Byte(1)));
        }
    }
    Ok(())
}

/// A stack's count as a file may hold it: the game's `ItemStack.CODEC`
/// reads 1 to 99 and nothing else.
fn file_count(count: i64) -> Result<i32> {
    if !(1..=99).contains(&count) {
        bail!("a count of {count}, which the game can't read: a stack holds 1 to 99");
    }
    Ok(count as i32)
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

    /// Container contents from the block entities' `Items`, with each
    /// stack's component patch. A stack saved before 1.20.5 reads its
    /// `Count`, and keeps its `tag` for the engine to refuse by name; a
    /// newer one reads `count` (1 if it has none, as the game reads it) and
    /// ignores keys the game doesn't know. A stack the game would read as
    /// empty (a count outside 1 to 99, which `ItemStack.CODEC` rejects, or a
    /// legacy stack with no `Count`) is left out; one without an id or slot
    /// is an error, never quietly dropped.
    pub fn containers(&self) -> Result<Vec<Container>> {
        let mut out = Vec::new();
        for be in &self.block_entities {
            let Some(items) = be.data.get("Items") else { continue };
            let mut slots = Vec::new();
            for i in items.list() {
                let at = be.pos;
                let id = i.get("id").and_then(Tag::as_str).with_context(|| format!("an item at {at:?} has no id"))?;
                let slot =
                    i.get("Slot").and_then(Tag::as_i64).with_context(|| format!("{id} at {at:?} has no slot"))?;
                let legacy = legacy_stack(i, self.data_version);
                let count = match i.get(if legacy { "Count" } else { "count" }) {
                    // `ItemStack.of` read a missing `Count` as 0, an empty stack.
                    None if legacy => 0,
                    None => 1,
                    Some(c) => c.as_i64().with_context(|| format!("{id} at {at:?}: its count is not a number"))?,
                };
                if !(1..=99).contains(&count) {
                    continue;
                }
                let components = match i.get("components") {
                    Some(c) if !legacy && !c.entries().is_empty() => Some(snbt::write(c)),
                    _ => None,
                };
                let tag = match i.get("tag") {
                    Some(t) if legacy => Some(snbt::write(t)),
                    _ => None,
                };
                let data_version =
                    (components.is_some() || tag.is_some()).then(|| stack_version(i, self.data_version)).flatten();
                slots.push(ItemSlot {
                    slot: slot as u32,
                    id: id.strip_prefix("minecraft:").unwrap_or(id).to_string(),
                    count: count as u32,
                    components,
                    tag,
                    data_version,
                });
            }
            if !slots.is_empty() {
                out.push(Container { pos: be.pos, items: slots });
            }
        }
        Ok(out)
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
            seeds: 0,
            origin: None,
            tags: Vec::new(),
            palette,
            layers,
            stimuli: Vec::new(),
            containers: self.containers()?,
            comparators: self.comparators(),
            lecterns: Vec::new(),
            experiments: Vec::new(),
            day_time: None,
            max_chained_updates: None,
            entities: false,
            start_entities: self.entities.iter().map(|e| e.to_summon(self.data_version)).collect(),
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
        let mut s = Schematic::from_blocks(f.size, blocks);
        // Components, tags and entity data are written in the format they
        // came in, and a file holds one data version, which the game's data
        // fixer brings up to date. So they must all share one; the engine
        // can't convert between versions. Before 1.20.5 every stack is
        // written the old way (`Count`), and a stack with components can't
        // be.
        let mut common: Option<(Option<i32>, String)> = None;
        let shown = |v: Option<i32>| {
            v.map_or(format!("26.3's data version {DATA_VERSION_26_3}"), |v| format!("data version {v}"))
        };
        let carried = f
            .containers
            .iter()
            .flat_map(|c| {
                c.items.iter().filter(|i| i.components.is_some() || i.tag.is_some()).map(move |i| {
                    (i.data_version, format!("the {} in slot {} of the container at {:?}", i.id, i.slot, c.pos))
                })
            })
            .chain(f.start_entities.iter().map(|e| (e.data_version, format!("the {} entity at {:?}", e.entity, e.at))));
        for (version, place) in carried {
            match &common {
                None => common = Some((version, place)),
                Some((v, first)) if *v != version => bail!(
                    "{}: {place} is in {} but {first} is in {}, and a file holds one data version; save the build in one version first",
                    f.id,
                    shown(version),
                    shown(*v)
                ),
                Some(_) => {}
            }
        }
        let oldest = common.and_then(|(v, _)| v);
        let legacy = oldest.is_some_and(|v| v < ITEM_COMPONENTS_DATA_VERSION);
        for c in &f.containers {
            let block = s.block(c.pos).id;
            let items = c
                .items
                .iter()
                .map(|i| {
                    let id = if i.id.contains(':') { i.id.clone() } else { format!("minecraft:{}", i.id) };
                    let place = || format!("{}: {id} in slot {} of the {block} at {:?}", f.id, i.slot, c.pos);
                    let slot = i8::try_from(i.slot).with_context(|| format!("{}: a slot past 127", place()))?;
                    let count = file_count(i.count as i64).with_context(place)?;
                    let mut entries = vec![("Slot", Tag::Byte(slot)), ("id", Tag::String(id.clone()))];
                    if legacy {
                        if i.components.is_some() {
                            bail!(
                                "{} has item components, which a file for data version {} (before 1.20.5) can't hold",
                                place(),
                                oldest.unwrap_or_default()
                            );
                        }
                        entries.push(("Count", Tag::Byte(count as i8)));
                    } else {
                        entries.push(("count", Tag::Int(count)));
                    }
                    if let Some(c) = &i.components {
                        entries.push((
                            "components",
                            snbt::parse(c).with_context(|| format!("{}: components {c}", place()))?,
                        ));
                    }
                    if let Some(t) = &i.tag {
                        entries.push(("tag", snbt::parse(t).with_context(|| format!("{}: tag {t}", place()))?));
                    }
                    Ok(nbt::compound(entries))
                })
                .collect::<Result<Vec<_>>>()?;
            let id = block;
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
        for e in &f.start_entities {
            s.entities.push(Entity::from_summon(e, legacy).with_context(|| format!("{}: entity {}", f.id, e.entity))?);
        }
        s.data_version = Some(oldest.unwrap_or(DATA_VERSION_26_3));
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
    use tickwright_trace::Transform;

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
                        // In key order, as SNBT gives a compound back.
                        nbt::compound(vec![("count", Tag::Int(1)), ("id", Tag::String("minecraft:stone".into()))]),
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
            assert_eq!(back.containers().unwrap(), s.containers().unwrap(), "{f:?}");
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

    /// A chest at the origin holding `items`, saved with `data_version`.
    fn chest_with(items: Vec<Tag>, data_version: Option<i32>) -> Schematic {
        let b = |s: &str| BlockState::parse(s).unwrap();
        Schematic {
            size: [1, 1, 1],
            blocks: vec![([0, 0, 0], b("chest[facing=north,type=single,waterlogged=false]"))],
            block_entities: vec![BlockEntity {
                pos: [0, 0, 0],
                data: nbt::compound(vec![
                    ("id", Tag::String("minecraft:chest".into())),
                    ("Items", Tag::List(10, items)),
                ]),
            }],
            entities: Vec::new(),
            data_version,
        }
    }

    fn stack(entries: Vec<(&str, Tag)>) -> Tag {
        nbt::compound(entries)
    }

    #[test]
    fn legacy_stacks_read_count_and_keep_their_tag() {
        let name = stack(vec![("display", stack(vec![("Name", Tag::String(r#"{"text":"Filler"}"#.into()))]))]);
        let items = vec![
            stack(vec![("Slot", Tag::Byte(0)), ("id", Tag::String("minecraft:stone".into())), ("Count", Tag::Byte(5))]),
            stack(vec![
                ("Slot", Tag::Byte(1)),
                ("id", Tag::String("minecraft:stone".into())),
                ("Count", Tag::Byte(1)),
                ("tag", name.clone()),
            ]),
        ];
        for dv in [Some(3700), None] {
            let c = chest_with(items.clone(), dv).containers().unwrap();
            assert_eq!((c[0].items[0].count, &c[0].items[0].tag), (5, &None), "{dv:?}");
            assert_eq!(c[0].items[1].count, 1);
            assert_eq!(c[0].items[1].tag.as_deref(), Some(snbt::write(&name).as_str()));
            assert_eq!(c[0].items[1].components, None);
        }
        // From 1.20.5 on the game reads `count` and ignores `Count`, so a
        // stack with only `Count` holds one.
        let c = chest_with(items, Some(DATA_VERSION_26_3)).containers().unwrap();
        assert_eq!((c[0].items[0].count, &c[0].items[1].tag), (1, &None));
    }

    #[test]
    fn components_arrived_with_3819() {
        let stack = |entries| stack(entries);
        let items = vec![stack(vec![
            ("Slot", Tag::Byte(0)),
            ("id", Tag::String("minecraft:stone".into())),
            ("Count", Tag::Byte(5)),
            ("count", Tag::Int(2)),
        ])];
        // 3818 is the last data version with `Count`; 3819 reads `count`.
        assert_eq!(chest_with(items.clone(), Some(3818)).containers().unwrap()[0].items[0].count, 5);
        assert_eq!(chest_with(items, Some(3819)).containers().unwrap()[0].items[0].count, 2);
    }

    #[test]
    fn a_stack_that_cant_be_read_is_an_error() {
        let items = vec![stack(vec![("Slot", Tag::Byte(0)), ("count", Tag::Int(2))])];
        assert!(chest_with(items, Some(DATA_VERSION_26_3)).containers().is_err());
        let items = vec![stack(vec![("id", Tag::String("minecraft:stone".into())), ("count", Tag::Int(2))])];
        assert!(chest_with(items, Some(DATA_VERSION_26_3)).containers().is_err());
        // A legacy stack without a count is empty to the game too.
        let items = vec![stack(vec![("Slot", Tag::Byte(0)), ("id", Tag::String("minecraft:stone".into()))])];
        assert!(chest_with(items, Some(3000)).containers().unwrap().is_empty());
    }

    #[test]
    fn older_components_export_in_their_own_data_version() {
        let named = stack(vec![("minecraft:custom_name", Tag::String(r#"{"text":"Filler"}"#.into()))]);
        let items = vec![
            stack(vec![
                ("Slot", Tag::Byte(0)),
                ("id", Tag::String("minecraft:stone".into())),
                ("count", Tag::Int(1)),
                ("components", named.clone()),
            ]),
            stack(vec![("Slot", Tag::Byte(1)), ("id", Tag::String("minecraft:dirt".into())), ("count", Tag::Int(4))]),
        ];
        // A 1.21.1 file: its name is in that version's format.
        let s = chest_with(items, Some(3955));
        let c = s.containers().unwrap();
        assert_eq!((c[0].items[0].data_version, c[0].items[1].data_version), (Some(3955), None));
        let f = s.to_fixture("t", 1).unwrap().resolve(Transform::IDENTITY).unwrap();
        let back = Schematic::from_fixture(&f).unwrap();
        assert_eq!(back.data_version, Some(3955));
        for format in [Format::Litematic, Format::Sponge, Format::Structure] {
            let read = Schematic::read(&back.write(format)).unwrap();
            assert_eq!((read.data_version, read.containers().unwrap()), (Some(3955), c.clone()), "{format:?}");
        }
        // Without older data, a file is 26.3's.
        let plain = chest_with(vec![], Some(3955)).to_fixture("t", 1).unwrap().resolve(Transform::IDENTITY).unwrap();
        assert_eq!(Schematic::from_fixture(&plain).unwrap().data_version, Some(DATA_VERSION_26_3));
    }

    #[test]
    fn legacy_tags_export_before_components() {
        let name = stack(vec![("display", stack(vec![("Name", Tag::String(r#"{"text":"Filler"}"#.into()))]))]);
        let items = vec![
            stack(vec![
                ("Slot", Tag::Byte(0)),
                ("id", Tag::String("minecraft:stone".into())),
                ("Count", Tag::Byte(1)),
                ("tag", name),
            ]),
            stack(vec![("Slot", Tag::Byte(1)), ("id", Tag::String("minecraft:dirt".into())), ("Count", Tag::Byte(4))]),
        ];
        let s = chest_with(items, Some(3700));
        let f = s.to_fixture("t", 1).unwrap().resolve(Transform::IDENTITY).unwrap();
        let back = Schematic::from_fixture(&f).unwrap();
        assert_eq!(back.data_version, Some(3700));
        let written = back.block_entities[0].data.get("Items").unwrap().list().to_vec();
        assert_eq!(written[1].get("Count"), Some(&Tag::Byte(4)));
        assert!(written[1].get("count").is_none());
        assert_eq!(
            Schematic::read(&back.write(Format::Sponge)).unwrap().containers().unwrap(),
            s.containers().unwrap()
        );
        // A stack with components in 26.3's format can't share a file with
        // the older tag, and one in the older format can't go into it.
        let mut f = f;
        f.containers[0].items[1].components = Some(r#"{"minecraft:custom_name":"x"}"#.into());
        let err = Schematic::from_fixture(&f).unwrap_err().to_string();
        assert!(err.contains("the minecraft:dirt in slot 1 of the container at [0, 0, 0] is in 26.3's"), "{err}");
        f.containers[0].items[1].data_version = Some(3700);
        let err = Schematic::from_fixture(&f).unwrap_err().to_string();
        assert!(err.contains("can't hold"), "{err}");
    }

    #[test]
    fn entities_keep_their_files_data_version() {
        // An armor stand's name from a 1.20.2 file is in that file's format.
        let mut s = chest_with(Vec::new(), Some(3578));
        s.entities.push(Entity {
            id: "minecraft:armor_stand".into(),
            pos: [0.5, 1.0, 0.5],
            data: stack(vec![("CustomName", Tag::String(r#"{"text":"Stand"}"#.into()))]),
        });
        let mut f = s.to_fixture("t", 1).unwrap().resolve(Transform::IDENTITY).unwrap();
        assert_eq!(f.start_entities[0].data_version, Some(3578));
        assert_eq!(Schematic::from_fixture(&f).unwrap().data_version, Some(3578));
        // An entity in 26.3's format can't join it.
        let mut fresh = f.start_entities[0].clone();
        fresh.entity = "tnt".into();
        fresh.data_version = None;
        f.start_entities.push(fresh);
        let err = Schematic::from_fixture(&f).unwrap_err().to_string();
        assert!(err.contains("the tnt entity at [0.5, 1.0, 0.5] is in 26.3's"), "{err}");
    }

    #[test]
    fn counts_the_game_cant_read_are_left_out_or_refused() {
        let at = |slot: i8, count: i32| {
            stack(vec![
                ("Slot", Tag::Byte(slot)),
                ("id", Tag::String("minecraft:stone".into())),
                ("count", Tag::Int(count)),
            ])
        };
        let s = chest_with(vec![at(0, 0), at(1, 100), at(2, 99)], Some(DATA_VERSION_26_3));
        let c = s.containers().unwrap();
        assert_eq!(c[0].items.iter().map(|i| (i.slot, i.count)).collect::<Vec<_>>(), [(2, 99)]);
        let mut f = s.to_fixture("t", 1).unwrap().resolve(Transform::IDENTITY).unwrap();
        f.containers[0].items[0].count = 100;
        let err = format!("{:#}", Schematic::from_fixture(&f).unwrap_err());
        assert!(err.contains("a count of 100"), "{err}");
        f.containers[0].items[0].count = 1;
        f.containers[0].items[0].slot = 200;
        let err = format!("{:#}", Schematic::from_fixture(&f).unwrap_err());
        assert!(err.contains("a slot past 127"), "{err}");
    }

    #[test]
    fn a_modern_stack_drops_a_stray_tag() {
        let mut s = chest_with(Vec::new(), Some(DATA_VERSION_26_3));
        let item = stack(vec![
            ("id", Tag::String("minecraft:stone".into())),
            ("count", Tag::Int(2)),
            ("tag", stack(vec![("x", Tag::Int(1))])),
        ]);
        s.entities.push(Entity {
            id: "minecraft:item".into(),
            pos: [0.5, 1.0, 0.5],
            data: stack(vec![("Item", item)]),
        });
        let e = &s.to_fixture("t", 1).unwrap().start_entities[0];
        assert_eq!(e.nbt.as_deref(), Some(r#"{Item:{count:2,id:"minecraft:stone"}}"#));
        assert_eq!(e.data_version, None);
    }

    #[test]
    fn components_round_trip_through_fixtures_and_files() {
        let named = stack(vec![("minecraft:custom_name", Tag::String("Filler".into()))]);
        let items = vec![stack(vec![
            ("Slot", Tag::Byte(2)),
            ("id", Tag::String("minecraft:stone".into())),
            ("count", Tag::Int(1)),
            ("components", named.clone()),
        ])];
        let s = chest_with(items, Some(DATA_VERSION_26_3));
        let c = s.containers().unwrap();
        assert_eq!(c[0].items[0].components.as_deref(), Some(snbt::write(&named).as_str()));
        let f = s.to_fixture("t", 1).unwrap().resolve(Transform::IDENTITY).unwrap();
        let back = Schematic::from_fixture(&f).unwrap();
        for format in [Format::Litematic, Format::Sponge, Format::Structure] {
            assert_eq!(Schematic::read(&back.write(format)).unwrap().containers().unwrap(), c, "{format:?}");
        }
    }

    #[test]
    fn legacy_entities_count_their_items() {
        let mut s = chest_with(Vec::new(), Some(3700));
        s.entities.push(Entity {
            id: "minecraft:item".into(),
            pos: [0.5, 1.0, 0.5],
            data: stack(vec![(
                "Item",
                stack(vec![("id", Tag::String("minecraft:stone".into())), ("Count", Tag::Byte(3))]),
            )]),
        });
        let f = s.to_fixture("t", 1).unwrap();
        assert_eq!(f.start_entities[0].nbt.as_deref(), Some(r#"{Item:{count:3,id:"minecraft:stone"}}"#));
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
