//! `TripWireBlock` and the string-handling half of `TripWireHookBlock`.
//!
//! Without entities nothing walks through the string, but breaking armed
//! string still trips its hooks: the broken string counts as powered.

use crate::game_event::GameEvent;
use crate::level::{Level, flags};
use crate::pos::{Dir, Pos};
use crate::registry::StateId;

use super::Kind;

/// How far apart two hooks can be, plus one.
const MAX_LENGTH: i32 = 42;

/// `TripWireBlock.shouldConnectTo`.
fn should_connect_to(level: &Level, state: StateId, dir: Dir) -> bool {
    match level.kind(state) {
        Kind::TripwireHook => level.dir(state, "facing") == dir.opposite(),
        Kind::Tripwire => true,
        _ => false,
    }
}

/// `TripWireBlock.updateShape`.
pub fn update_shape(level: &Level, state: StateId, dir: Dir, neighbor_state: StateId) -> StateId {
    if dir.is_horizontal() {
        level.with_bool(state, dir.name(), should_connect_to(level, neighbor_state, dir))
    } else {
        state
    }
}

/// `TripWireBlock.onPlace`.
pub fn on_place(level: &mut Level, pos: Pos, state: StateId, old: StateId) {
    if level.block(old) != level.block(state) {
        update_source(level, pos, state);
    }
}

/// `TripWireBlock.affectNeighborsAfterRemoval`: broken string trips.
pub fn affect_neighbors_after_removal(level: &mut Level, pos: Pos, state: StateId, moved_by_piston: bool) {
    if !moved_by_piston {
        let powered = level.with_bool(state, "powered", true);
        update_source(level, pos, powered);
    }
}

/// `TripWireBlock.updateSource`: recalculate the hooks south and west of
/// the string, which also reach the hooks across from them.
fn update_source(level: &mut Level, pos: Pos, state: StateId) {
    for d in [Dir::South, Dir::West] {
        for i in 1..MAX_LENGTH {
            let p = pos.offset_by(d, i);
            let s = level.get(p);
            match level.kind(s) {
                Kind::TripwireHook => {
                    if level.dir(s, "facing") == d.opposite() {
                        calculate_state(level, p, s, false, true, i, Some(state));
                    }
                    break;
                }
                Kind::Tripwire => {}
                _ => break,
            }
        }
    }
}

/// `TripWireHookBlock.calculateState`.
pub fn calculate_state(
    level: &mut Level,
    pos: Pos,
    state: StateId,
    being_destroyed: bool,
    do_update: bool,
    string_pos: i32,
    string_state: Option<StateId>,
) {
    let direction = level.dir(state, "facing");
    let was_attached = level.flag(state, "attached");
    let was_powered = level.flag(state, "powered");
    let block = level.block(state);
    let mut can_connect = !being_destroyed;
    let mut any_powered = false;
    let mut length = 0;
    let mut strings: Vec<Option<StateId>> = vec![None; MAX_LENGTH as usize];
    for i in 1..MAX_LENGTH {
        let p = pos.offset_by(direction, i);
        let mut s = level.get(p);
        if level.kind(s) == Kind::TripwireHook {
            if level.dir(s, "facing") == direction.opposite() {
                length = i;
            }
            break;
        }
        if level.kind(s) != Kind::Tripwire && i != string_pos {
            can_connect = false;
        } else {
            if i == string_pos {
                s = string_state.unwrap_or(s);
            }
            let armed = !level.flag(s, "disarmed");
            any_powered |= armed && level.flag(s, "powered");
            strings[i as usize] = Some(s);
            if i == string_pos {
                level.schedule_tick(pos, block, 10, 0);
                can_connect &= armed;
            }
        }
    }
    can_connect &= length > 1;
    any_powered &= can_connect;
    let hook =
        level.with_bool(level.with_bool(level.default_state(block), "attached", can_connect), "powered", any_powered);
    if length > 0 {
        let other = pos.offset_by(direction, length);
        let opposite = direction.opposite();
        level.set_block(other, level.with_dir(hook, "facing", opposite), flags::ALL);
        notify_neighbors(level, block, other, opposite);
        emit_state(level, other, can_connect, any_powered, was_attached, was_powered);
    }
    emit_state(level, pos, can_connect, any_powered, was_attached, was_powered);
    if !being_destroyed {
        level.set_block(pos, level.with_dir(hook, "facing", direction), flags::ALL);
        if do_update {
            notify_neighbors(level, block, pos, direction);
        }
    }
    // A hook being destroyed leaves its string attached in 26.3.
    if !being_destroyed && was_attached != can_connect {
        for i in 1..length {
            let p = pos.offset_by(direction, i);
            if let Some(s) = strings[i as usize] {
                let cur = level.get(p);
                if matches!(level.kind(cur), Kind::Tripwire | Kind::TripwireHook) {
                    let s = level.with_bool(s, "attached", can_connect);
                    level.set_block(p, s, flags::ALL);
                }
            }
        }
    }
}

/// `TripWireHookBlock.emitState`: only its game event matters here.
fn emit_state(level: &mut Level, pos: Pos, attached: bool, powered: bool, was_attached: bool, was_powered: bool) {
    let event = if powered && !was_powered {
        GameEvent::BlockActivate
    } else if !powered && was_powered {
        GameEvent::BlockDeactivate
    } else if attached && !was_attached {
        GameEvent::BlockAttach
    } else if !attached && was_attached {
        GameEvent::BlockDetach
    } else {
        return;
    };
    level.game_event(event, pos);
}

/// `TripWireHookBlock.notifyNeighbors`.
fn notify_neighbors(level: &mut Level, block: crate::registry::BlockId, pos: Pos, dir: Dir) {
    let opposite = dir.opposite();
    let orientation = level.initial_orientation(Some(opposite), Some(Dir::Up));
    level.update_neighbors_at_oriented(pos, block, orientation);
    level.update_neighbors_at_oriented(pos.offset(opposite), block, orientation);
}

/// `TripWireHookBlock.tick`.
pub fn hook_tick(level: &mut Level, pos: Pos, state: StateId) {
    calculate_state(level, pos, state, false, true, -1, None);
}

/// `TripWireHookBlock.setPlacedBy`.
pub fn hook_placed_by(level: &mut Level, pos: Pos, state: StateId) {
    calculate_state(level, pos, state, false, false, -1, None);
}

/// `TripWireHookBlock.affectNeighborsAfterRemoval`.
pub fn hook_affect_neighbors_after_removal(level: &mut Level, pos: Pos, state: StateId, moved_by_piston: bool) {
    if moved_by_piston {
        return;
    }
    let attached = level.flag(state, "attached");
    let powered = level.flag(state, "powered");
    if attached || powered {
        calculate_state(level, pos, state, true, false, -1, None);
    }
    if powered {
        notify_neighbors(level, level.block(state), pos, level.dir(state, "facing"));
    }
}
