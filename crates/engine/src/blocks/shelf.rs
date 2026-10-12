//! `ShelfBlock`: three item slots, read by comparators as a bitmask, and
//! powered shelves side by side join into chains of up to three
//! (`SideChainPartBlock`).

use crate::game_event::GameEvent;
use crate::level::{Level, flags};
use crate::pos::{Dir, Pos};
use crate::registry::StateId;

use super::{BlockEntity, Kind};

/// `getMaxChainLength`.
const MAX_CHAIN: usize = 3;

/// The direction toward the shelf's left, seen from in front of it.
fn left(level: &Level, state: StateId) -> Dir {
    level.dir(state, "facing").clockwise()
}

/// Whether the shelf at `p` can be part of a chain with shelves facing
/// `facing`.
fn joins(level: &Level, p: Pos, facing: Dir) -> bool {
    let s = level.get(p);
    level.kind(s) == Kind::Shelf && level.flag(s, "powered") && level.dir(s, "facing") == facing
}

/// `neighborChanged`.
pub fn neighbor_changed(level: &mut Level, pos: Pos, state: StateId) {
    let powered = level.has_neighbor_signal(pos);
    if level.flag(state, "powered") == powered {
        return;
    }
    let mut new = level.with_bool(state, "powered", powered);
    if !powered {
        new = level.with(new, "side_chain", "unconnected");
    }
    level.set_block(pos, new, flags::ALL);
    level.game_event(if powered { GameEvent::BlockActivate } else { GameEvent::BlockDeactivate }, pos);
}

/// `onPlace`: chains form and break as the shelf's power changes, before
/// the shelf's own neighbor updates go out.
pub fn on_place(level: &mut Level, pos: Pos, state: StateId, old: StateId) {
    if level.kind(old) != Kind::Shelf {
        return;
    }
    let (was, is) = (level.flag(old, "powered"), level.flag(state, "powered"));
    if is && !was {
        connect(level, pos, state);
    } else if was && !is {
        disconnect(level, pos, old);
    }
}

/// Join the newly powered shelf with the powered shelves beside it, if the
/// whole row fits in one chain.
fn connect(level: &mut Level, pos: Pos, state: StateId) {
    let facing = level.dir(state, "facing");
    let l = left(level, state);
    let mut row = vec![pos];
    let mut p = pos.offset(l);
    while joins(level, p, facing) && row.len() <= MAX_CHAIN {
        row.insert(0, p);
        p = p.offset(l);
    }
    let mut p = pos.offset(l.opposite());
    while joins(level, p, facing) && row.len() <= MAX_CHAIN {
        row.push(p);
        p = p.offset(l.opposite());
    }
    if row.len() < 2 || row.len() > MAX_CHAIN {
        return;
    }
    // The others from left to right, then the shelf that was powered.
    let n = row.len();
    let part = |i: usize| match i {
        0 => "left",
        _ if i == n - 1 => "right",
        _ => "center",
    };
    let me = row.iter().position(|&p| p == pos).unwrap();
    for i in (0..n).filter(|&i| i != me).chain([me]) {
        let s = level.get(row[i]);
        level.set_block(row[i], level.with(s, "side_chain", part(i)), flags::ALL);
    }
}

/// Break up the chain the unpowered shelf was in.
fn disconnect(level: &mut Level, pos: Pos, old: StateId) {
    let facing = level.dir(old, "facing");
    let l = left(level, old);
    let part = level.prop(old, "side_chain").to_string();
    // Toward the left while the chain goes on, then toward the right.
    for (dir, goes_on) in [(l, ["right", "center"]), (l.opposite(), ["left", "center"])] {
        if part == "unconnected" || !goes_on.contains(&part.as_str()) {
            continue;
        }
        let mut p = pos.offset(dir);
        loop {
            let s = level.get(p);
            if level.kind(s) != Kind::Shelf || level.dir(s, "facing") != facing {
                break;
            }
            let sp = level.prop(s, "side_chain").to_string();
            if sp == "unconnected" {
                break;
            }
            level.set_block(p, level.with(s, "side_chain", "unconnected"), flags::ALL);
            if sp != "center" {
                break;
            }
            p = p.offset(dir);
        }
    }
}

/// `getAnalogOutputSignal`: one bit per filled slot.
pub fn analog_output(level: &Level, pos: Pos) -> i32 {
    match level.block_entities.get(&pos) {
        Some(BlockEntity::Shelf(items)) => {
            items.iter().enumerate().filter(|(_, s)| !s.is_empty()).map(|(i, _)| 1 << i).sum()
        }
        _ => 0,
    }
}
