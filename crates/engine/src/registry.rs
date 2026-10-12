//! Every block and block state in the game, from `data/<version>/blocks.json`.
//!
//! A [`StateId`] is a single integer per block state, so states are cheap to
//! copy, compare and store. Each block owns a contiguous range of ids starting
//! at the game's own first id for that block; within the range, a state's
//! offset is its property values read as a mixed-radix number, last property
//! fastest, the way the game numbers them.

use std::collections::HashMap;
use std::sync::LazyLock;

use anyhow::{Context, Result, bail};
use tickwright_trace::BlockState;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct BlockId(pub u16);

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, PartialOrd, Ord)]
pub struct StateId(pub u32);

#[derive(Debug)]
pub struct Property {
    pub name: String,
    pub values: Vec<String>,
}

#[derive(Debug)]
pub struct BlockInfo {
    pub name: String,
    pub properties: Vec<Property>,
    pub first_state: u32,
    pub states: u32,
    pub default: StateId,
}

pub struct Registry {
    pub version: &'static str,
    blocks: Vec<BlockInfo>,
    by_name: HashMap<String, BlockId>,
    /// Block ids sorted by first state, for state-to-block lookup.
    by_first_state: Vec<(u32, BlockId)>,
}

static REGISTRY_26_3: LazyLock<Registry> = LazyLock::new(|| {
    Registry::from_json("26.3", include_str!("../../../data/26.3/blocks.json")).expect("bundled 26.3 block data")
});

impl Registry {
    pub fn get(version: &str) -> Result<&'static Registry> {
        match version {
            "26.3" => Ok(&REGISTRY_26_3),
            v => bail!("no block data for Minecraft {v}"),
        }
    }

    pub fn from_json(version: &'static str, text: &str) -> Result<Registry> {
        let raw: serde_json::Map<String, serde_json::Value> = serde_json::from_str(text)?;
        let mut blocks = Vec::with_capacity(raw.len());
        let mut by_name = HashMap::new();
        for (name, b) in raw {
            let properties: Vec<Property> = b["properties"]
                .as_object()
                .context("properties")?
                .iter()
                .map(|(k, v)| Property {
                    name: k.clone(),
                    values: v.as_array().unwrap().iter().map(|s| s.as_str().unwrap().to_string()).collect(),
                })
                .collect();
            let first_state = b["first_state"].as_u64().context("first_state")? as u32;
            let states = b["states"].as_u64().context("states")? as u32;
            let mut info =
                BlockInfo { name: name.clone(), properties, first_state, states, default: StateId(first_state) };
            let default = b["default"].as_object().context("default")?;
            let mut offset = 0;
            for p in &info.properties {
                let v = default[&p.name].as_str().unwrap();
                let i = p.values.iter().position(|x| x == v).context("default value")?;
                offset = offset * p.values.len() as u32 + i as u32;
            }
            info.default = StateId(first_state + offset);
            by_name.insert(name, BlockId(blocks.len() as u16));
            blocks.push(info);
        }
        let mut by_first_state: Vec<(u32, BlockId)> =
            blocks.iter().enumerate().map(|(i, b)| (b.first_state, BlockId(i as u16))).collect();
        by_first_state.sort();
        Ok(Registry { version, blocks, by_name, by_first_state })
    }

    pub fn len(&self) -> usize {
        self.blocks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.blocks.is_empty()
    }

    pub fn block(&self, id: BlockId) -> &BlockInfo {
        &self.blocks[id.0 as usize]
    }

    pub fn block_id(&self, name: &str) -> Option<BlockId> {
        let name = if name.contains(':') { name.to_string() } else { format!("minecraft:{name}") };
        self.by_name.get(&name).copied()
    }

    pub fn block_of(&self, state: StateId) -> BlockId {
        let i = self.by_first_state.partition_point(|(first, _)| *first <= state.0) - 1;
        self.by_first_state[i].1
    }

    fn offset_parts(&self, state: StateId) -> (&BlockInfo, Vec<usize>) {
        let info = self.block(self.block_of(state));
        let mut rem = state.0 - info.first_state;
        let mut idx = vec![0; info.properties.len()];
        for (i, p) in info.properties.iter().enumerate().rev() {
            let n = p.values.len() as u32;
            idx[i] = (rem % n) as usize;
            rem /= n;
        }
        (info, idx)
    }

    fn compose(info: &BlockInfo, idx: &[usize]) -> StateId {
        let mut offset = 0u32;
        for (p, &i) in info.properties.iter().zip(idx) {
            offset = offset * p.values.len() as u32 + i as u32;
        }
        StateId(info.first_state + offset)
    }

    /// The value of property `name`, if the state's block has it.
    pub fn value<'a>(&'a self, state: StateId, name: &str) -> Option<&'a str> {
        let (info, idx) = self.offset_parts(state);
        let i = info.properties.iter().position(|p| p.name == name)?;
        Some(info.properties[i].values[idx[i]].as_str())
    }

    /// The same state with property `name` set to `value`.
    ///
    /// Panics if the block has no such property or value: that is a bug in a
    /// block's behaviour, not something a fixture can cause.
    pub fn with(&self, state: StateId, name: &str, value: &str) -> StateId {
        let (info, mut idx) = self.offset_parts(state);
        let i = info
            .properties
            .iter()
            .position(|p| p.name == name)
            .unwrap_or_else(|| panic!("{} has no property {name}", info.name));
        idx[i] = info.properties[i]
            .values
            .iter()
            .position(|v| v == value)
            .unwrap_or_else(|| panic!("{}.{name} has no value {value}", info.name));
        Self::compose(info, &idx)
    }

    pub fn parse(&self, text: &str) -> Result<StateId> {
        let bs = BlockState::parse(text)?;
        let id = self.block_id(&bs.id).with_context(|| format!("unknown block {}", bs.id))?;
        let info = self.block(id);
        let mut state = info.default;
        for (k, v) in &bs.props {
            let Some(p) = info.properties.iter().find(|p| &p.name == k) else {
                bail!("{} has no property {k}", info.name);
            };
            if !p.values.contains(v) {
                bail!("{}.{k} has no value {v}", info.name);
            }
            state = self.with(state, k, v);
        }
        Ok(state)
    }

    /// The state as the game prints it, properties in canonical (sorted) order.
    pub fn format(&self, state: StateId) -> String {
        let (info, idx) = self.offset_parts(state);
        let mut bs = BlockState { id: info.name.clone(), props: Default::default() };
        for (p, i) in info.properties.iter().zip(idx) {
            bs.props.insert(p.name.clone(), p.values[i].clone());
        }
        bs.to_string()
    }

    pub fn name(&self, state: StateId) -> &str {
        &self.block(self.block_of(state)).name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_states() {
        let r = Registry::get("26.3").unwrap();
        for s in [
            "minecraft:stone",
            "minecraft:lever[face=floor,facing=north,powered=true]",
            "minecraft:redstone_wire[east=side,north=none,power=7,south=up,west=none]",
            "minecraft:repeater[delay=3,facing=west,locked=false,powered=true]",
        ] {
            let id = r.parse(s).unwrap();
            assert_eq!(r.format(id), s);
        }
        assert_eq!(r.format(r.block(r.block_id("air").unwrap()).default), "minecraft:air");
    }

    #[test]
    fn with_changes_one_property() {
        let r = Registry::get("26.3").unwrap();
        let lever = r.parse("lever[face=floor,facing=north,powered=false]").unwrap();
        let on = r.with(lever, "powered", "true");
        assert_eq!(r.value(on, "powered"), Some("true"));
        assert_eq!(r.value(on, "face"), Some("floor"));
        assert_eq!(r.block_of(on), r.block_of(lever));
    }
}
