//! `RedstoneTorchBlock` and `RedstoneWallTorchBlock`.

use crate::level::{Level, flags};
use crate::pos::{Dir, Pos};
use crate::registry::StateId;

use super::Kind;

/// `RECENT_TOGGLE_TIMER`: how far back forced turn-offs are remembered.
const RECENT_TOGGLE_TIMER: i64 = 60;
/// `MAX_RECENT_TOGGLES`: turn-offs within the timer that burn a torch out.
const MAX_RECENT_TOGGLES: usize = 8;
/// `RESTART_DELAY`: how long a burnt-out torch waits before checking again.
const RESTART_DELAY: i32 = 160;

fn is_wall(level: &Level, state: StateId) -> bool {
    matches!(level.kind(state), Kind::RedstoneWallTorch | Kind::WallTorch)
}

/// `hasNeighborSignal`: power into the block the torch hangs on.
fn has_neighbor_signal(level: &Level, pos: Pos, state: StateId) -> bool {
    if is_wall(level, state) {
        let dir = level.dir(state, "facing").opposite();
        level.has_signal(pos.offset(dir), dir)
    } else {
        level.has_signal(pos.below(), Dir::Down)
    }
}

/// `notifyNeighbors`: update the neighbors of every block around the torch.
fn notify_neighbors(level: &mut Level, pos: Pos, state: StateId) {
    let block = level.block(state);
    for dir in Dir::ALL {
        level.update_neighbors_at(pos.offset(dir), block);
    }
}

pub fn on_place(level: &mut Level, pos: Pos, state: StateId) {
    notify_neighbors(level, pos, state);
}

pub fn affect_neighbors_after_removal(level: &mut Level, pos: Pos, state: StateId, moved_by_piston: bool) {
    if !moved_by_piston {
        notify_neighbors(level, pos, state);
    }
}

pub fn get_signal(level: &Level, _pos: Pos, state: StateId, dir: Dir) -> i32 {
    let off = if is_wall(level, state) { level.dir(state, "facing") } else { Dir::Up };
    if level.flag(state, "lit") && dir != off { 15 } else { 0 }
}

pub fn get_direct_signal(level: &Level, pos: Pos, state: StateId, dir: Dir) -> i32 {
    if dir == Dir::Down { get_signal(level, pos, state, dir) } else { 0 }
}

pub fn neighbor_changed(level: &mut Level, pos: Pos, state: StateId) {
    let block = level.block(state);
    if level.flag(state, "lit") == has_neighbor_signal(level, pos, state) && !level.will_tick_this_tick(pos, block) {
        level.schedule_tick(pos, block, 2, 0);
    }
}

/// `updateShape`: a torch whose support is gone pops off.
pub fn update_shape(level: &Level, pos: Pos, state: StateId, dir: Dir) -> StateId {
    let support = if is_wall(level, state) { level.dir(state, "facing").opposite() } else { Dir::Down };
    if dir == support
        && !super::is_face_sturdy(level, pos.offset(support), level.get(pos.offset(support)), support.opposite())
    {
        level.air
    } else {
        state
    }
}

pub fn tick(level: &mut Level, pos: Pos, state: StateId) {
    let powered = has_neighbor_signal(level, pos, state);
    let now = level.game_time;
    let toggles = &mut level.torch_toggles;
    let expired = toggles.iter().take_while(|(_, when)| now - when > RECENT_TOGGLE_TIMER).count();
    toggles.drain(..expired);
    if level.flag(state, "lit") {
        if powered {
            level.set_block(pos, level.with_bool(state, "lit", false), flags::ALL);
            if toggled_too_frequently(level, pos, true) {
                let block = level.block(level.get(pos));
                level.schedule_tick(pos, block, RESTART_DELAY, 0);
            }
        }
    } else if !powered && !toggled_too_frequently(level, pos, false) {
        level.set_block(pos, level.with_bool(state, "lit", true), flags::ALL);
    }
}

/// `isToggledTooFrequently`.
fn toggled_too_frequently(level: &mut Level, pos: Pos, add: bool) -> bool {
    if add {
        let now = level.game_time;
        level.torch_toggles.push((pos, now));
    }
    level.torch_toggles.iter().filter(|(p, _)| *p == pos).count() >= MAX_RECENT_TOGGLES
}
