//! Java Edition 26.3: the first module, so it names every block the engine
//! simulates.

use super::Module;
use crate::blocks::Kind;
use crate::registry::Registry;

pub const MODULE: Module = Module { version: "26.3", base: None, registry: || Registry::get("26.3"), block };

fn block(name: &str) -> Option<Kind> {
    Some(match name {
        "air" | "cave_air" | "void_air" => Kind::Air,
        "stone" | "obsidian" | "cobblestone" | "dirt" | "smooth_stone" | "white_wool" | "iron_block" | "gold_block"
        | "diamond_block" | "bedrock" => Kind::Solid,
        n if n.ends_with("_concrete") => Kind::Solid,
        "lever" => Kind::Lever,
        "stone_button" | "polished_blackstone_button" => Kind::StoneButton,
        n if n == "glass" || n == "tinted_glass" || n.ends_with("_stained_glass") => Kind::Glass,
        "stone_slab" | "smooth_stone_slab" | "cobblestone_slab" | "oak_slab" => Kind::Slab,
        "redstone_block" => Kind::RedstoneBlock,
        "redstone_lamp" => Kind::RedstoneLamp,
        "redstone_wire" => Kind::RedstoneWire,
        "redstone_torch" => Kind::RedstoneTorch,
        "redstone_wall_torch" => Kind::RedstoneWallTorch,
        "repeater" => Kind::Repeater,
        "piston" => Kind::Piston,
        "sticky_piston" => Kind::StickyPiston,
        "piston_head" => Kind::PistonHead,
        "moving_piston" => Kind::MovingPiston,
        "slime_block" => Kind::Slime,
        "honey_block" => Kind::Honey,
        "chest" | "trapped_chest" => Kind::Chest,
        "observer" => Kind::Observer,
        "comparator" => Kind::Comparator,
        "composter" => Kind::Composter,
        "cake" => Kind::Cake,
        _ => return None,
    })
}
