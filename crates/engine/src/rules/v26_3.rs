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
        n if n.ends_with("_concrete")
            || n.ends_with("_wool")
            || (n.ends_with("_terracotta") && !n.contains("glazed")) =>
        {
            Kind::Solid
        }
        "quartz_block" | "chiseled_quartz_block" | "smooth_quartz" | "quartz_pillar" => Kind::Solid,
        n if n.ends_with("_log") || n.ends_with("_wood") || n.ends_with("_stem") || n.ends_with("_hyphae") => Kind::Log,
        n if n.ends_with("_leaves") => Kind::Leaves,
        "target" => Kind::Target,
        "jukebox" => Kind::Jukebox,
        n if n == "shulker_box" || n.ends_with("_shulker_box") => Kind::ShulkerBox,
        "tripwire_hook" => Kind::TripwireHook,
        "lever" => Kind::Lever,
        n if n.ends_with("_button") => Kind::Button,
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
        "note_block" => Kind::NoteBlock,
        "powered_rail" | "activator_rail" => Kind::PoweredRail,
        "water" => Kind::Water,
        "hopper" => Kind::Hopper,
        "dropper" => Kind::Dropper,
        "dispenser" => Kind::Dispenser,
        "barrel" => Kind::Barrel,
        "powder_snow" => Kind::PowderSnow,
        "waxed_copper_bulb"
        | "waxed_exposed_copper_bulb"
        | "waxed_weathered_copper_bulb"
        | "waxed_oxidized_copper_bulb" => Kind::CopperBulb,
        "bell" => Kind::Bell,
        n if n.ends_with("_trapdoor") => Kind::Trapdoor,
        n if n.ends_with("_door") => Kind::Door,
        _ => return None,
    })
}
