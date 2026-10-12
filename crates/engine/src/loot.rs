//! What blocks drop when they break, from `data/<version>/loot.json`: the
//! tables a break with no tool settles without drawing a random number.
//! Each block drops one item, nothing, or per pool the first rule whose
//! block state matches (doors and beds drop from one half).

use std::collections::HashMap;
use std::sync::LazyLock;

use serde::Deserialize;

use crate::entity::Stack;
use crate::level::Level;
use crate::registry::StateId;

#[derive(Deserialize)]
#[serde(untagged)]
enum Table {
    /// `"minecraft:<item>"`, `""` for nothing, or `"complex"`.
    Item(String),
    Pools {
        pools: Vec<Vec<Rule>>,
    },
}

#[derive(Deserialize)]
struct Rule {
    #[serde(default)]
    when: HashMap<String, String>,
    item: String,
    #[serde(default = "one")]
    count: i32,
}

fn one() -> i32 {
    1
}

static LOOT_26_3: LazyLock<HashMap<String, Table>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../data/26.3/loot.json")).expect("data/26.3/loot.json is valid")
});

fn stack(item: &str, count: i32) -> Stack {
    Stack::new(item.strip_prefix("minecraft:").unwrap_or(item), count)
}

/// `BlockState.getDrops` for a break with no tool. Tables the engine can't
/// evaluate exactly mark the run unsupported and drop nothing.
pub fn block_drops(level: &mut Level, state: StateId) -> Vec<Stack> {
    let name = level.reg.name(state).to_string();
    match LOOT_26_3.get(&name) {
        None => Vec::new(),
        Some(Table::Item(item)) => match item.as_str() {
            "" => Vec::new(),
            "complex" => {
                level.unsupported(format!("what {name} drops"));
                Vec::new()
            }
            item => vec![stack(item, 1)],
        },
        Some(Table::Pools { pools }) => pools
            .iter()
            .filter_map(|rules| {
                rules.iter().find(|r| r.when.iter().all(|(k, v)| level.reg.value(state, k) == Some(v.as_str())))
            })
            .map(|r| stack(&r.item, r.count))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::block_drops;
    use crate::level::Level;
    use crate::rules::Rules;

    fn drops(level: &mut Level, state: &str) -> Vec<(String, i32)> {
        let s = level.reg.parse(state).unwrap();
        block_drops(level, s).into_iter().map(|st| (st.id, st.count)).collect()
    }

    #[test]
    fn doors_and_beds_drop_from_one_half() {
        let mut level = Level::new(Rules::for_version("26.3").unwrap());
        assert_eq!(drops(&mut level, "oak_door[half=lower]"), [("oak_door".to_string(), 1)]);
        assert!(drops(&mut level, "oak_door[half=upper]").is_empty());
        assert_eq!(drops(&mut level, "red_bed[part=head]"), [("red_bed".to_string(), 1)]);
        assert!(drops(&mut level, "red_bed[part=foot]").is_empty());
        // Silk touch only, and stone falls back to cobblestone.
        assert!(drops(&mut level, "glass").is_empty());
        assert_eq!(drops(&mut level, "stone"), [("cobblestone".to_string(), 1)]);
        assert!(level.unsupported.is_none());
    }
}
