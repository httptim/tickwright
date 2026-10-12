//! `RedStoneWireBlock` with both of its evaluators.
//!
//! `DefaultRedstoneWireEvaluator` recomputes power one wire at a time. A wire
//! whose power changes writes itself with flag 2 and then updates the
//! neighbors of itself and its six neighbors, in the iteration order of a
//! `HashSet<BlockPos>`. That order depends on the absolute position, which is
//! why dust is locational.
//!
//! `ExperimentalRedstoneWireEvaluator`, used when the `redstone_experiments`
//! feature is on, settles the whole network at once: it turns wires off, then
//! back on, breadth first in the direction the update travels, writes every
//! changed wire, and only then sends updates, each wire to the blocks it
//! points into, in an order given by its [`Orientation`].

use std::collections::{HashMap, VecDeque};

use crate::level::{Level, flags};
use crate::orientation::Orientation;
use crate::pos::{Dir, Pos, java_hash_set_order};
use crate::registry::{BlockId, StateId};

use super::Kind;

/// `Direction.Plane.HORIZONTAL`: clockwise from north.
const HORIZONTAL: [Dir; 4] = Dir::HORIZONTAL;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum RedstoneSide {
    None,
    Side,
    Up,
}

impl RedstoneSide {
    fn connected(self) -> bool {
        self != RedstoneSide::None
    }

    fn name(self) -> &'static str {
        match self {
            RedstoneSide::None => "none",
            RedstoneSide::Side => "side",
            RedstoneSide::Up => "up",
        }
    }

    fn parse(s: &str) -> RedstoneSide {
        match s {
            "side" => RedstoneSide::Side,
            "up" => RedstoneSide::Up,
            _ => RedstoneSide::None,
        }
    }
}

fn side(level: &Level, state: StateId, dir: Dir) -> RedstoneSide {
    RedstoneSide::parse(level.prop(state, dir.name()))
}

fn with_side(level: &Level, state: StateId, dir: Dir, s: RedstoneSide) -> StateId {
    level.with(state, dir.name(), s.name())
}

fn is_wire(level: &Level, state: StateId) -> bool {
    level.kind(state) == Kind::RedstoneWire
}

fn is_dot(level: &Level, state: StateId) -> bool {
    HORIZONTAL.iter().all(|&d| !side(level, state, d).connected())
}

fn is_cross(level: &Level, state: StateId) -> bool {
    HORIZONTAL.iter().all(|&d| side(level, state, d).connected())
}

/// `shouldConnectTo(state, direction)`.
fn should_connect_to(level: &Level, state: StateId, dir: Option<Dir>) -> bool {
    match level.kind(state) {
        Kind::RedstoneWire => true,
        Kind::Repeater => {
            let facing = level.dir(state, "facing");
            dir.is_some_and(|d| facing == d || facing.opposite() == d)
        }
        // Only toward its back, where its pulse comes out.
        Kind::Observer => dir == Some(level.dir(state, "facing")),
        _ => super::is_signal_source(level, state) && dir.is_some(),
    }
}

/// `canSurviveOn`: a sturdy top, or a hopper.
fn can_survive_on(level: &Level, pos: Pos, state: StateId) -> bool {
    super::is_face_sturdy(level, pos, state, Dir::Up) || level.kind(state) == Kind::Hopper
}

/// `getConnectingSide(level, pos, dir, canConnectUp)`.
fn connecting_side(level: &Level, pos: Pos, dir: Dir, can_connect_up: bool) -> RedstoneSide {
    let n = pos.offset(dir);
    let ns = level.get(n);
    if can_connect_up && can_survive_on(level, n, ns) && should_connect_to(level, level.get(n.above()), None) {
        return if super::is_face_sturdy(level, n, ns, dir.opposite()) { RedstoneSide::Up } else { RedstoneSide::Side };
    }
    if !should_connect_to(level, ns, Some(dir))
        && (super::is_conductor(level, n, ns) || !should_connect_to(level, level.get(n.below()), None))
    {
        RedstoneSide::None
    } else {
        RedstoneSide::Side
    }
}

fn can_connect_up(level: &Level, pos: Pos) -> bool {
    let above = pos.above();
    !super::is_conductor(level, above, level.get(above))
}

/// `getMissingConnections`.
fn missing_connections(level: &Level, mut state: StateId, pos: Pos) -> StateId {
    let up = can_connect_up(level, pos);
    for dir in HORIZONTAL {
        if !side(level, state, dir).connected() {
            state = with_side(level, state, dir, connecting_side(level, pos, dir, up));
        }
    }
    state
}

/// `getConnectionState`.
pub(super) fn connection_state(level: &Level, state: StateId, pos: Pos) -> StateId {
    let was_dot = is_dot(level, state);
    let base = level.default_state(level.block(state));
    let mut s = missing_connections(level, level.with_int(base, "power", level.int(state, "power")), pos);
    if was_dot && is_dot(level, s) {
        return s;
    }
    let c = |s: StateId, d: Dir| side(level, s, d).connected();
    let (north, south, east, west) = (c(s, Dir::North), c(s, Dir::South), c(s, Dir::East), c(s, Dir::West));
    let east_west_empty = !east && !west;
    let north_south_empty = !north && !south;
    if !west && north_south_empty {
        s = with_side(level, s, Dir::West, RedstoneSide::Side);
    }
    if !east && north_south_empty {
        s = with_side(level, s, Dir::East, RedstoneSide::Side);
    }
    if !north && east_west_empty {
        s = with_side(level, s, Dir::North, RedstoneSide::Side);
    }
    if !south && east_west_empty {
        s = with_side(level, s, Dir::South, RedstoneSide::Side);
    }
    s
}

pub fn update_shape(
    level: &Level,
    pos: Pos,
    state: StateId,
    dir: Dir,
    neighbor: Pos,
    neighbor_state: StateId,
) -> StateId {
    match dir {
        Dir::Down => {
            if can_survive_on(level, neighbor, neighbor_state) {
                state
            } else {
                level.air
            }
        }
        Dir::Up => connection_state(level, state, pos),
        _ => {
            let s = connecting_side(level, pos, dir, can_connect_up(level, pos));
            if s.connected() == side(level, state, dir).connected() && !is_cross(level, state) {
                with_side(level, state, dir, s)
            } else {
                let cross = HORIZONTAL.iter().fold(level.default_state(level.block(state)), |acc, &d| {
                    with_side(level, acc, d, RedstoneSide::Side)
                });
                let cross = level.with_int(cross, "power", level.int(state, "power"));
                connection_state(level, with_side(level, cross, dir, s), pos)
            }
        }
    }
}

pub fn update_indirect_neighbour_shapes(level: &mut Level, pos: Pos, state: StateId, f: i32, recursion: i32) {
    for dir in HORIZONTAL {
        if side(level, state, dir) == RedstoneSide::None || is_wire(level, level.get(pos.offset(dir))) {
            continue;
        }
        for step in [Dir::Down, Dir::Up] {
            let p = pos.offset(dir).offset(step);
            if is_wire(level, level.get(p)) {
                let n = p.offset(dir.opposite());
                let ns = level.get(n);
                level.neighbor_shape_changed(dir.opposite(), p, n, ns, f, recursion);
            }
        }
    }
}

pub fn get_signal(level: &Level, pos: Pos, state: StateId, dir: Dir) -> i32 {
    if !level.wire_signals.get() || dir == Dir::Down {
        return 0;
    }
    let power = level.int(state, "power");
    if power == 0 {
        return 0;
    }
    if dir != Dir::Up && !side(level, connection_state(level, state, pos), dir.opposite()).connected() {
        return 0;
    }
    power
}

pub fn get_direct_signal(level: &Level, pos: Pos, state: StateId, dir: Dir) -> i32 {
    if level.wire_signals.get() { get_signal(level, pos, state, dir) } else { 0 }
}

fn wire_signal(level: &Level, pos: Pos) -> i32 {
    let s = level.get(pos);
    if is_wire(level, s) { level.int(s, "power") } else { 0 }
}

/// `getIncomingWireSignal`.
fn incoming_wire_signal(level: &Level, pos: Pos) -> i32 {
    let mut best = 0;
    let above = pos.above();
    let covered = super::is_conductor(level, above, level.get(above));
    for dir in HORIZONTAL {
        let n = pos.offset(dir);
        best = best.max(wire_signal(level, n));
        let conductor = super::is_conductor(level, n, level.get(n));
        if conductor && !covered {
            best = best.max(wire_signal(level, n.above()));
        } else if !conductor {
            best = best.max(wire_signal(level, n.below()));
        }
    }
    (best - 1).max(0)
}

/// `calculateTargetStrength`.
fn target_strength(level: &Level, pos: Pos) -> i32 {
    level.wire_signals.set(false);
    let block = level.best_neighbor_signal(pos);
    level.wire_signals.set(true);
    if block == 15 { 15 } else { block.max(incoming_wire_signal(level, pos)) }
}

/// `RedStoneWireBlock.updatePowerStrength`: the evaluator the level's
/// features select.
fn update_power_strength(level: &mut Level, pos: Pos, state: StateId, orientation: Option<Orientation>, added: bool) {
    if level.experimental_redstone {
        Experimental::default().update_power_strength(level, pos, orientation, added);
    } else {
        update_power_strength_default(level, pos, state);
    }
}

/// `DefaultRedstoneWireEvaluator.updatePowerStrength`.
fn update_power_strength_default(level: &mut Level, pos: Pos, state: StateId) {
    let target = target_strength(level, pos);
    if level.int(state, "power") == target {
        return;
    }
    if level.get(pos) == state {
        level.set_block(pos, level.with_int(state, "power", target), flags::CLIENTS);
    }
    let mut set = vec![pos];
    set.extend(Dir::ALL.iter().map(|&d| pos.offset(d)));
    let block = level.block(state);
    for p in java_hash_set_order(&set) {
        level.update_neighbors_at(p, block);
    }
}

/// `checkCornerChangeAt`.
fn check_corner_change_at(level: &mut Level, pos: Pos) {
    let state = level.get(pos);
    if is_wire(level, state) {
        let block = level.block(state);
        level.update_neighbors_at(pos, block);
        for dir in Dir::ALL {
            level.update_neighbors_at(pos.offset(dir), block);
        }
    }
}

/// `updateNeighborsOfNeighboringWires`.
fn update_neighbors_of_neighboring_wires(level: &mut Level, pos: Pos) {
    for dir in HORIZONTAL {
        check_corner_change_at(level, pos.offset(dir));
    }
    for dir in HORIZONTAL {
        let n = pos.offset(dir);
        if super::is_conductor(level, n, level.get(n)) {
            check_corner_change_at(level, n.above());
        } else {
            check_corner_change_at(level, n.below());
        }
    }
}

pub fn on_place(level: &mut Level, pos: Pos, state: StateId, old: StateId) {
    if level.block(old) == level.block(state) {
        return;
    }
    update_power_strength(level, pos, state, None, true);
    let block = level.block(state);
    for dir in [Dir::Up, Dir::Down] {
        level.update_neighbors_at(pos.offset(dir), block);
    }
    update_neighbors_of_neighboring_wires(level, pos);
}

pub fn affect_neighbors_after_removal(level: &mut Level, pos: Pos, state: StateId, moved_by_piston: bool) {
    if moved_by_piston {
        return;
    }
    let block = level.block(state);
    for dir in Dir::ALL {
        level.update_neighbors_at(pos.offset(dir), block);
    }
    update_power_strength(level, pos, state, None, false);
    update_neighbors_of_neighboring_wires(level, pos);
}

pub fn neighbor_changed(
    level: &mut Level,
    pos: Pos,
    state: StateId,
    source: BlockId,
    orientation: Option<Orientation>,
) {
    // The experimental evaluator settles a whole network at once, so dust
    // ignores updates from other dust.
    if level.experimental_redstone && source == level.block(state) {
        return;
    }
    if can_survive_on(level, pos.below(), level.get(pos.below())) {
        update_power_strength(level, pos, state, orientation, false);
    } else {
        level.remove_block(pos);
    }
}

/// `ExperimentalRedstoneWireEvaluator`, one per update.
#[derive(Default)]
struct Experimental {
    to_turn_off: VecDeque<Pos>,
    to_turn_on: VecDeque<Pos>,
    /// `updatedWires`: a linked map, so iteration follows insertion order.
    order: Vec<Pos>,
    updated: HashMap<Pos, (Orientation, i32)>,
}

impl Experimental {
    fn update_power_strength(&mut self, level: &mut Level, pos: Pos, orientation: Option<Orientation>, added: bool) {
        // `getInitialOrientation`: whatever the update brought, upright and
        // leaning left.
        let initial = match orientation {
            Some(o) => o,
            None => level.random_orientation(),
        }
        .with_up(Dir::Up)
        .with_bias(crate::orientation::SideBias::Left);
        self.calculate_current_changes(level, pos, initial);
        let mut kept = Vec::new();
        for (i, &p) in self.order.iter().enumerate() {
            let power = self.updated[&p].1;
            let s = level.get(p);
            if is_wire(level, s) && level.int(s, "power") != power {
                let mut f = flags::CLIENTS;
                if !added || i != 0 {
                    f |= flags::SKIP_SHAPE_UPDATE_ON_WIRE;
                }
                level.set_block(p, level.with_int(s, "power", power), f);
                kept.push(p);
            }
        }
        self.cause_neighbor_updates(level, &kept);
    }

    fn cause_neighbor_updates(&self, level: &mut Level, wires: &[Pos]) {
        for &pos in wires {
            let o = self.updated[&pos].0;
            let state = level.get(pos);
            let block = level.block(state);
            for d in o.directions() {
                if !is_connected(level, state, d) {
                    continue;
                }
                let n = pos.offset(d);
                let ns = level.get(n);
                let o2 = o.with_front_preserve_up(d);
                level.neighbor_changed_state_oriented(n, ns, block, false, Some(o2));
                if super::is_conductor(level, n, ns) {
                    for d2 in o2.directions() {
                        if d2 != d.opposite() {
                            level.neighbor_changed_oriented(n.offset(d2), block, Some(o2.with_front_preserve_up(d2)));
                        }
                    }
                }
            }
        }
    }

    fn calculate_current_changes(&mut self, level: &Level, pos: Pos, o: Orientation) {
        let s = level.get(pos);
        if is_wire(level, s) {
            self.set_power(pos, level.int(s, "power"), o);
            self.to_turn_off.push_back(pos);
        } else {
            self.propagate_change_to_neighbors(level, pos, 0, o, true);
        }
        while let Some(p) = self.to_turn_off.pop_front() {
            let (po, old) = self.updated[&p];
            let block_power = block_signal(level, p);
            let wire_power = self.incoming_wire_signal(level, p);
            let new = block_power.max(wire_power);
            let to_set = if new < old {
                if block_power > 0 && !self.to_turn_on.contains(&p) {
                    self.to_turn_on.push_back(p);
                }
                0
            } else {
                new
            };
            if to_set != old {
                self.set_power(p, to_set, po);
            }
            self.propagate_change_to_neighbors(level, p, to_set, po, old > new);
        }
        while let Some(p) = self.to_turn_on.pop_front() {
            let (po, old) = self.updated[&p];
            let new = block_signal(level, p).max(self.incoming_wire_signal(level, p));
            if new > old {
                self.set_power(p, new, po);
            }
            self.propagate_change_to_neighbors(level, p, new, po, false);
        }
    }

    /// `setPower`: a wire already in the map keeps its orientation.
    fn set_power(&mut self, pos: Pos, power: i32, o: Orientation) {
        match self.updated.get_mut(&pos) {
            Some(e) => e.1 = power,
            None => {
                self.updated.insert(pos, (o, power));
                self.order.push(pos);
            }
        }
    }

    fn propagate_change_to_neighbors(&mut self, level: &Level, pos: Pos, power: i32, o: Orientation, allow_off: bool) {
        for d in o.horizontal() {
            self.enqueue_neighbor_wire(level, pos.offset(d), power, o.with_front(d), allow_off);
        }
        for v in o.vertical() {
            let vp = pos.offset(v);
            let conductor = super::is_conductor(level, vp, level.get(vp));
            for d in o.horizontal() {
                let n = pos.offset(d);
                let reach = if v == Dir::Up { !conductor } else { !super::is_conductor(level, n, level.get(n)) };
                if reach {
                    self.enqueue_neighbor_wire(level, vp.offset(d), power, o.with_front(d), allow_off);
                }
            }
        }
    }

    fn enqueue_neighbor_wire(&mut self, level: &Level, pos: Pos, source: i32, o: Orientation, allow_off: bool) {
        let s = level.get(pos);
        if !is_wire(level, s) {
            return;
        }
        let power = self.wire_signal(level, pos);
        if power < source - 1 && !self.to_turn_on.contains(&pos) {
            self.to_turn_on.push_back(pos);
            self.set_power(pos, power, o);
        }
        if allow_off && power > source && !self.to_turn_off.contains(&pos) {
            self.to_turn_off.push_back(pos);
            self.set_power(pos, power, o);
        }
    }

    /// `getWireSignal`: the pending power if this update changed it.
    fn wire_signal(&self, level: &Level, pos: Pos) -> i32 {
        match self.updated.get(&pos) {
            Some(&(_, p)) => p,
            None => wire_signal(level, pos),
        }
    }

    fn incoming_wire_signal(&self, level: &Level, pos: Pos) -> i32 {
        let mut best = 0;
        let above = pos.above();
        let covered = super::is_conductor(level, above, level.get(above));
        for dir in HORIZONTAL {
            let n = pos.offset(dir);
            best = best.max(self.wire_signal(level, n));
            let conductor = super::is_conductor(level, n, level.get(n));
            if conductor && !covered {
                best = best.max(self.wire_signal(level, n.above()));
            } else if !conductor {
                best = best.max(self.wire_signal(level, n.below()));
            }
        }
        (best - 1).max(0)
    }
}

/// `getBlockSignal`: the strongest signal into `pos` from anything but dust.
fn block_signal(level: &Level, pos: Pos) -> i32 {
    level.wire_signals.set(false);
    let s = level.best_neighbor_signal(pos);
    level.wire_signals.set(true);
    s
}

/// `ExperimentalRedstoneWireEvaluator.isConnected`: down always, the sides
/// the wire points into.
fn is_connected(level: &Level, state: StateId, dir: Dir) -> bool {
    match dir {
        Dir::Down => true,
        Dir::Up => false,
        d => side(level, state, d).connected(),
    }
}
