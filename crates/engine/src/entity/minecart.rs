//! `AbstractMinecart` with `OldMinecartBehavior` (the game's default; the
//! experimental minecart physics are off in 26.3 worlds): plain and
//! furnace minecarts, without riders.

use super::{Data, Entity, apply_gravity, floor, move_entity};
use crate::blocks::rail::is_rail_state;
use crate::level::Level;
use crate::physics::Vec3;
use crate::pos::{Dir, Pos};

#[derive(Clone, Debug)]
pub struct Minecart {
    /// A furnace minecart; the engine's never have fuel.
    pub furnace: bool,
}

fn cart(e: &Entity) -> &Minecart {
    match &e.data {
        Data::Minecart(m) => m,
        _ => unreachable!("not a minecart"),
    }
}

/// `AbstractMinecart.exits(shape)`: the two ends of a rail, as offsets.
fn exits(shape: &str) -> ([i32; 3], [i32; 3]) {
    match shape {
        "north_south" => ([0, 0, -1], [0, 0, 1]),
        "east_west" => ([-1, 0, 0], [1, 0, 0]),
        "ascending_east" => ([-1, -1, 0], [1, 0, 0]),
        "ascending_west" => ([-1, 0, 0], [1, -1, 0]),
        "ascending_north" => ([0, 0, -1], [0, -1, 1]),
        "ascending_south" => ([0, -1, -1], [0, 0, 1]),
        "south_east" => ([0, 0, 1], [1, 0, 0]),
        "south_west" => ([0, 0, 1], [-1, 0, 0]),
        "north_west" => ([0, 0, -1], [-1, 0, 0]),
        _ => ([0, 0, -1], [1, 0, 0]),
    }
}

/// `getCurrentBlockPosOrRailBelow`.
fn rail_pos(level: &Level, p: Vec3) -> Pos {
    let pos = Pos::new(floor(p.x), floor(p.y), floor(p.z));
    if is_rail_state(level, level.get(pos.below())) { pos.below() } else { pos }
}

/// `OldMinecartBehavior.getPos`: where on the rail under `p` a cart there
/// sits, or `None` off the rails.
fn rail_point(level: &Level, p: Vec3) -> Option<Vec3> {
    let r = rail_pos(level, p);
    let state = level.get(r);
    if !is_rail_state(level, state) {
        return None;
    }
    let (e0, e1) = exits(level.prop(state, "shape"));
    let (x0, y0, z0) = (
        r.x as f64 + 0.5 + e0[0] as f64 * 0.5,
        r.y as f64 + 0.0625 + e0[1] as f64 * 0.5,
        r.z as f64 + 0.5 + e0[2] as f64 * 0.5,
    );
    let (x1, y1, z1) = (
        r.x as f64 + 0.5 + e1[0] as f64 * 0.5,
        r.y as f64 + 0.0625 + e1[1] as f64 * 0.5,
        r.z as f64 + 0.5 + e1[2] as f64 * 0.5,
    );
    let (xd, yd, zd) = (x1 - x0, (y1 - y0) * 2.0, z1 - z0);
    let progress = if xd == 0.0 {
        p.z - r.z as f64
    } else if zd == 0.0 {
        p.x - r.x as f64
    } else {
        ((p.x - x0) * xd + (p.z - z0) * zd) * 2.0
    };
    let x = x0 + xd * progress;
    let mut y = y0 + yd * progress;
    let z = z0 + zd * progress;
    if yd < 0.0 {
        y += 1.0;
    } else if yd > 0.0 {
        y += 0.5;
    }
    Some(Vec3::new(x, y, z))
}

/// `OldMinecartBehavior.getMaxSpeed`, and `FurnaceMinecart`'s.
fn max_speed(e: &Entity) -> f64 {
    let furnace = cart(e).furnace;
    match (furnace, e.was_touching_water) {
        (false, false) => 8.0 / 20.0,
        (false, true) => 4.0 / 20.0,
        (true, false) => 4.0 / 20.0,
        (true, true) => 3.0 / 20.0,
    }
}

/// `applyNaturalSlowdown`: a furnace cart without fuel slows by 0.98 first.
fn natural_slowdown(e: &Entity, m: Vec3) -> Vec3 {
    let m = if cart(e).furnace { m.multiply(0.98, 0.0, 0.98) } else { m };
    let m = m.multiply(0.96, 0.0, 0.96);
    if e.was_touching_water { m.scale(0.95f32 as f64) } else { m }
}

/// `AbstractMinecart.tick`.
pub fn tick(level: &mut Level, e: &mut Entity) {
    if e.pos.y < (level.min_y() - 64) as f64 {
        e.discard();
        return;
    }
    behavior_tick(level, e);
    if !e.removed {
        super::fluid::update_in_water(level, e);
        e.first_tick = false;
    }
}

/// `OldMinecartBehavior.tick`, on the server.
fn behavior_tick(level: &mut Level, e: &mut Entity) {
    let old = e.pos;
    apply_gravity(e, 0.04);
    let r = rail_pos(level, e.pos);
    if is_rail_state(level, level.get(r)) {
        move_along_track(level, e);
    } else {
        come_off_track(level, e);
    }
    super::apply_effects_from_blocks(level, e, old);
    push_entities(level, e);
}

/// `pushAndPickupEntities`, for the carts the engine has: another cart
/// close by would be pushed, which isn't simulated yet.
fn push_entities(level: &mut Level, e: &Entity) {
    let b = e.bounding_box().inflate(0.2, 0.0, 0.2);
    if level
        .entities
        .iter()
        .any(|o| !o.removed && matches!(o.data, Data::Minecart(_)) && o.bounding_box().intersects(&b))
    {
        level.unsupported("minecarts pushing each other");
    }
}

/// `comeOffTrack`.
fn come_off_track(level: &mut Level, e: &mut Entity) {
    let max = max_speed(e);
    let m = e.motion;
    e.motion = Vec3::new(m.x.clamp(-max, max), m.y, m.z.clamp(-max, max));
    if e.on_ground {
        e.motion = e.motion.scale(0.5);
    }
    move_entity(level, e, e.motion);
    if !e.on_ground {
        e.motion = e.motion.scale(0.95f32 as f64);
    }
}

/// `OldMinecartBehavior.moveAlongTrack`.
fn move_along_track(level: &mut Level, e: &mut Entity) {
    let r = rail_pos(level, e.pos);
    let state = level.get(r);
    let (x, z) = (e.pos.x, e.pos.z);
    let old_point = rail_point(level, e.pos);
    let mut y = r.y as f64;
    let is_powered_rail = level.reg.name(state) == "minecraft:powered_rail";
    let power_track = is_powered_rail && level.flag(state, "powered");
    let halt_track = is_powered_rail && !power_track;
    let mut slide = 0.0078125;
    if e.was_touching_water {
        slide *= 0.2;
    }
    let shape = level.prop(state, "shape");
    let m = e.motion;
    match shape {
        "ascending_east" => {
            e.motion = m + Vec3::new(-slide, 0.0, 0.0);
            y += 1.0;
        }
        "ascending_west" => {
            e.motion = m + Vec3::new(slide, 0.0, 0.0);
            y += 1.0;
        }
        "ascending_north" => {
            e.motion = m + Vec3::new(0.0, 0.0, slide);
            y += 1.0;
        }
        "ascending_south" => {
            e.motion = m + Vec3::new(0.0, 0.0, -slide);
            y += 1.0;
        }
        _ => {}
    }
    let m = e.motion;
    let (e0, e1) = exits(shape);
    let mut xd = (e1[0] - e0[0]) as f64;
    let mut zd = (e1[2] - e0[2]) as f64;
    let dir_length = (xd * xd + zd * zd).sqrt();
    if m.x * xd + m.z * zd < 0.0 {
        xd = -xd;
        zd = -zd;
    }
    let pow = m.horizontal_distance().min(2.0);
    e.motion = Vec3::new(pow * xd / dir_length, m.y, pow * zd / dir_length);
    if halt_track {
        if e.motion.horizontal_distance() < 0.03 {
            e.motion = Vec3::ZERO;
        } else {
            e.motion = e.motion.multiply(0.5, 0.0, 0.5);
        }
    }
    let x0 = r.x as f64 + 0.5 + e0[0] as f64 * 0.5;
    let z0 = r.z as f64 + 0.5 + e0[2] as f64 * 0.5;
    let x1 = r.x as f64 + 0.5 + e1[0] as f64 * 0.5;
    let z1 = r.z as f64 + 0.5 + e1[2] as f64 * 0.5;
    let (xd, zd) = (x1 - x0, z1 - z0);
    let progress = if xd == 0.0 {
        z - r.z as f64
    } else if zd == 0.0 {
        x - r.x as f64
    } else {
        ((x - x0) * xd + (z - z0) * zd) * 2.0
    };
    e.pos = Vec3::new(x0 + xd * progress, y, z0 + zd * progress);
    let max = max_speed(e);
    let m = e.motion;
    move_entity(level, e, Vec3::new(m.x.clamp(-max, max), 0.0, m.z.clamp(-max, max)));
    let (fx, fz) = (floor(e.pos.x) - r.x, floor(e.pos.z) - r.z);
    if e0[1] != 0 && fx == e0[0] && fz == e0[2] {
        e.pos = Vec3::new(e.pos.x, e.pos.y + e0[1] as f64, e.pos.z);
    } else if e1[1] != 0 && fx == e1[0] && fz == e1[2] {
        e.pos = Vec3::new(e.pos.x, e.pos.y + e1[1] as f64, e.pos.z);
    }
    e.motion = natural_slowdown(e, e.motion);
    if let (Some(new_point), Some(old_point)) = (rail_point(level, e.pos), old_point) {
        let speed = (old_point.y - new_point.y) * 0.05;
        let m = e.motion;
        let len = m.horizontal_distance();
        if len > 0.0 {
            e.motion = m.multiply((len + speed) / len, 1.0, (len + speed) / len);
        }
        e.pos = Vec3::new(e.pos.x, new_point.y, e.pos.z);
    }
    let (xn, zn) = (floor(e.pos.x), floor(e.pos.z));
    if xn != r.x || zn != r.z {
        let m = e.motion;
        let len = m.horizontal_distance();
        e.motion = Vec3::new(len * (xn - r.x) as f64, m.y, len * (zn - r.z) as f64);
    }
    if power_track {
        let m = e.motion;
        let len = m.horizontal_distance();
        if len > 0.01 {
            e.motion = m + Vec3::new(m.x / len * 0.06, 0.0, m.z / len * 0.06);
        } else {
            let (mut xdd, mut zdd) = (m.x, m.z);
            let conductor = |level: &Level, d: Dir| {
                let p = r.offset(d);
                crate::blocks::is_conductor(level, p, level.get(p))
            };
            if shape == "east_west" {
                if conductor(level, Dir::West) {
                    xdd = 0.02;
                } else if conductor(level, Dir::East) {
                    xdd = -0.02;
                }
            } else if shape == "north_south" {
                if conductor(level, Dir::North) {
                    zdd = 0.02;
                } else if conductor(level, Dir::South) {
                    zdd = -0.02;
                }
            } else {
                return;
            }
            e.motion = Vec3::new(xdd, m.y, zdd);
        }
    }
}
