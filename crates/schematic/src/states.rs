//! Block states saved as NBT compounds: the palettes of structures and
//! litematics, and the states held in entity and block entity data (a
//! falling block's `BlockState`, a moving piston's `blockState`).
//!
//! Since data version 5006 the game saves a state's block as `id` and its
//! properties as `properties`. Older files say `Name` and `Properties`,
//! and the game's `BlockStateFieldNamesFix` renames them as it loads one;
//! 26.3 itself reads only the new keys (`NbtUtils.readBlockState`,
//! `BlockState.CODEC`). A file is read and written in the keys of its own
//! data version, and a [`Schematic`]'s entity and block entity data always
//! holds the new ones.

use std::collections::BTreeMap;

use anyhow::{Context, Result};
use tickwright_trace::BlockState;

use crate::Schematic;
use crate::nbt::Tag;

/// The first data version that saves block states as `id` and
/// `properties`: the game's `BlockStateFieldNamesFix` is schema 5006.
pub const STATE_ID_DATA_VERSION: i32 = 5006;

/// The keys a block state compound names its block and properties by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Keys {
    pub id: &'static str,
    pub properties: &'static str,
}

/// `id` and `properties`, from data version 5006 on.
pub(crate) const MODERN: Keys = Keys { id: "id", properties: "properties" };
/// `Name` and `Properties`, before data version 5006. Litematica writes
/// its palettes so in every version.
pub(crate) const LEGACY: Keys = Keys { id: "Name", properties: "Properties" };

impl Keys {
    /// The keys a file of `data_version` saves states under. A file that
    /// doesn't say is old: the game reads a structure without a version as
    /// version 500.
    pub(crate) fn of(data_version: Option<i32>) -> Keys {
        if data_version.is_some_and(|v| v >= STATE_ID_DATA_VERSION) { MODERN } else { LEGACY }
    }

    fn other(self) -> Keys {
        if self == MODERN { LEGACY } else { MODERN }
    }
}

/// A palette's block state, in the keys of its file's data version. A state
/// that has only the other keys is read by those: Litematica writes `Name`
/// in every version (and reads either), and Tickwright's own exports said
/// `Name` at data version 5023 before this was fixed.
pub(crate) fn read(tag: &Tag, data_version: Option<i32>) -> Result<BlockState> {
    let want = Keys::of(data_version);
    let keys = if tag.get(want.id).is_some() { want } else { want.other() };
    let name = tag.get(keys.id).with_context(|| format!("a block state has no `{}`", want.id))?;
    let name = name.as_str().with_context(|| format!("a block state's `{}` is not a string", keys.id))?;
    let id = if name.contains(':') { name.to_string() } else { format!("minecraft:{name}") };
    let mut props = BTreeMap::new();
    if let Some(p) = tag.get(keys.properties) {
        for (k, v) in p.entries() {
            props.insert(k.clone(), v.as_str().context("a property is not a string")?.to_string());
        }
    }
    Ok(BlockState { id, props })
}

/// A block state as a compound under `keys`.
pub(crate) fn write(state: &BlockState, keys: Keys) -> Tag {
    let mut entries = vec![(keys.id.to_string(), Tag::String(state.id.clone()))];
    if !state.props.is_empty() {
        let props = state.props.iter().map(|(k, v)| (k.clone(), Tag::String(v.clone()))).collect();
        entries.push((keys.properties.to_string(), Tag::Compound(props)));
    }
    Tag::Compound(entries)
}

/// Rename one saved block state's keys to `to`. With `replace`, as the
/// game's fixer renames, the old keys win over any new ones already there;
/// without it a state that already has `to.id` is left alone. A block id
/// saved alone as a string (`BlockState.CODEC` writes a default state so)
/// becomes a compound for an old file, whose game only read compounds.
fn rename(tag: &mut Tag, to: Keys, replace: bool) {
    let from = to.other();
    match tag {
        Tag::String(id) if to == LEGACY => {
            *tag = Tag::Compound(vec![(LEGACY.id.to_string(), Tag::String(std::mem::take(id)))]);
        }
        Tag::Compound(entries) => {
            let has = |entries: &[(String, Tag)], k: &str| entries.iter().any(|(key, _)| key == k);
            if !has(entries, from.id) || (!replace && has(entries, to.id)) {
                return;
            }
            let props = has(entries, from.properties);
            entries.retain(|(k, _)| k != to.id && !(props && k == to.properties));
            for (k, _) in entries.iter_mut() {
                if k == from.id {
                    *k = to.id.to_string();
                } else if k == from.properties {
                    *k = to.properties.to_string();
                }
            }
        }
        _ => {}
    }
}

/// The value under `key` in a compound.
fn field<'a>(tag: &'a mut Tag, key: &str) -> Option<&'a mut Tag> {
    match tag {
        Tag::Compound(entries) => entries.iter_mut().find(|(k, _)| k == key).map(|(_, v)| v),
        _ => None,
    }
}

/// Each block state in the saved data of an entity of type `id`: the fields
/// the game's data fixer types as block states, and those of its passengers
/// and of a falling block's block entity.
fn entity_states(id: &str, data: &mut Tag, f: &mut dyn FnMut(&mut Tag)) {
    let name = id.strip_prefix("minecraft:").unwrap_or(id);
    let key = match name {
        "falling_block" => Some("BlockState"),
        "tnt" | "block_display" => Some("block_state"),
        "enderman" => Some("carriedBlockState"),
        "arrow" | "spectral_arrow" | "trident" => Some("inBlockState"),
        n if n == "minecart" || n.ends_with("_minecart") => Some("DisplayState"),
        _ => None,
    };
    if let Some(state) = key.and_then(|k| field(data, k)) {
        f(state);
    }
    if name == "area_effect_cloud"
        && let Some(state) = field(data, "custom_particle").and_then(|p| field(p, "block_state"))
    {
        f(state);
    }
    if name == "falling_block"
        && let Some(be) = field(data, "TileEntityData")
    {
        let be_id = be.get("id").and_then(Tag::as_str).unwrap_or_default().to_string();
        block_entity_states(&be_id, be, f);
    }
    if let Some(Tag::List(_, passengers)) = field(data, "Passengers") {
        for p in passengers {
            let pid = p.get("id").and_then(Tag::as_str).unwrap_or_default().to_string();
            entity_states(&pid, p, f);
        }
    }
}

/// Each block state in a block entity's saved data: a moving piston's.
fn block_entity_states(id: &str, data: &mut Tag, f: &mut dyn FnMut(&mut Tag)) {
    if id == "minecraft:piston"
        && let Some(state) = field(data, "blockState")
    {
        f(state);
    }
}

impl Schematic {
    /// Rename the block states in the entity and block entity data to `to`,
    /// replacing what is there (see [`rename`]) when `replace`.
    fn rename_states(&mut self, to: Keys, replace: bool) {
        let mut f = |t: &mut Tag| rename(t, to, replace);
        for e in &mut self.entities {
            entity_states(&e.id, &mut e.data, &mut f);
        }
        for i in 0..self.block_entities.len() {
            // A block entity a file saved without its id is a moving
            // piston's if the block is one.
            let id = match self.block_entities[i].data.get("id").and_then(Tag::as_str) {
                Some(id) => id.to_string(),
                None if self.block(self.block_entities[i].pos).id == "minecraft:moving_piston" => {
                    "minecraft:piston".to_string()
                }
                None => continue,
            };
            block_entity_states(&id, &mut self.block_entities[i].data, &mut f);
        }
    }

    /// Bring the block states a file's entity and block entity data holds to
    /// the keys 26.3 reads: in a file from before data version 5006, as the
    /// game's fixer renames them, and in a newer one only where a state has
    /// `Name` and no `id`, which the game would read as nothing but which
    /// Tickwright's earlier exports wrote.
    pub(crate) fn read_states(&mut self) {
        let legacy = Keys::of(self.data_version) == LEGACY;
        self.rename_states(MODERN, legacy);
    }

    /// This schematic with its entity and block entity data's block states
    /// in the keys of the data version it is written in.
    pub(crate) fn states_for_writing(&self) -> std::borrow::Cow<'_, Schematic> {
        if Keys::of(Some(self.written_version())) == MODERN {
            return std::borrow::Cow::Borrowed(self);
        }
        let mut s = self.clone();
        s.rename_states(LEGACY, true);
        std::borrow::Cow::Owned(s)
    }

    /// The data version a file of this schematic says it is.
    pub(crate) fn written_version(&self) -> i32 {
        self.data_version.unwrap_or(crate::DATA_VERSION_26_3)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::nbt::compound;
    use crate::{BlockEntity, Entity, Format};

    fn state(keys: Keys, id: &str, props: &[(&str, &str)]) -> Tag {
        let mut e = vec![(keys.id, Tag::String(id.into()))];
        if !props.is_empty() {
            let p = props.iter().map(|(k, v)| (k.to_string(), Tag::String(v.to_string()))).collect();
            e.push((keys.properties, Tag::Compound(p)));
        }
        compound(e)
    }

    #[test]
    fn data_versions_choose_the_keys() {
        assert_eq!(Keys::of(Some(5005)), LEGACY);
        assert_eq!(Keys::of(Some(5006)), MODERN);
        assert_eq!(Keys::of(Some(crate::DATA_VERSION_26_3)), MODERN);
        assert_eq!(Keys::of(None), LEGACY);
    }

    #[test]
    fn palette_states_read_in_either_keys() {
        let repeater = BlockState::parse("repeater[delay=2]").unwrap();
        for dv in [None, Some(3955), Some(5005), Some(5006), Some(5023)] {
            for keys in [MODERN, LEGACY] {
                let t = state(keys, "minecraft:repeater", &[("delay", "2")]);
                assert_eq!(read(&t, dv).unwrap(), repeater, "{dv:?} {keys:?}");
            }
        }
        // With both, the version's own keys win, as the game reads them.
        let both = compound(vec![("Name", Tag::String("minecraft:stone".into())), ("id", Tag::String("dirt".into()))]);
        assert_eq!(read(&both, Some(5005)).unwrap().id, "minecraft:stone");
        assert_eq!(read(&both, Some(5023)).unwrap().id, "minecraft:dirt");
        let err = read(&compound(vec![]), Some(5023)).unwrap_err().to_string();
        assert!(err.contains("no `id`"), "{err}");
    }

    fn falling(block_state: Tag) -> Entity {
        Entity {
            id: "minecraft:falling_block".into(),
            pos: [0.5, 1.0, 0.5],
            data: compound(vec![("BlockState", block_state), ("Time", Tag::Int(1))]),
        }
    }

    fn piston(block_state: Tag) -> BlockEntity {
        BlockEntity {
            pos: [0, 0, 0],
            data: compound(vec![
                ("id", Tag::String("minecraft:piston".into())),
                ("blockState", block_state),
                ("facing", Tag::Int(1)),
            ]),
        }
    }

    fn with_states(data_version: i32, keys: Keys) -> Schematic {
        let mut cart = Entity {
            id: "minecraft:minecart".into(),
            pos: [0.5, 1.0, 0.5],
            data: compound(vec![("DisplayState", state(keys, "minecraft:gravel", &[]))]),
        };
        // A falling block riding the minecart.
        let mut rider = match falling(state(keys, "minecraft:anvil", &[("facing", "east")])).saved() {
            Tag::Compound(e) => e,
            _ => unreachable!(),
        };
        rider.retain(|(k, _)| k != "Pos");
        if let Tag::Compound(e) = &mut cart.data {
            e.push(("Passengers".into(), Tag::List(10, vec![Tag::Compound(rider)])));
        }
        Schematic {
            size: [1, 2, 1],
            blocks: vec![([0, 0, 0], BlockState::parse("moving_piston[facing=up,type=normal]").unwrap())],
            block_entities: vec![piston(state(keys, "minecraft:stone", &[]))],
            entities: vec![falling(state(keys, "minecraft:gravel", &[])), cart],
            data_version: Some(data_version),
        }
    }

    #[test]
    fn files_keep_the_keys_of_their_data_version() {
        let modern = with_states(crate::DATA_VERSION_26_3, MODERN);
        for (dv, keys) in [(crate::DATA_VERSION_26_3, MODERN), (5006, MODERN), (5005, LEGACY), (3955, LEGACY)] {
            let mut s = modern.clone();
            s.data_version = Some(dv);
            for format in [Format::Litematic, Format::Sponge, Format::Structure] {
                let bytes = s.write(format);
                // On disk: the version's keys.
                let (_, root) = crate::nbt::read(&bytes).unwrap();
                let text = format!("{root:?}");
                let (want, not) = (format!("{:?}", keys.id), format!("{:?}", keys.other().id));
                assert!(text.contains(&format!("{want}, String(\"minecraft:gravel\")")), "{dv} {format:?}: {text}");
                assert!(!text.contains(&format!("{not}, String(\"minecraft:stone\")")), "{dv} {format:?}: {text}");
                // Read back: the new keys, whatever the file said.
                let back = Schematic::read(&bytes).unwrap();
                assert_eq!(back.entities, modern.entities, "{dv} {format:?}");
                assert_eq!(back.block_entities, modern.block_entities, "{dv} {format:?}");
            }
        }
    }

    #[test]
    fn old_files_are_brought_to_the_new_keys() {
        let mut s = with_states(3955, LEGACY);
        s.read_states();
        let modern = with_states(crate::DATA_VERSION_26_3, MODERN);
        assert_eq!(s.entities, modern.entities);
        assert_eq!(s.block_entities, modern.block_entities);
    }

    #[test]
    fn a_new_file_keeps_ids_and_reads_a_stray_name() {
        let both = compound(vec![
            ("Name", Tag::String("minecraft:gravel".into())),
            ("id", Tag::String("minecraft:red_sand".into())),
        ]);
        let mut s = Schematic {
            entities: vec![falling(both.clone()), falling(state(LEGACY, "minecraft:gravel", &[]))],
            data_version: Some(crate::DATA_VERSION_26_3),
            ..Default::default()
        };
        s.read_states();
        assert_eq!(s.entities[0], falling(both));
        assert_eq!(s.entities[1], falling(state(MODERN, "minecraft:gravel", &[])));
        // In an old file the fixer's rename wins.
        let mut old = Schematic {
            entities: vec![falling(compound(vec![
                ("Name", Tag::String("minecraft:gravel".into())),
                ("id", Tag::String("minecraft:red_sand".into())),
            ]))],
            data_version: Some(3955),
            ..Default::default()
        };
        old.read_states();
        assert_eq!(old.entities[0], falling(state(MODERN, "minecraft:gravel", &[])));
    }

    #[test]
    fn a_state_saved_as_its_block_id_is_written_as_a_compound_for_old_files() {
        let s = Schematic {
            entities: vec![falling(Tag::String("minecraft:red_sand".into()))],
            data_version: Some(3955),
            ..Default::default()
        };
        let old = s.states_for_writing();
        assert_eq!(old.entities[0], falling(state(LEGACY, "minecraft:red_sand", &[])));
        let mut new = s.clone();
        new.data_version = Some(crate::DATA_VERSION_26_3);
        assert_eq!(new.states_for_writing().entities[0], falling(Tag::String("minecraft:red_sand".into())));
    }
}
