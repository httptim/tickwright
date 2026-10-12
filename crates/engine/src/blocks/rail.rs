//! `PoweredRailBlock` (powered and activator rails) and `DetectorRailBlock`,
//! with the parts of `BaseRailBlock` they use.
//!
//! `RailState`, which turns a newly placed rail toward its neighbours and
//! turns them toward it, is here for straight rails. Plain rails are
//! simulated while they stay as placed; one that would bend is
//! unsupported.

use crate::level::{Level, flags};
use crate::pos::{Dir, Pos};
use crate::registry::StateId;

use super::Kind;

fn is_slope(shape: &str) -> bool {
    shape.starts_with("ascending_")
}

/// `BaseRailBlock.canSupportRigidBlock`, approximated by a sturdy face.
fn supports(level: &Level, pos: Pos) -> bool {
    super::is_face_sturdy(level, pos, level.get(pos), Dir::Up)
}

/// `shouldBeRemoved`: no support below, or none on the high side of a slope.
fn should_be_removed(level: &Level, pos: Pos, shape: &str) -> bool {
    if !supports(level, pos.below()) {
        return true;
    }
    match shape {
        "ascending_east" => !supports(level, pos.offset(Dir::East)),
        "ascending_west" => !supports(level, pos.offset(Dir::West)),
        "ascending_north" => !supports(level, pos.offset(Dir::North)),
        "ascending_south" => !supports(level, pos.offset(Dir::South)),
        _ => false,
    }
}

/// `findPoweredRailSignal`: follow the rail one way, through at most eight
/// powered rails, looking for one that has a signal of its own.
fn find_signal(level: &Level, pos: Pos, state: StateId, forward: bool, depth: i32) -> bool {
    if depth >= 8 {
        return false;
    }
    let (mut x, mut y, mut z) = (pos.x, pos.y, pos.z);
    let mut also_below = true;
    let mut shape = level.prop(state, "shape");
    match shape {
        "north_south" => z += if forward { 1 } else { -1 },
        "east_west" => x += if forward { -1 } else { 1 },
        "ascending_east" => {
            if forward {
                x -= 1;
            } else {
                x += 1;
                y += 1;
                also_below = false;
            }
            shape = "east_west";
        }
        "ascending_west" => {
            if forward {
                x -= 1;
                y += 1;
                also_below = false;
            } else {
                x += 1;
            }
            shape = "east_west";
        }
        "ascending_north" => {
            if forward {
                z += 1;
            } else {
                z -= 1;
                y += 1;
                also_below = false;
            }
            shape = "north_south";
        }
        "ascending_south" => {
            if forward {
                z += 1;
                y += 1;
                also_below = false;
            } else {
                z -= 1;
            }
            shape = "north_south";
        }
        _ => {}
    }
    let next = Pos::new(x, y, z);
    is_same_rail_with_power(level, state, next, forward, depth, shape)
        || (also_below && is_same_rail_with_power(level, state, next.below(), forward, depth, shape))
}

/// `isSameRailWithPower`: the same kind of rail, lined up, powered, and
/// either signalled itself or further along the chain.
fn is_same_rail_with_power(level: &Level, rail: StateId, pos: Pos, forward: bool, depth: i32, shape: &str) -> bool {
    let s = level.get(pos);
    if !level.is(s, level.block(rail)) {
        return false;
    }
    let other = level.prop(s, "shape");
    let crosses = match shape {
        "east_west" => matches!(other, "north_south" | "ascending_north" | "ascending_south"),
        "north_south" => matches!(other, "east_west" | "ascending_east" | "ascending_west"),
        _ => false,
    };
    if crosses || !level.flag(s, "powered") {
        return false;
    }
    level.has_neighbor_signal(pos) || find_signal(level, pos, s, forward, depth + 1)
}

/// `PoweredRailBlock.updateState`.
fn update_state(level: &mut Level, pos: Pos, state: StateId) {
    let powered = level.flag(state, "powered");
    let now = level.has_neighbor_signal(pos)
        || find_signal(level, pos, state, true, 0)
        || find_signal(level, pos, state, false, 0);
    if now != powered {
        let block = level.block(state);
        level.set_block(pos, level.with_bool(state, "powered", now), flags::ALL);
        level.update_neighbors_at(pos.below(), block);
        if is_slope(level.prop(state, "shape")) {
            level.update_neighbors_at(pos.above(), block);
        }
    }
}

/// `BaseRailBlock.neighborChanged`.
pub fn neighbor_changed(level: &mut Level, pos: Pos, state: StateId, moved_by_piston: bool) {
    if should_be_removed(level, pos, level.prop(state, "shape")) {
        if level.simulate_entities {
            level.drop_resources(pos, state);
        }
        let f = if moved_by_piston { flags::ALL | flags::MOVE_BY_PISTON } else { flags::ALL };
        level.set_block(pos, level.air, f);
    } else {
        update_state(level, pos, state);
    }
}

/// `BaseRailBlock.neighborChanged` for a detector rail: only the survival
/// check, as its `updateState` does nothing.
pub fn detector_neighbor_changed(level: &mut Level, pos: Pos, state: StateId, moved_by_piston: bool) {
    if should_be_removed(level, pos, level.prop(state, "shape")) {
        if level.simulate_entities {
            level.drop_resources(pos, state);
        }
        let f = if moved_by_piston { flags::ALL | flags::MOVE_BY_PISTON } else { flags::ALL };
        level.set_block(pos, level.air, f);
    }
}

/// `DetectorRailBlock.getSignal`.
pub fn detector_signal(level: &Level, state: StateId) -> i32 {
    if level.flag(state, "powered") { 15 } else { 0 }
}

/// `DetectorRailBlock.onPlace`: a fresh detector rail settles its shape
/// like any straight rail, then looks for a minecart on it.
pub fn detector_on_place(level: &mut Level, pos: Pos, state: StateId, old: StateId) {
    let block = level.block(state);
    if !level.is(old, block) {
        let placed = RailState::new(level, pos, state).place(level, level.has_neighbor_signal(pos), true);
        level.neighbor_changed_state(pos, placed, block, false);
        detector_check_pressed(level, pos, placed);
    } else if old != state {
        removal_updates(level, pos, old, block);
    }
}

/// `entityInside`: an unpowered detector rail checks as soon as something
/// is inside it.
pub fn detector_entity_inside(level: &mut Level, pos: Pos, state: StateId) {
    if !level.flag(state, "powered") {
        detector_check_pressed(level, pos, state);
    }
}

/// `tick`: a powered detector rail checks again every 20 ticks.
pub fn detector_tick(level: &mut Level, pos: Pos, state: StateId) {
    if level.flag(state, "powered") {
        detector_check_pressed(level, pos, state);
    }
}

/// `getSearchBB`: the middle of the block, where carts are looked for.
fn search_box(pos: Pos) -> crate::physics::Aabb {
    crate::physics::Aabb {
        min: [pos.x as f64 + 0.2, pos.y as f64, pos.z as f64 + 0.2],
        max: [pos.x as f64 + 0.8, pos.y as f64 + 0.8, pos.z as f64 + 0.8],
    }
}

/// `DetectorRailBlock.getAnalogOutputSignal`: while powered, how full the
/// first chest or hopper minecart on it is. The engine has no command
/// block minecarts.
pub fn detector_analog_output(level: &Level, pos: Pos, state: StateId) -> i32 {
    if !level.flag(state, "powered") {
        return 0;
    }
    let b = search_box(pos);
    level
        .entities
        .iter()
        .filter(|e| !e.removed && e.bounding_box().intersects(&b))
        .find_map(|e| match &e.data {
            crate::entity::Data::Minecart(m) => m.items.as_deref(),
            _ => None,
        })
        .map_or(0, super::container::signal_from_items)
}

/// `checkPressed`: powered while a minecart's box meets the middle of the
/// block.
fn detector_check_pressed(level: &mut Level, pos: Pos, state: StateId) {
    if !super::placement::state_can_survive(level, pos, state) {
        return;
    }
    let was = level.flag(state, "powered");
    let b = search_box(pos);
    let now = level
        .entities
        .iter()
        .any(|e| !e.removed && matches!(e.data, crate::entity::Data::Minecart(_)) && e.bounding_box().intersects(&b));
    let block = level.block(state);
    if now != was {
        let s = level.with_bool(state, "powered", now);
        level.set_block(pos, s, flags::ALL);
        for c in RailState::new(level, pos, s).connections {
            let cs = level.get(c);
            level.neighbor_changed_state(c, cs, level.block(cs), false);
        }
        level.update_neighbors_at(pos, block);
        level.update_neighbors_at(pos.below(), block);
    }
    if now {
        level.schedule_tick(pos, block, 20, 0);
    }
    level.update_neighbour_for_output_signal(pos, block);
}

/// `BaseRailBlock.onPlace`: a fresh rail runs `updateState`, which reshapes
/// it and its neighbours (`updateDir`, `RailState.place`) and then sends
/// itself a neighbor update carrying the new shape, so it checks its power
/// at once.
///
/// When the rail only changes state (power, or a shape `RailState` gives
/// it), it updates the neighbors above it (if it was sloped), its own and
/// those below it, in that order, before the write's own updates go out:
/// the same updates as removing it.
pub fn on_place(level: &mut Level, pos: Pos, state: StateId, old: StateId) {
    let block = level.block(state);
    if !level.is(old, block) {
        let placed = RailState::new(level, pos, state).place(level, level.has_neighbor_signal(pos), true);
        level.neighbor_changed_state(pos, placed, block, false);
    } else if old != state {
        removal_updates(level, pos, old, block);
    }
}

fn removal_updates(level: &mut Level, pos: Pos, state: StateId, block: crate::registry::BlockId) {
    if is_slope(level.prop(state, "shape")) {
        level.update_neighbors_at(pos.above(), block);
    }
    level.update_neighbors_at(pos, block);
    level.update_neighbors_at(pos.below(), block);
}

pub fn affect_neighbors_after_removal(level: &mut Level, pos: Pos, state: StateId, moved_by_piston: bool) {
    if !moved_by_piston {
        removal_updates(level, pos, state, level.block(state));
    }
}

/// `BaseRailBlock.isRail`.
fn is_rail(level: &Level, pos: Pos) -> bool {
    is_rail_state(level, level.get(pos))
}

/// `BaseRailBlock.isRail(state)`.
pub fn is_rail_state(level: &Level, state: StateId) -> bool {
    matches!(level.kind(state), Kind::PoweredRail | Kind::DetectorRail | Kind::Rail)
}

/// `RailBlock.updateState` after `BaseRailBlock.neighborChanged`'s
/// survival check: a signal source next to a rail with three rails to
/// connect to turns it, which the engine can't do yet.
pub fn plain_neighbor_changed(
    level: &mut Level,
    pos: Pos,
    state: StateId,
    source: crate::registry::BlockId,
    moved_by_piston: bool,
) {
    if should_be_removed(level, pos, level.prop(state, "shape")) {
        if level.simulate_entities {
            level.drop_resources(pos, state);
        }
        let f = if moved_by_piston { flags::ALL | flags::MOVE_BY_PISTON } else { flags::ALL };
        level.set_block(pos, level.air, f);
    } else if super::is_signal_source(level, level.default_state(source))
        && RailState::new(level, pos, state).count_potential_connections(level) == 3
    {
        level.unsupported("a plain rail turned by a signal");
    }
}

/// `BaseRailBlock.onPlace` for a plain rail: `updateDir`, and no neighbor
/// update to itself, since it isn't straight.
pub fn plain_on_place(level: &mut Level, pos: Pos, state: StateId, old: StateId) {
    if !level.is(old, level.block(state)) {
        RailState::new(level, pos, state).place(level, level.has_neighbor_signal(pos), true);
    }
}

/// `BaseRailBlock.affectNeighborsAfterRemoval`: a plain rail isn't
/// straight, so only a slope updates (above it).
pub fn plain_affect_neighbors_after_removal(level: &mut Level, pos: Pos, state: StateId, moved_by_piston: bool) {
    if !moved_by_piston && is_slope(level.prop(state, "shape")) {
        level.update_neighbors_at(pos.above(), level.block(state));
    }
}

/// `RailState`: a rail and the positions its shape connects to.
struct RailState {
    pos: Pos,
    state: StateId,
    connections: Vec<Pos>,
}

impl RailState {
    fn new(level: &Level, pos: Pos, state: StateId) -> RailState {
        let mut r = RailState { pos, state, connections: Vec::new() };
        r.update_connections(level.prop(state, "shape"));
        r
    }

    fn update_connections(&mut self, shape: &str) {
        let p = self.pos;
        let (n, s, w, e) = (p.offset(Dir::North), p.offset(Dir::South), p.offset(Dir::West), p.offset(Dir::East));
        self.connections = match shape {
            "north_south" => vec![n, s],
            "east_west" => vec![w, e],
            "ascending_east" => vec![w, e.above()],
            "ascending_west" => vec![w.above(), e],
            "ascending_north" => vec![n.above(), s],
            "ascending_south" => vec![n, s.above()],
            "south_east" => vec![e, s],
            "south_west" => vec![w, s],
            "north_west" => vec![w, n],
            _ => vec![e, n],
        };
    }

    /// `getRail`: a rail at `pos`, or just above or below it.
    fn get_rail(level: &Level, pos: Pos) -> Option<RailState> {
        [pos, pos.above(), pos.below()]
            .into_iter()
            .find(|&p| is_rail(level, p))
            .map(|p| RailState::new(level, p, level.get(p)))
    }

    fn has_connection(&self, p: Pos) -> bool {
        self.connections.iter().any(|c| c.x == p.x && c.z == p.z)
    }

    fn connects_to(&self, other: &RailState) -> bool {
        self.has_connection(other.pos)
    }

    fn can_connect_to(&self, other: &RailState) -> bool {
        self.connects_to(other) || self.connections.len() != 2
    }

    /// `removeSoftConnections`: drop connections to rails that do not
    /// connect back.
    fn remove_soft_connections(&mut self, level: &Level) {
        let me = RailState { pos: self.pos, state: self.state, connections: Vec::new() };
        self.connections.retain(|&c| RailState::get_rail(level, c).is_some_and(|r| r.connects_to(&me)));
    }

    /// `countPotentialConnections`.
    fn count_potential_connections(&self, level: &Level) -> usize {
        let p = self.pos;
        [Dir::North, Dir::South, Dir::West, Dir::East]
            .into_iter()
            .filter(|&d| RailState::get_rail(level, p.offset(d)).is_some())
            .count()
    }

    fn has_neighbor_rail(&self, level: &Level, pos: Pos) -> bool {
        match RailState::get_rail(level, pos) {
            Some(mut r) => {
                r.remove_soft_connections(level);
                r.can_connect_to(self)
            }
            None => false,
        }
    }

    /// `connectTo` for a straight rail.
    fn connect_to(&mut self, level: &mut Level, other: &RailState) {
        self.connections.push(other.pos);
        let p = self.pos;
        let (n, s, w, e) = (p.offset(Dir::North), p.offset(Dir::South), p.offset(Dir::West), p.offset(Dir::East));
        let mut shape = None;
        if self.has_connection(n) || self.has_connection(s) {
            shape = Some("north_south");
        }
        if self.has_connection(w) || self.has_connection(e) {
            shape = Some("east_west");
        }
        if level.kind(self.state) == Kind::Rail
            && (self.has_connection(n) || self.has_connection(s))
            && (self.has_connection(w) || self.has_connection(e))
        {
            level.unsupported("a plain rail bending toward a new rail");
            return;
        }
        let shape = ascend(level, shape, n, s, w, e).unwrap_or("north_south");
        self.state = level.with(self.state, "shape", shape);
        level.set_block(self.pos, self.state, flags::ALL);
    }

    /// `place` for a straight rail: its new shape, written, and its
    /// neighbours turned toward it. Returns the rail's new state.
    fn place(mut self, level: &mut Level, _powered: bool, always_place: bool) -> StateId {
        let p = self.pos;
        let (n, s, w, e) = (p.offset(Dir::North), p.offset(Dir::South), p.offset(Dir::West), p.offset(Dir::East));
        let (bn, bs, bw, be) = (
            self.has_neighbor_rail(level, n),
            self.has_neighbor_rail(level, s),
            self.has_neighbor_rail(level, w),
            self.has_neighbor_rail(level, e),
        );
        let (ns, ew) = (bn || bs, bw || be);
        if ns && ew && level.kind(self.state) == Kind::Rail {
            level.unsupported("a plain rail bending");
            return self.state;
        }
        let current = level.prop(self.state, "shape");
        let mut shape = None;
        if ns && !ew {
            shape = Some("north_south");
        }
        if ew && !ns {
            shape = Some("east_west");
        }
        if shape.is_none() {
            shape = if ns && ew {
                Some(current)
            } else if ns {
                Some("north_south")
            } else if ew {
                Some("east_west")
            } else {
                None
            };
        }
        let shape = ascend(level, shape, n, s, w, e).unwrap_or(current);
        self.update_connections(shape);
        self.state = level.with(self.state, "shape", shape);
        if always_place || level.get(p) != self.state {
            level.set_block(p, self.state, flags::ALL);
            for c in self.connections.clone() {
                if let Some(mut r) = RailState::get_rail(level, c) {
                    r.remove_soft_connections(level);
                    if r.can_connect_to(&self) {
                        r.connect_to(level, &self);
                    }
                }
            }
        }
        self.state
    }
}

/// A flat shape turned into a slope toward a rail one block up.
fn ascend(level: &Level, shape: Option<&'static str>, n: Pos, s: Pos, w: Pos, e: Pos) -> Option<&'static str> {
    match shape {
        Some("north_south") => Some(if is_rail(level, s.above()) {
            "ascending_south"
        } else if is_rail(level, n.above()) {
            "ascending_north"
        } else {
            "north_south"
        }),
        Some("east_west") => Some(if is_rail(level, w.above()) {
            "ascending_west"
        } else if is_rail(level, e.above()) {
            "ascending_east"
        } else {
            "east_west"
        }),
        other => other,
    }
}
