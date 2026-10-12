//! What blocks drop when they break, from `data/<version>/loot.json`: only
//! tables that always give one item, so a drop never draws a random number.

use std::collections::HashMap;
use std::sync::LazyLock;

use crate::entity::Stack;
use crate::level::Level;
use crate::registry::StateId;

static LOOT_26_3: LazyLock<HashMap<String, String>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../../../data/26.3/loot.json")).expect("data/26.3/loot.json is valid")
});

/// `BlockState.getDrops` for a break with no tool. Tables the engine can't
/// evaluate exactly mark the run unsupported and drop nothing.
pub fn block_drops(level: &mut Level, state: StateId) -> Vec<Stack> {
    let name = level.reg.name(state).to_string();
    match LOOT_26_3.get(&name).map(|s| s.as_str()) {
        None | Some("") => Vec::new(),
        Some("complex") => {
            level.unsupported(format!("what {name} drops"));
            Vec::new()
        }
        Some(item) => vec![Stack::new(item.strip_prefix("minecraft:").unwrap_or(item), 1)],
    }
}
