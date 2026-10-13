//! A player placing a block: `BlockItem.place` and each block's
//! `getStateForPlacement`, `canSurvive` and `setPlacedBy`.
//!
//! The player's look is reduced to what placement reads from it: the
//! horizontal direction faced (`getHorizontalDirection`) and the six
//! directions ordered by how directly the player looks along them
//! (`Direction.orderedByNearest`), computed with the game's sine table so
//! ties break the same way.

use anyhow::{Result, bail};

use crate::level::{Level, flags};
use crate::pos::{Dir, Pos};
use crate::registry::{BlockId, StateId};

use super::{Kind, is_face_sturdy};

/// What a click carries: the block and face clicked, the exact point, the
/// player's view and whether they sneak.
#[derive(Clone, Copy, Debug)]
pub struct Click {
    pub clicked: Pos,
    pub face: Dir,
    /// The click point, in sixteenths of a block from `clicked`'s least corner.
    pub hit: [i32; 3],
    /// The horizontal direction the player faces.
    pub look: Dir,
    /// Degrees below the horizon; negative looks up.
    pub pitch: i32,
    pub sneak: bool,
}

impl Click {
    /// The yaw a player facing `look` has; the harness uses the same.
    fn yaw(&self) -> f32 {
        match self.look {
            Dir::South => 0.0,
            Dir::West => 90.0,
            Dir::North => 180.0,
            _ => 270.0,
        }
    }

    /// The click point's height above the bottom of `pos`.
    fn hit_y_above(&self, pos: Pos) -> f64 {
        self.clicked.y as f64 + self.hit[1] as f64 / 16.0 - pos.y as f64
    }

    fn hit_x_z(&self, pos: Pos) -> (f64, f64) {
        (
            self.clicked.x as f64 + self.hit[0] as f64 / 16.0 - pos.x as f64,
            self.clicked.z as f64 + self.hit[2] as f64 / 16.0 - pos.z as f64,
        )
    }
}

/// `Mth.sin`, from the game's 65536-entry table.
fn mth_sin(v: f64) -> f32 {
    table(((v * 10430.378350470453) as i64 & 65535) as usize)
}

fn mth_cos(v: f64) -> f32 {
    table(((v * 10430.378350470453 + 16384.0) as i64 & 65535) as usize)
}

fn table(i: usize) -> f32 {
    (i as f64 * std::f64::consts::PI * 2.0 / 65536.0).sin() as f32
}

/// `Direction.orderedByNearest(entity)`.
fn ordered_by_nearest(yaw: f32, pitch: f32) -> [Dir; 6] {
    let rad = (std::f64::consts::PI / 180.0) as f32;
    let f = pitch * rad;
    let g = -yaw * rad;
    let (h, i, j, k) = (mth_sin(f as f64), mth_cos(f as f64), mth_sin(g as f64), mth_cos(g as f64));
    let (east, up, south) = (j > 0.0, h < 0.0, k > 0.0);
    let l = if east { j } else { -j };
    let m = if up { -h } else { h };
    let n = if south { k } else { -k };
    let o = l * i;
    let p = n * i;
    let d1 = if east { Dir::East } else { Dir::West };
    let d2 = if up { Dir::Up } else { Dir::Down };
    let d3 = if south { Dir::South } else { Dir::North };
    let make = |a: Dir, b: Dir, c: Dir| [a, b, c, c.opposite(), b.opposite(), a.opposite()];
    if l > n {
        if m > o {
            make(d2, d1, d3)
        } else if p > m {
            make(d1, d3, d2)
        } else {
            make(d1, d2, d3)
        }
    } else if m > p {
        make(d2, d3, d1)
    } else if o > m {
        make(d3, d1, d2)
    } else {
        make(d3, d2, d1)
    }
}

/// `BlockPlaceContext`.
struct Context {
    click: Click,
    pos: Pos,
    replace_clicked: bool,
    block: BlockId,
}

impl Context {
    fn horizontal(&self) -> Dir {
        // `Direction.fromYRot`: floor(yaw / 90 + 0.5) & 3, south first.
        let i = ((self.click.yaw() as f64 / 90.0 + 0.5).floor() as i64 & 3) as usize;
        [Dir::South, Dir::West, Dir::North, Dir::East][i]
    }

    fn nearest_looking(&self) -> Dir {
        ordered_by_nearest(self.click.yaw(), self.click.pitch as f32)[0]
    }

    /// `getNearestLookingDirections`: the clicked face's opposite first,
    /// unless the click replaces the clicked block.
    fn nearest_looking_all(&self) -> [Dir; 6] {
        let mut dirs = ordered_by_nearest(self.click.yaw(), self.click.pitch as f32);
        if !self.replace_clicked {
            let want = self.click.face.opposite();
            if let Some(i) = dirs.iter().position(|&d| d == want) {
                dirs.copy_within(0..i, 1);
                dirs[0] = want;
            }
        }
        dirs
    }
}

/// The block an item places, if it is a block item the engine simulates.
fn block_for_item(level: &Level, item: &str) -> Option<BlockId> {
    let item = item.strip_prefix("minecraft:").unwrap_or(item);
    let name = match item {
        "redstone" => "redstone_wire",
        other => other,
    };
    level.reg.block_id(&format!("minecraft:{name}"))
}

/// `BlockState.canBeReplaced(BlockPlaceContext)` for the state at `pos`.
fn can_be_replaced(level: &Level, state: StateId, click: &Click, pos: Pos, block: BlockId, replacing: bool) -> bool {
    match level.kind(state) {
        Kind::Air | Kind::Water => true,
        Kind::Slab => {
            let ty = level.prop(state, "type");
            if ty == "double" || level.block(state) != block {
                return false;
            }
            if replacing {
                let upper = click.hit_y_above(pos) > 0.5;
                let d = click.face;
                if ty == "bottom" {
                    d == Dir::Up || upper && d.is_horizontal()
                } else {
                    d == Dir::Down || !upper && d.is_horizontal()
                }
            } else {
                true
            }
        }
        _ => false,
    }
}

fn sturdy(level: &Level, pos: Pos, dir: Dir) -> bool {
    is_face_sturdy(level, pos, level.get(pos), dir)
}

/// `isCollisionShapeFullBlock`, for the door hinge rule.
fn full_block(level: &Level, pos: Pos) -> bool {
    let s = level.get(pos);
    match level.kind(s) {
        Kind::Slab => level.prop(s, "type") == "double",
        Kind::Piston | Kind::StickyPiston => !level.flag(s, "extended"),
        Kind::Honey | Kind::PowderSnow | Kind::Trapdoor | Kind::Door | Kind::Chest | Kind::Hopper => false,
        _ => is_face_sturdy(level, pos, s, Dir::Up) && is_face_sturdy(level, pos, s, Dir::Down),
    }
}

/// `canSurvive` for a state about to be placed at `pos`.
pub fn state_can_survive(level: &Level, pos: Pos, state: StateId) -> bool {
    can_survive(level, pos, state)
}

fn can_survive(level: &Level, pos: Pos, state: StateId) -> bool {
    let below = pos.below();
    match level.kind(state) {
        Kind::RedstoneWire => {
            let b = level.get(below);
            is_face_sturdy(level, below, b, Dir::Up) || level.kind(b) == Kind::Hopper
        }
        Kind::Repeater
        | Kind::Comparator
        | Kind::Cake
        | Kind::PoweredRail
        | Kind::DetectorRail
        | Kind::Rail
        | Kind::RedstoneTorch
        | Kind::Torch
        | Kind::DeadCoral => sturdy(level, below, Dir::Up),
        Kind::Carpet => !level.is_air(level.get(below)),
        Kind::Door => level.prop(state, "half") != "lower" || sturdy(level, below, Dir::Up),
        Kind::RedstoneWallTorch | Kind::WallTorch | Kind::TripwireHook | Kind::DeadCoralWallFan => {
            let facing = level.dir(state, "facing");
            sturdy(level, pos.offset(facing.opposite()), facing)
        }
        Kind::Lever | Kind::Button => {
            let d = super::connected_direction(level, state);
            sturdy(level, pos.offset(d.opposite()), d)
        }
        Kind::Bell => {
            let d = match level.prop(state, "attachment") {
                "floor" => Dir::Up,
                "ceiling" => Dir::Down,
                _ => level.dir(state, "facing"),
            };
            sturdy(level, pos.offset(d.opposite()), d)
        }
        _ => true,
    }
}

fn waterlogged(level: &Level, pos: Pos, state: StateId) -> StateId {
    if level.reg.value(state, "waterlogged").is_none() {
        return state;
    }
    let here = level.get(pos);
    let water = level.kind(here) == Kind::Water && level.int(here, "level") == 0;
    level.with_bool(state, "waterlogged", water)
}

/// `getStateForPlacement`, or `None` when the block cannot go here.
fn state_for_placement(level: &Level, ctx: &Context) -> Option<StateId> {
    let pos = ctx.pos;
    let base = waterlogged(level, pos, level.default_state(ctx.block));
    let survives = |s: StateId| can_survive(level, pos, s).then_some(s);
    let s = match level.kind(base) {
        Kind::RedstoneWire => {
            let mut cross = base;
            for d in Dir::HORIZONTAL {
                cross = level.with(cross, d.name(), "side");
            }
            super::wire::connection_state(level, cross, pos)
        }
        Kind::Repeater => {
            let s = level.with_dir(base, "facing", ctx.horizontal().opposite());
            level.with_bool(s, "locked", super::repeater::is_locked(level, pos, s))
        }
        Kind::Comparator => level.with_dir(base, "facing", ctx.horizontal().opposite()),
        Kind::RedstoneTorch => return torch_state(level, ctx, base),
        Kind::Lever | Kind::Button => {
            for d in ctx.nearest_looking_all() {
                let s = if d.axis() == 1 {
                    let s = level.with(base, "face", if d == Dir::Up { "ceiling" } else { "floor" });
                    level.with_dir(s, "facing", ctx.horizontal())
                } else {
                    level.with_dir(level.with(base, "face", "wall"), "facing", d.opposite())
                };
                if can_survive(level, pos, s) {
                    return Some(s);
                }
            }
            return None;
        }
        Kind::Piston | Kind::StickyPiston | Kind::Dropper | Kind::Dispenser | Kind::Barrel => {
            level.with_dir(base, "facing", ctx.nearest_looking().opposite())
        }
        Kind::Observer => level.with_dir(base, "facing", ctx.nearest_looking()),
        Kind::ShulkerBox => level.with_dir(base, "facing", ctx.click.face),
        Kind::Hopper => {
            let d = ctx.click.face.opposite();
            let s = level.with_dir(base, "facing", if d.axis() == 1 { Dir::Down } else { d });
            level.with_bool(s, "enabled", true)
        }
        Kind::Chest => chest_state(level, ctx, base),
        Kind::Door => {
            if !can_be_replaced(level, level.get(pos.above()), &ctx.click, pos.above(), ctx.block, false) {
                return None;
            }
            let powered = level.has_neighbor_signal(pos) || level.has_neighbor_signal(pos.above());
            let s = level.with_dir(base, "facing", ctx.horizontal());
            let s = level.with(s, "hinge", door_hinge(level, ctx));
            let s = level.with_bool(level.with_bool(s, "powered", powered), "open", powered);
            level.with(s, "half", "lower")
        }
        Kind::Trapdoor => {
            let face = ctx.click.face;
            let s = if !ctx.replace_clicked && face.is_horizontal() {
                let top = ctx.click.hit_y_above(pos) > 0.5;
                level.with(level.with_dir(base, "facing", face), "half", if top { "top" } else { "bottom" })
            } else {
                let s = level.with_dir(base, "facing", ctx.horizontal().opposite());
                level.with(s, "half", if face == Dir::Up { "bottom" } else { "top" })
            };
            if level.has_neighbor_signal(pos) {
                level.with_bool(level.with_bool(s, "open", true), "powered", true)
            } else {
                s
            }
        }
        Kind::Slab => {
            let here = level.get(pos);
            if level.block(here) == ctx.block {
                return Some(level.with_bool(level.with(here, "type", "double"), "waterlogged", false));
            }
            let s = level.with(base, "type", "bottom");
            let face = ctx.click.face;
            if face != Dir::Down && (face == Dir::Up || ctx.click.hit_y_above(pos) <= 0.5) {
                s
            } else {
                level.with(s, "type", "top")
            }
        }
        Kind::PoweredRail | Kind::DetectorRail => {
            let h = ctx.horizontal();
            level.with(base, "shape", if h.axis() == 0 { "east_west" } else { "north_south" })
        }
        Kind::RedstoneLamp => level.with_bool(base, "lit", level.has_neighbor_signal(pos)),
        Kind::NoteBlock => super::note::with_instrument(level, pos, base),
        Kind::Log => level.with(base, "axis", ["x", "y", "z"][ctx.click.face.axis() as usize]),
        Kind::Leaves => {
            let s = level.with_bool(base, "persistent", true);
            let mut d = 7;
            for dir in Dir::ALL {
                d = d.min(super::plain::distance_at(level, level.get(pos.offset(dir))) + 1);
                if d == 1 {
                    break;
                }
            }
            level.with_int(s, "distance", d)
        }
        Kind::TripwireHook => {
            let s = level.with_bool(level.with_bool(base, "powered", false), "attached", false);
            for d in ctx.nearest_looking_all() {
                if d.is_horizontal() {
                    let s = level.with_dir(s, "facing", d.opposite());
                    if can_survive(level, pos, s) {
                        return Some(s);
                    }
                }
            }
            return None;
        }
        Kind::Bell => return bell_state(level, ctx, base),
        Kind::Water | Kind::PistonHead | Kind::MovingPiston | Kind::Other => return None,
        _ => base,
    };
    survives(s)
}

/// `StandingAndWallBlockItem.getPlacementState` for redstone torches.
fn torch_state(level: &Level, ctx: &Context, standing: StateId) -> Option<StateId> {
    let wall_block = level.reg.block_id(&level.reg.name(standing).replace("_torch", "_wall_torch"))?;
    let wall = level.default_state(wall_block);
    // `WallTorchBlock.getStateForPlacement`.
    let wall_state = ctx.nearest_looking_all().into_iter().filter(|d| d.is_horizontal()).find_map(|d| {
        let s = level.with_dir(wall, "facing", d.opposite());
        can_survive(level, ctx.pos, s).then_some(s)
    });
    for d in ctx.nearest_looking_all() {
        if d == Dir::Up {
            continue;
        }
        let s = if d == Dir::Down { Some(standing) } else { wall_state };
        if let Some(s) = s.filter(|&s| can_survive(level, ctx.pos, s)) {
            return Some(s);
        }
    }
    None
}

/// `ChestBlock.getStateForPlacement`: a chest joins a single chest beside
/// it that faces the same way.
fn chest_state(level: &Level, ctx: &Context, base: StateId) -> StateId {
    let partner = |d: Dir| {
        let s = level.get(ctx.pos.offset(d));
        (level.block(s) == ctx.block && level.prop(s, "type") == "single").then(|| level.dir(s, "facing"))
    };
    let mut facing = ctx.horizontal().opposite();
    let mut ty = "single";
    let face = ctx.click.face;
    if face.is_horizontal()
        && ctx.click.sneak
        && let Some(d) = partner(face.opposite()).filter(|d| d.axis() != face.axis())
    {
        facing = d;
        ty = if d.counter_clockwise() == face.opposite() { "right" } else { "left" };
    }
    if ty == "single" && !ctx.click.sneak {
        if partner(facing.clockwise()) == Some(facing) {
            ty = "left";
        } else if partner(facing.counter_clockwise()) == Some(facing) {
            ty = "right";
        }
    }
    level.with(level.with_dir(base, "facing", facing), "type", ty)
}

/// `DoorBlock.getHinge`.
fn door_hinge(level: &Level, ctx: &Context) -> &'static str {
    let pos = ctx.pos;
    let dir = ctx.horizontal();
    let above = pos.above();
    let (ccw, cw) = (dir.counter_clockwise(), dir.clockwise());
    let (p1, p2, p3, p4) = (pos.offset(ccw), above.offset(ccw), pos.offset(cw), above.offset(cw));
    let score = -(full_block(level, p1) as i32) - full_block(level, p2) as i32
        + full_block(level, p3) as i32
        + full_block(level, p4) as i32;
    let lower_door = |p: Pos| {
        let s = level.get(p);
        level.kind(s) == Kind::Door && level.prop(s, "half") == "lower"
    };
    let (b1, b2) = (lower_door(p1), lower_door(p3));
    if (!b1 || b2) && score <= 0 {
        if (!b2 || b1) && score >= 0 {
            let [dx, _, dz] = dir.step();
            let (x, z) = ctx.click.hit_x_z(pos);
            if (dx >= 0 || z >= 0.5) && (dx <= 0 || z <= 0.5) && (dz >= 0 || x <= 0.5) && (dz <= 0 || x >= 0.5) {
                "left"
            } else {
                "right"
            }
        } else {
            "left"
        }
    } else {
        "right"
    }
}

/// `BellBlock.getStateForPlacement`.
fn bell_state(level: &Level, ctx: &Context, base: StateId) -> Option<StateId> {
    let pos = ctx.pos;
    let face = ctx.click.face;
    if face.axis() == 1 {
        let s = level.with(base, "attachment", if face == Dir::Down { "ceiling" } else { "floor" });
        let s = level.with_dir(s, "facing", ctx.horizontal());
        return can_survive(level, pos, s).then_some(s);
    }
    let (a, b) = if face.axis() == 0 { (Dir::West, Dir::East) } else { (Dir::North, Dir::South) };
    let double = sturdy(level, pos.offset(a), b) && sturdy(level, pos.offset(b), a);
    let s = level.with_dir(base, "facing", face.opposite());
    let s = level.with(s, "attachment", if double { "double_wall" } else { "single_wall" });
    if can_survive(level, pos, s) {
        return Some(s);
    }
    let s = level.with(s, "attachment", if sturdy(level, pos.below(), Dir::Up) { "floor" } else { "ceiling" });
    can_survive(level, pos, s).then_some(s)
}

/// `setPlacedBy`, run after the block is set.
fn placed_by(level: &mut Level, pos: Pos, state: StateId) {
    match level.kind(state) {
        Kind::TripwireHook => super::tripwire::hook_placed_by(level, pos, state),
        Kind::Door => {
            let upper = level.with(state, "half", "upper");
            level.set_block(pos.above(), upper, flags::ALL);
        }
        Kind::Repeater => {
            if super::repeater::should_turn_on(level, pos, state) {
                level.schedule_tick(pos, level.block(state), 1, 0);
            }
        }
        Kind::Comparator => {
            if super::comparator::should_turn_on(level, pos, state) {
                level.schedule_tick(pos, level.block(state), 1, 0);
            }
        }
        Kind::Piston | Kind::StickyPiston => super::piston::check_if_extend(level, pos, state),
        _ => {}
    }
}

/// Where `place` would put a block and the state it would set, without
/// changing the world: `BlockItem.place` up to `getStateForPlacement`. None
/// when the click would place nothing; an item the engine cannot place is
/// an error. The editor draws this as the block about to be placed.
pub fn preview(level: &Level, item: &str, click: Click) -> Result<Option<(Pos, StateId)>> {
    let Some(block) = block_for_item(level, item) else {
        bail!("placing {item} is not supported by the engine yet");
    };
    if level.kind(level.default_state(block)) == Kind::Other {
        bail!("{} is not supported by the engine yet", level.reg.name(level.default_state(block)));
    }
    let clicked_state = level.get(click.clicked);
    let replace_clicked = can_be_replaced(level, clicked_state, &click, click.clicked, block, true);
    let pos = if replace_clicked { click.clicked } else { click.clicked.offset(click.face) };
    let ctx = Context { click, pos, replace_clicked, block };
    if !replace_clicked && !can_be_replaced(level, level.get(pos), &click, pos, block, false) {
        return Ok(None);
    }
    Ok(state_for_placement(level, &ctx).map(|state| (pos, state)))
}

/// `BlockItem.place`: right-click `click.face` of `click.clicked` holding
/// `item`. Returns whether a block was placed; an item the engine cannot
/// place is an error.
pub fn place(level: &mut Level, item: &str, click: Click) -> Result<bool> {
    let Some((pos, state)) = preview(level, item, click)? else {
        return Ok(false);
    };
    if !level.set_block(pos, state, flags::ALL_IMMEDIATE) {
        return Ok(false);
    }
    let placed = level.get(pos);
    if level.block(placed) == level.block(state) {
        // `updateBlockEntityComponents`: the new block entity is marked changed.
        if level.block_entities.contains_key(&pos) {
            super::container::set_changed(level, pos);
        }
        placed_by(level, pos, placed);
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn looking_level_north_puts_north_first() {
        assert_eq!(ordered_by_nearest(180.0, 0.0)[0], Dir::North);
        assert_eq!(ordered_by_nearest(180.0, 80.0)[0], Dir::Down);
        assert_eq!(ordered_by_nearest(90.0, -70.0)[0], Dir::Up);
        assert_eq!(ordered_by_nearest(270.0, 10.0)[0], Dir::East);
    }
}
