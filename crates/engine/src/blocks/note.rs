//! `NoteBlock`: plays (a block event) on a rising signal or a click, and
//! takes its instrument from the block beneath it.

use crate::game_event::GameEvent;
use crate::level::{Level, flags};
use crate::pos::{Dir, Pos};
use crate::registry::StateId;

/// `BlockBehaviour.Properties.instrument` for the blocks the engine has.
/// Blocks that set none play the harp.
fn instrument(level: &Level, state: StateId) -> &'static str {
    let name = level.reg.name(state);
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    match name {
        "stone" | "cobblestone" | "obsidian" | "bedrock" | "smooth_stone" | "observer" | "stone_slab"
        | "smooth_stone_slab" | "cobblestone_slab" => "basedrum",
        n if n.ends_with("_concrete") => "basedrum",
        n if n == "glass" || n == "tinted_glass" || n.ends_with("_stained_glass") => "hat",
        "oak_slab" | "note_block" | "chest" | "trapped_chest" | "composter" => "bass",
        "white_wool" => "guitar",
        "gold_block" => "bell",
        "iron_block" => "iron_xylophone",
        _ => "harp",
    }
}

/// `setInstrument`: from the block below. Mob heads above, the only blocks
/// that play on top of a note block, are not in the engine yet.
pub(super) fn with_instrument(level: &Level, pos: Pos, state: StateId) -> StateId {
    let below = level.get(pos.below());
    level.with(state, "instrument", instrument(level, below))
}

pub fn update_shape(level: &Level, pos: Pos, state: StateId, dir: Dir) -> StateId {
    if dir.axis() == Dir::Up.axis() { with_instrument(level, pos, state) } else { state }
}

/// `playNote`: only with air above it.
fn play_note(level: &mut Level, pos: Pos, state: StateId) {
    if level.is_air(level.get(pos.above())) {
        level.block_event(pos, level.block(state), 0, 0);
        level.game_event(GameEvent::NoteBlockPlay, pos);
    }
}

pub fn neighbor_changed(level: &mut Level, pos: Pos, state: StateId) {
    let powered = level.has_neighbor_signal(pos);
    if powered != level.flag(state, "powered") {
        if powered {
            play_note(level, pos, state);
        }
        level.set_block(pos, level.with_bool(state, "powered", powered), flags::ALL);
    }
}

/// Right-click: next note, then play it.
pub fn use_without_item(level: &mut Level, pos: Pos, state: StateId) {
    let next = level.with_int(state, "note", (level.int(state, "note") + 1) % 25);
    level.set_block(pos, next, flags::ALL);
    play_note(level, pos, next);
}
