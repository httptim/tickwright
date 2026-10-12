//! `DoorBlock` and `TrapDoorBlock`. Both open with power and, unless iron,
//! by hand. Their writes use flag 2, so opening tells no neighbors; only
//! shape updates go out.

use crate::game_event::GameEvent;
use crate::level::{Level, flags};
use crate::pos::{Dir, Pos};
use crate::registry::StateId;

/// `BlockSetType.canOpenByHand`: everything but iron.
fn opens_by_hand(level: &Level, state: StateId) -> bool {
    !level.reg.name(state).starts_with("minecraft:iron_")
}

pub fn trapdoor_neighbor_changed(level: &mut Level, pos: Pos, state: StateId) {
    let powered = level.has_neighbor_signal(pos);
    if powered != level.flag(state, "powered") {
        if level.flag(state, "open") != powered {
            level.game_event(open_event(powered), pos);
        }
        let state = level.with_bool(state, "open", powered);
        level.set_block(pos, level.with_bool(state, "powered", powered), flags::CLIENTS);
    }
}

/// `BLOCK_OPEN` or `BLOCK_CLOSE`.
fn open_event(open: bool) -> GameEvent {
    if open { GameEvent::BlockOpen } else { GameEvent::BlockClose }
}

pub fn trapdoor_use(level: &mut Level, pos: Pos, state: StateId) {
    if opens_by_hand(level, state) {
        let open = !level.flag(state, "open");
        level.set_block(pos, level.with_bool(state, "open", open), flags::CLIENTS);
        level.game_event(open_event(open), pos);
    }
}

/// A closed trapdoor is a thin slab at its top or bottom; an open one stands
/// against the side opposite its facing.
pub fn trapdoor_face_sturdy(level: &Level, state: StateId, dir: Dir) -> bool {
    if level.flag(state, "open") {
        dir == level.dir(state, "facing").opposite()
    } else if level.prop(state, "half") == "top" {
        dir == Dir::Up
    } else {
        dir == Dir::Down
    }
}

fn is_lower(level: &Level, state: StateId) -> bool {
    level.prop(state, "half") == "lower"
}

pub fn door_neighbor_changed(level: &mut Level, pos: Pos, state: StateId, source: crate::registry::BlockId) {
    // Updates from the door's own other half are ignored.
    if level.is(state, source) {
        return;
    }
    let other = pos.offset(if is_lower(level, state) { Dir::Up } else { Dir::Down });
    let powered = level.has_neighbor_signal(pos) || level.has_neighbor_signal(other);
    if powered != level.flag(state, "powered") {
        if level.flag(state, "open") != powered {
            level.game_event(open_event(powered), pos);
        }
        let state = level.with_bool(state, "open", powered);
        level.set_block(pos, level.with_bool(state, "powered", powered), flags::CLIENTS);
    }
}

pub fn door_use(level: &mut Level, pos: Pos, state: StateId) {
    if opens_by_hand(level, state) {
        let open = !level.flag(state, "open");
        level.set_block(pos, level.with_bool(state, "open", open), flags::CLIENTS | flags::IMMEDIATE);
        level.game_event(open_event(open), pos);
    }
}

/// `DoorBlock.updateShape`: each half mirrors the other, and the lower half
/// needs a sturdy top below it.
pub fn door_update_shape(
    level: &Level,
    pos: Pos,
    state: StateId,
    dir: Dir,
    neighbor: Pos,
    neighbor_state: StateId,
) -> StateId {
    let lower = is_lower(level, state);
    let toward_other = if lower { Dir::Up } else { Dir::Down };
    if dir == toward_other {
        if level.is(neighbor_state, level.block(state)) && is_lower(level, neighbor_state) != lower {
            return level.with(neighbor_state, "half", level.prop(state, "half"));
        }
        return level.air;
    }
    if lower && dir == Dir::Down && !super::is_face_sturdy(level, neighbor, neighbor_state, Dir::Up) {
        return level.air;
    }
    let _ = pos;
    state
}
