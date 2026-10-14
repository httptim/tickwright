//! The items a design takes to build: a material list with stack and shulker
//! box counts.
//!
//! Which items each block state takes comes from the game's own data, in
//! `data/<version>/materials.json`, which `tools/materials.py` generates
//! from the server jar's loot tables and item report (see that script for
//! the rules). So a double slab is two slabs, a door or bed is counted once,
//! redstone wire is redstone dust, a water source is a water bucket, and
//! candles, sea pickles and the like count by how many the block holds. A
//! waterlogged block takes a water bucket on top of its own items, and
//! container contents count as well.
//!
//! The same file is what the companion mod reads, so the editor's list and
//! the mod's gathering agree.

use std::collections::{BTreeMap, HashMap};
use std::sync::OnceLock;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use tickwright_trace::fixture::Container;
use tickwright_trace::{BlockState, ResolvedFixture};

/// Slots in a shulker box.
pub const SHULKER_BOX_SLOTS: u64 = 27;

/// Versions with a material table.
pub const VERSIONS: &[&str] = &["26.3"];

const DATA_26_3: &str = include_str!("../../../data/26.3/materials.json");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    waterlogged: String,
    stack_sizes: HashMap<String, u32>,
    blocks: HashMap<String, RawBlock>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum RawBlock {
    Fixed { items: Items },
    By { by: Vec<String>, default: String, cases: HashMap<String, Items> },
}

type Items = BTreeMap<String, u32>;

/// One version's table of what every block state takes and how items stack.
pub struct Table {
    raw: Raw,
}

impl Table {
    /// The table for a game version, such as `26.3`.
    pub fn for_version(version: &str) -> Result<&'static Table> {
        static T26_3: OnceLock<Table> = OnceLock::new();
        match version {
            "26.3" => Ok(T26_3.get_or_init(|| Table::parse(DATA_26_3).expect("data/26.3/materials.json is valid"))),
            _ => bail!("no material table for version {version:?}; have {}", VERSIONS.join(", ")),
        }
    }

    fn parse(text: &str) -> Result<Table> {
        Ok(Table { raw: serde_json::from_str(text).context("reading the material table")? })
    }

    /// How many of `item` fit in one slot: 64, 16 or 1.
    pub fn stack_size(&self, item: &str) -> u32 {
        self.raw.stack_sizes.get(&full_id(item)).copied().unwrap_or(64)
    }

    /// The items one block state takes. A property the state leaves out
    /// reads as the block's default. `None` if the block is not in the table
    /// (a block from a mod or another version).
    pub fn block(&self, state: &BlockState) -> Option<Vec<(String, u32)>> {
        let items = match self.raw.blocks.get(&state.id)? {
            RawBlock::Fixed { items } => items,
            RawBlock::By { by, default, cases } => {
                let defaults: Vec<&str> = default.split(',').collect();
                let key: Vec<&str> = by.iter().zip(&defaults).map(|(k, d)| state.get(k).unwrap_or(d)).collect();
                cases.get(&key.join(",")).or_else(|| cases.get(default))?
            }
        };
        let mut out: Vec<(String, u32)> = items.iter().map(|(k, &v)| (k.clone(), v)).collect();
        if state.get("waterlogged") == Some("true") {
            out.push((self.raw.waterlogged.clone(), 1));
        }
        Some(out)
    }

    /// The material list for a set of blocks and the containers among them.
    pub fn list<'a>(&self, blocks: impl IntoIterator<Item = &'a BlockState>, containers: &[Container]) -> MaterialList {
        let mut placed: BTreeMap<String, u64> = BTreeMap::new();
        let mut held: BTreeMap<String, u64> = BTreeMap::new();
        let mut unknown: BTreeMap<String, u64> = BTreeMap::new();
        for state in blocks {
            match self.block(state) {
                Some(items) => {
                    for (item, n) in items {
                        *placed.entry(item).or_default() += u64::from(n);
                    }
                }
                None => *unknown.entry(state.id.clone()).or_default() += 1,
            }
        }
        for c in containers {
            for slot in &c.items {
                // A slot holds at most a stack: filling a container cuts a
                // bigger count down (`Container.setItem`).
                let n = slot.count.min(self.stack_size(&slot.id));
                *held.entry(full_id(&slot.id)).or_default() += u64::from(n);
            }
        }

        let mut ids: Vec<&String> = placed.keys().chain(held.keys()).collect();
        ids.sort();
        ids.dedup();
        let mut items: Vec<Material> = ids
            .into_iter()
            .map(|id| {
                let from_blocks = placed.get(id).copied().unwrap_or(0);
                let from_containers = held.get(id).copied().unwrap_or(0);
                Material::new(id.clone(), from_blocks, from_containers, self.stack_size(id))
            })
            .collect();
        items.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.item.cmp(&b.item)));
        let slots = items.iter().map(|m| m.slots).sum();
        MaterialList {
            items,
            slots,
            shulker_boxes: slots.div_ceil(SHULKER_BOX_SLOTS),
            unknown_blocks: unknown.into_iter().map(|(block, count)| UnknownBlock { block, count }).collect(),
        }
    }

    /// The material list for a resolved fixture: its blocks and the
    /// contents its containers start with.
    pub fn list_fixture(&self, fixture: &ResolvedFixture) -> Result<MaterialList> {
        let states = fixture.blocks.iter().map(|(_, _, _, s)| BlockState::parse(s)).collect::<Result<Vec<_>>>()?;
        Ok(self.list(&states, &fixture.containers))
    }
}

/// `stone` and `minecraft:stone` are the same item.
fn full_id(id: &str) -> String {
    if id.contains(':') { id.to_string() } else { format!("minecraft:{id}") }
}

/// A material list, most needed item first.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MaterialList {
    pub items: Vec<Material>,
    /// Inventory slots the whole list fills.
    pub slots: u64,
    /// Shulker boxes the whole list fills, packing items of different kinds
    /// into the same box.
    pub shulker_boxes: u64,
    /// Blocks the table does not know, so they are not in `items`.
    pub unknown_blocks: Vec<UnknownBlock>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct UnknownBlock {
    pub block: String,
    pub count: u64,
}

/// One item on the list. `count` is `shulker_boxes` full boxes, then
/// `stacks` full stacks, then `extra` more.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Material {
    pub item: String,
    pub count: u64,
    /// Taken by the blocks themselves.
    pub from_blocks: u64,
    /// Held in the design's containers.
    pub from_containers: u64,
    pub stack_size: u32,
    /// Slots it fills: one per stack, so 1-stacking items take one each.
    pub slots: u64,
    pub shulker_boxes: u64,
    pub stacks: u64,
    pub extra: u64,
}

impl Material {
    fn new(item: String, from_blocks: u64, from_containers: u64, stack_size: u32) -> Material {
        let count = from_blocks + from_containers;
        let per = u64::from(stack_size.max(1));
        let per_box = per * SHULKER_BOX_SLOTS;
        Material {
            item,
            count,
            from_blocks,
            from_containers,
            stack_size,
            slots: count.div_ceil(per),
            shulker_boxes: count / per_box,
            stacks: count % per_box / per,
            extra: count % per,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tickwright_trace::fixture::ItemSlot;

    fn table() -> &'static Table {
        Table::for_version("26.3").unwrap()
    }

    fn list(states: &[&str]) -> MaterialList {
        list_with(states, &[])
    }

    fn list_with(states: &[&str], containers: &[Container]) -> MaterialList {
        let states: Vec<BlockState> = states.iter().map(|s| BlockState::parse(s).unwrap()).collect();
        table().list(&states, containers)
    }

    fn counts(l: &MaterialList) -> Vec<(&str, u64)> {
        l.items.iter().map(|m| (m.item.strip_prefix("minecraft:").unwrap(), m.count)).collect()
    }

    #[test]
    fn two_block_doors_and_beds_count_once() {
        let l = list(&[
            "oak_door[half=lower,facing=north,hinge=left,open=false,powered=false]",
            "oak_door[half=upper,facing=north,hinge=left,open=false,powered=false]",
            "red_bed[part=foot,facing=north,occupied=false]",
            "red_bed[part=head,facing=north,occupied=false]",
            "tall_grass[half=lower]",
            "tall_grass[half=upper]",
        ]);
        assert_eq!(counts(&l), [("short_grass", 2), ("oak_door", 1), ("red_bed", 1)]);
    }

    #[test]
    fn double_slabs_take_two_and_waterlogging_takes_a_bucket() {
        let l = list(&[
            "smooth_stone_slab[type=double,waterlogged=false]",
            "smooth_stone_slab[type=bottom,waterlogged=true]",
            "smooth_stone_slab[type=top]",
        ]);
        assert_eq!(counts(&l), [("smooth_stone_slab", 4), ("water_bucket", 1)]);
        let bucket = &l.items[1];
        assert_eq!((bucket.stack_size, bucket.slots), (1, 1));
    }

    #[test]
    fn redstone_parts_are_the_items_that_place_them() {
        let l = list(&[
            "redstone_wire[east=side,north=none,power=7,south=none,west=up]",
            "redstone_wire",
            "redstone_wall_torch[facing=east,lit=true]",
            "tripwire[attached=true]",
            "piston[extended=true,facing=up]",
            "piston_head[facing=up,short=false,type=normal]",
            "moving_piston",
        ]);
        assert_eq!(counts(&l), [("redstone", 2), ("piston", 1), ("redstone_torch", 1), ("string", 1)]);
    }

    #[test]
    fn fluids_and_counted_blocks() {
        let l = list(&[
            "water[level=0]",
            "water[level=3]",
            "lava",
            "candle[candles=3,lit=false]",
            "sea_pickle[pickles=4,waterlogged=true]",
            "snow[layers=8]",
            "glow_lichen[down=true,east=true,north=false,south=false,up=false,west=true]",
            "potted_cactus",
            "wheat[age=7]",
            "cocoa[age=2,facing=north]",
        ]);
        assert_eq!(
            counts(&l),
            [
                ("snow", 8),
                ("sea_pickle", 4),
                ("candle", 3),
                ("glow_lichen", 3),
                ("water_bucket", 2),
                ("cactus", 1),
                ("cocoa_beans", 1),
                ("flower_pot", 1),
                ("lava_bucket", 1),
                ("wheat_seeds", 1),
            ]
        );
    }

    #[test]
    fn container_contents_count_with_their_stack_sizes() {
        let chest = Container {
            pos: [0, 0, 0],
            items: vec![
                ItemSlot { slot: 0, id: "ender_pearl".into(), count: 16 },
                ItemSlot { slot: 1, id: "ender_pearl".into(), count: 16 },
                ItemSlot { slot: 2, id: "ender_pearl".into(), count: 8 },
                ItemSlot { slot: 3, id: "minecraft:redstone".into(), count: 64 },
                ItemSlot { slot: 4, id: "diamond_sword".into(), count: 1 },
            ],
        };
        let l = list_with(&["chest[facing=north,type=single,waterlogged=false]", "redstone_wire"], &[chest]);
        let get = |id: &str| l.items.iter().find(|m| m.item == format!("minecraft:{id}")).unwrap();
        let pearls = get("ender_pearl");
        assert_eq!((pearls.count, pearls.stack_size, pearls.slots, pearls.stacks, pearls.extra), (40, 16, 3, 2, 8));
        let dust = get("redstone");
        assert_eq!((dust.count, dust.from_blocks, dust.from_containers, dust.slots), (65, 1, 64, 2));
        assert_eq!(get("diamond_sword").stack_size, 1);
        assert_eq!(l.slots, 3 + 2 + 1 + 1);
        assert_eq!(l.shulker_boxes, 1);
    }

    #[test]
    fn a_slot_holds_at_most_a_stack() {
        let chest = Container {
            pos: [0, 0, 0],
            items: vec![
                ItemSlot { slot: 0, id: "ender_pearl".into(), count: 20 },
                ItemSlot { slot: 1, id: "diamond_sword".into(), count: 3 },
                ItemSlot { slot: 2, id: "stone".into(), count: 64 },
            ],
        };
        let l = list_with(&["chest[facing=north,type=single,waterlogged=false]"], &[chest]);
        let held = |id: &str| l.items.iter().find(|m| m.item == format!("minecraft:{id}")).unwrap().from_containers;
        assert_eq!((held("ender_pearl"), held("diamond_sword"), held("stone")), (16, 1, 64));
    }

    #[test]
    fn shulker_box_arithmetic() {
        let m = Material::new("minecraft:stone".into(), 64 * 27 * 2 + 64 * 3 + 5, 0, 64);
        assert_eq!((m.shulker_boxes, m.stacks, m.extra, m.slots), (2, 3, 5, 58));
        let m = Material::new("minecraft:oak_sign".into(), 16 * 27, 0, 16);
        assert_eq!((m.shulker_boxes, m.stacks, m.extra), (1, 0, 0));
        let l = list(&["stone"; 64 * 27 + 1]);
        assert_eq!((l.slots, l.shulker_boxes), (28, 2));
    }

    #[test]
    fn unknown_blocks_are_reported_not_counted() {
        let l = list(&["create:cogwheel", "create:cogwheel", "stone"]);
        assert_eq!(counts(&l), [("stone", 1)]);
        assert_eq!(l.unknown_blocks, [UnknownBlock { block: "create:cogwheel".into(), count: 2 }]);
        assert!(Table::for_version("1.8.9").is_err());
    }

    #[test]
    fn every_block_and_item_in_the_table_is_in_the_game() {
        let blocks: HashMap<String, serde_json::Value> =
            serde_json::from_str(include_str!("../../../data/26.3/blocks.json")).unwrap();
        let items: Vec<String> = serde_json::from_str(include_str!("../../../data/26.3/items.json")).unwrap();
        let t = table();
        assert_eq!(t.raw.blocks.len(), blocks.len());
        for (id, b) in &blocks {
            let mut state = BlockState::parse(id).unwrap();
            for (k, v) in b["default"].as_object().unwrap() {
                state.props.insert(k.clone(), v.as_str().unwrap().into());
            }
            for (item, n) in t.block(&state).unwrap_or_else(|| panic!("{id} is not in the table")) {
                assert!(items.contains(&item), "{id} takes {item}, which is not an item");
                assert!(n > 0);
            }
        }
        for item in t.raw.stack_sizes.keys() {
            assert!(items.contains(item), "{item} has a stack size but is not an item");
        }
    }
}
