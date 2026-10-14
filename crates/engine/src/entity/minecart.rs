//! `AbstractMinecart` with `OldMinecartBehavior` (the game's default; the
//! experimental minecart physics are off in 26.3 worlds): plain and
//! furnace minecarts, and chest and hopper minecarts, without riders,
//! which push each other. A hopper minecart takes items each tick unless
//! a powered activator rail turned it off.

use super::{Data, Entity, apply_gravity, floor, move_entity};
use crate::blocks::container::{self, Stack};
use crate::blocks::rail::is_rail_state;
use crate::level::Level;
use crate::physics::Vec3;
use crate::physics::mth;
use crate::pos::{Dir, Pos};

#[derive(Clone, Debug)]
pub struct Minecart {
    /// A furnace minecart; the engine's never have fuel.
    pub furnace: bool,
    /// `yRot`, which way the cart faces; carts only push carts in line
    /// with it.
    pub y_rot: f32,
    /// `flipped`: the cart turned round to face the way it moves.
    pub flipped: bool,
    /// What a chest or hopper minecart holds.
    pub items: Option<Vec<Stack>>,
    /// A hopper minecart, and whether it is taking items (`enabled`).
    pub hopper: bool,
    pub enabled: bool,
    /// `VehicleEntity`'s damage, which falls by 1 a tick; past 40 a hit
    /// breaks the cart.
    pub damage: f32,
    /// `hurtTime`: ticks left of the last hit's shake.
    pub hurt_time: i32,
}

impl Minecart {
    /// A minecart of the given type (`minecart`, `furnace_minecart`,
    /// `chest_minecart` or `hopper_minecart`).
    pub fn new(name: &str) -> Minecart {
        let size = match name {
            "chest_minecart" => 27,
            "hopper_minecart" => 5,
            _ => 0,
        };
        Minecart {
            furnace: name == "furnace_minecart",
            y_rot: 0.0,
            flipped: false,
            items: (size > 0).then(|| vec![Stack::default(); size]),
            hopper: name == "hopper_minecart",
            enabled: true,
            damage: 0.0,
            hurt_time: 0,
        }
    }

    /// `getDropItem`: what the cart leaves when it breaks.
    fn drop_item(&self) -> &'static str {
        match (self.furnace, self.hopper, self.items.is_some()) {
            (true, _, _) => "furnace_minecart",
            (_, true, _) => "hopper_minecart",
            (_, _, true) => "chest_minecart",
            _ => "minecart",
        }
    }

    pub fn type_name(&self) -> &'static str {
        match (self.furnace, self.hopper, self.items.is_some()) {
            (true, _, _) => "minecraft:furnace_minecart",
            (_, true, _) => "minecraft:hopper_minecart",
            (_, _, true) => "minecraft:chest_minecart",
            _ => "minecraft:minecart",
        }
    }
}

fn cart(e: &Entity) -> &Minecart {
    match &e.data {
        Data::Minecart(m) => m,
        _ => unreachable!("not a minecart"),
    }
}

fn cart_mut(e: &mut Entity) -> &mut Minecart {
    match &mut e.data {
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
/// A chest or hopper cart slows less the emptier it is.
fn natural_slowdown(e: &Entity, m: Vec3) -> Vec3 {
    if let Some(items) = &cart(e).items {
        let empty = 15 - container::signal_from_items(items);
        let mut f = 0.98f32 + empty as f32 * 0.001f32;
        if e.was_touching_water {
            f *= 0.95f32;
        }
        return m.multiply(f as f64, 0.0, f as f64);
    }
    let m = if cart(e).furnace { m.multiply(0.98, 0.0, 0.98) } else { m };
    let m = m.multiply(0.96, 0.0, 0.96);
    if e.was_touching_water { m.scale(0.95f32 as f64) } else { m }
}

/// `AbstractMinecart.tick`.
pub fn tick(level: &mut Level, e: &mut Entity) {
    let m = cart_mut(e);
    if m.hurt_time > 0 {
        m.hurt_time -= 1;
    }
    if m.damage > 0.0 {
        m.damage -= 1.0;
    }
    if e.pos.y < (level.min_y() - 64) as f64 {
        e.discard();
        return;
    }
    behavior_tick(level, e);
    if !e.removed {
        super::fluid::update_in_water(level, e);
        e.first_tick = false;
    }
    // `MinecartHopper.tick`, after the cart has moved. It is back on the
    // level's list while it takes items, as containers are found there.
    let m = cart(e);
    if m.hopper && m.enabled && !e.removed {
        level.with_listed(e, container::hopper_cart_suck);
    }
}

/// `OldMinecartBehavior.tick`, on the server.
fn behavior_tick(level: &mut Level, e: &mut Entity) {
    let old = e.pos;
    let y_rot_o = cart(e).y_rot;
    // `AbstractMinecart.getDefaultGravity`: a cart in water falls slowly.
    apply_gravity(e, if e.was_touching_water { 0.005 } else { 0.04 });
    let r = rail_pos(level, e.pos);
    let state = level.get(r);
    if is_rail_state(level, state) {
        move_along_track(level, e);
        // `activateMinecart`: a hopper minecart is off on a powered
        // activator rail and on again on an unpowered one, and a plain
        // minecart on a powered one shakes as if hit hard (`Minecart`).
        if level.reg.name(state) == "minecraft:activator_rail" {
            let powered = level.flag(state, "powered");
            let m = cart_mut(e);
            if m.hopper {
                m.enabled = !powered;
            } else if powered && !m.furnace && m.items.is_none() && m.hurt_time == 0 {
                m.hurt_time = 10;
                m.damage = 50.0;
            }
        }
    } else {
        come_off_track(level, e);
    }
    // `AbstractMinecart.applyEffectsFromBlocks`: only where the cart is.
    let at = e.pos;
    super::apply_effects_between(level, e, at, at);
    e.clear_movements();
    turn(e, old, y_rot_o);
    push_entities(level, e);
}

/// The end of `OldMinecartBehavior.tick`: face the way the cart moved,
/// turning round (`flipped`) rather than swinging by more than 170°.
fn turn(e: &mut Entity, old: Vec3, y_rot_o: f32) {
    let (dx, dz) = (old.x - e.pos.x, old.z - e.pos.z);
    let m = cart_mut(e);
    if dx * dx + dz * dz > 0.001 {
        m.y_rot = (mth::atan2(dz, dx) * 180.0 / std::f64::consts::PI) as f32;
        if m.flipped {
            m.y_rot += 180.0;
        }
    }
    let delta = mth::wrap_degrees(m.y_rot - y_rot_o);
    if !(-170.0..170.0).contains(&delta) {
        m.y_rot += 180.0;
        m.flipped = !m.flipped;
    }
    m.y_rot %= 360.0;
}

/// `pushAndPickupEntities`: each other cart close by gets
/// `push(this cart)`. The engine has no mobs for a cart to pick up.
fn push_entities(level: &mut Level, e: &mut Entity) {
    let b = e.bounding_box().inflate(0.2, 0.0, 0.2);
    for i in 0..level.entities.len() {
        let o = &level.entities[i];
        if !o.removed && matches!(o.data, Data::Minecart(_)) && o.bounding_box().intersects(&b) {
            push(&mut level.entities[i], e);
        }
    }
}

/// `AbstractMinecart.push(entity)` on cart `b` by cart `a`: if `a` is in
/// line with the way `b` faces, the two share their speed and are pushed
/// apart. A furnace cart takes most of a plain cart's speed instead.
fn push(b: &mut Entity, a: &mut Entity) {
    let (mut dx, mut dz) = (a.pos.x - b.pos.x, a.pos.z - b.pos.z);
    let mut dd = dx * dx + dz * dz;
    if dd < 1.0E-4f32 as f64 {
        return;
    }
    dd = dd.sqrt();
    dx /= dd;
    dz /= dd;
    let inv = (1.0 / dd).min(1.0);
    dx *= inv;
    dz *= inv;
    dx *= 0.1f32 as f64;
    dz *= 0.1f32 as f64;
    dx *= 0.5;
    dz *= 0.5;
    let v = Vec3::new(a.pos.x - b.pos.x, 0.0, a.pos.z - b.pos.z).normalize();
    let r = cart(b).y_rot as f64 * (std::f32::consts::PI / 180.0) as f64;
    let look = Vec3::new(mth::cos(r) as f64, 0.0, mth::sin(r) as f64).normalize();
    if (v.x * look.x + v.z * look.z).abs() < 0.8f32 as f64 {
        return;
    }
    let (mb, ma) = (b.motion, a.motion);
    match (cart(a).furnace, cart(b).furnace) {
        (true, false) => {
            b.motion = mb.multiply(0.2, 1.0, 0.2) + Vec3::new(ma.x - dx, 0.0, ma.z - dz);
            a.motion = ma.multiply(0.95, 1.0, 0.95);
        }
        (false, true) => {
            a.motion = ma.multiply(0.2, 1.0, 0.2) + Vec3::new(mb.x + dx, 0.0, mb.z + dz);
            b.motion = mb.multiply(0.95, 1.0, 0.95);
        }
        _ => {
            let (mx, mz) = ((ma.x + mb.x) / 2.0, (ma.z + mb.z) / 2.0);
            b.motion = mb.multiply(0.2, 1.0, 0.2) + Vec3::new(mx - dx, 0.0, mz - dz);
            a.motion = ma.multiply(0.2, 1.0, 0.2) + Vec3::new(mx + dx, 0.0, mz + dz);
        }
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

/// `VehicleEntity.hurtServer` for the cart at `k`, hit by an explosion for
/// `amount`: past 40 damage it breaks (`destroy`).
pub fn hurt(level: &mut Level, k: usize, amount: f32) {
    let e = &mut level.entities[k];
    if e.removed {
        return;
    }
    let m = cart_mut(e);
    m.hurt_time = 10;
    m.damage += amount * 10.0;
    let broken = m.damage > 40.0;
    // `gameEvent(ENTITY_DAMAGE)`, and `ENTITY_DIE` when it breaks.
    let at = e.pos;
    if crate::blocks::could_hear_entity(level, at) {
        level.unsupported("a sculk sensor hearing a minecart hit by an explosion");
    }
    if broken {
        destroy(level, k);
    }
}

/// `VehicleEntity.destroy`, for a chest or hopper cart as
/// `AbstractMinecartContainer` overrides it: killing the cart spills what
/// it holds (`remove`), then the cart's own item drops, then
/// `chestVehicleDestroyed` spills it again, which finds every slot empty
/// but still draws a place for each.
fn destroy(level: &mut Level, k: usize) {
    let e = &mut level.entities[k];
    e.discard();
    let at = e.pos;
    let m = cart_mut(e);
    let drop = m.drop_item();
    let items = m.items.take();
    if let Some(mut items) = items {
        drop_contents(level, at, &mut items);
        super::item::spawn_at_location(level, at, Stack::new(drop, 1));
        drop_contents(level, at, &mut items);
        cart_mut(&mut level.entities[k]).items = Some(items);
    } else {
        super::item::spawn_at_location(level, at, Stack::new(drop, 1));
    }
}

/// `Containers.dropContents` at an entity: each slot's stack thrown out in
/// handfuls of 10 to 30, from a random point in the block the entity is in.
fn drop_contents(level: &mut Level, at: Vec3, items: &mut [Stack]) {
    // `EntityTypes.ITEM.getWidth()`.
    let size = 0.25f32 as f64;
    let range = 1.0 - size;
    for stack in items.iter_mut() {
        let x = at.x.floor() + level.random.next_double() * range + size / 2.0;
        let y = at.y.floor() + level.random.next_double() * range;
        let z = at.z.floor() + level.random.next_double() * range + size / 2.0;
        while !stack.is_empty() {
            let n = level.random.next_int_bounded(21) + 10;
            let part = stack.split(n);
            let id = part.id.strip_prefix("minecraft:").unwrap_or(&part.id).to_string();
            let mut item = super::item::new_tossed(level, Vec3::new(x, y, z), Stack::new(&id, part.count));
            let spread = 0.11485000171139836;
            let dx = level.random.triangle(0.0, spread);
            let dy = level.random.triangle(0.2, spread);
            let dz = level.random.triangle(0.0, spread);
            item.motion = Vec3::new(dx, dy, dz);
            level.add_entity(item);
        }
    }
}
