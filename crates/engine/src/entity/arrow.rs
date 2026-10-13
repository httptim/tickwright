//! Arrows (`Arrow`, through `AbstractArrow`): they fly under drag and
//! gravity until a ray along their motion meets a block's collision shape,
//! then stick just short of the hit. A target block lights up by how near
//! the middle it was hit, and a wooden button with an arrow in it stays
//! pressed. Arrows hitting entities, falling out of the ground and flying
//! through water are refused.

use super::{
    Data, Entity, apply_effects_between, apply_effects_from_blocks, base_tick, collision_shape, floor,
    no_block_collision,
};
use crate::blocks::{Kind, plain};
use crate::level::Level;
use crate::physics::{Aabb, Vec3};
use crate::pos::{Dir, Pos};
use crate::registry::StateId;

#[derive(Clone, Debug, Default)]
pub struct Arrow {
    pub in_ground: bool,
    /// The state of the block it stuck in (`lastState`). While the block
    /// at the arrow differs from it, the arrow falls once nothing holds it.
    last_state: Option<StateId>,
    /// Ticks spent in the ground; it despawns after 1200.
    life: i32,
}

/// What a ray met: where, which block, and the face it crossed.
pub(super) struct Hit {
    pub at: Vec3,
    pub pos: Pos,
    pub face: Dir,
}

fn arrow(e: &mut Entity) -> &mut Arrow {
    match &mut e.data {
        Data::Arrow(a) => a,
        _ => unreachable!(),
    }
}

/// `AbstractArrow.tick`.
pub fn tick(level: &mut Level, e: &mut Entity) {
    base_tick(level, e);
    if e.was_touching_water {
        level.unsupported("an arrow in water");
        return;
    }
    if arrow(e).in_ground {
        in_ground_tick(level, e);
        return;
    }
    let from = e.pos;
    let to = from + e.motion;
    let hit = clip(level, from, to);
    if level.unsupported.is_some() {
        return;
    }
    let end = hit.as_ref().map_or(to, |h| h.at);
    if hits_an_entity(level, e, from, end) {
        level.unsupported("an arrow hitting an entity");
        return;
    }
    match hit {
        Some(h) => {
            // Stuck just short of the hit, by 0.05 against each way it was
            // going (fitted from 26.3 captures).
            let back = |v: f64| {
                if v > 0.0 {
                    0.05f32 as f64
                } else if v < 0.0 {
                    -(0.05f32 as f64)
                } else {
                    0.0
                }
            };
            let m = e.motion;
            e.pos = Vec3::new(h.at.x - back(m.x), h.at.y - back(m.y), h.at.z - back(m.z));
            e.motion = Vec3::ZERO;
            let state = level.get(h.pos);
            let a = arrow(e);
            a.in_ground = true;
            a.last_state = Some(state);
            match level.kind(state) {
                Kind::Target => plain::target_hit(level, h.pos, state, [h.at.x, h.at.y, h.at.z], h.face, true),
                Kind::Bell | Kind::Tnt => level.unsupported("an arrow hitting a bell or TNT"),
                _ => {}
            }
        }
        None => {
            e.pos = to;
            let f = 0.99f32 as f64;
            let m = e.motion;
            e.motion = Vec3::new(m.x * f, m.y * f - 0.05, m.z * f);
        }
    }
    let to = e.pos;
    apply_effects_between(level, e, from, to);
}

/// An arrow in the ground stays until what holds it goes, or it despawns.
fn in_ground_tick(level: &mut Level, e: &mut Entity) {
    let state = level.get(e.block_position());
    let p = e.pos;
    let around = Aabb::new(p.x, p.y, p.z, p.x, p.y, p.z).inflate(0.06, 0.06, 0.06);
    if arrow(e).last_state != Some(state) && no_block_collision(level, around) {
        level.unsupported("an arrow falling out of the ground");
        return;
    }
    let a = arrow(e);
    a.life += 1;
    if a.life >= 1200 {
        e.discard();
        return;
    }
    apply_effects_from_blocks(level, e);
}

/// Whether anything a projectile can hit lies on its path. Items and
/// arrows can't be hit; the rest are refused rather than simulated.
pub(super) fn hits_an_entity(level: &Level, e: &Entity, from: Vec3, to: Vec3) -> bool {
    level.entities.iter().any(|o| {
        o.id != e.id
            && !o.removed
            && !matches!(o.data, Data::Item(_) | Data::Arrow(_) | Data::Gone)
            && o.bounding_box().inflate(0.3, 0.3, 0.3).clip([from.x, from.y, from.z], [to.x, to.y, to.z]).is_some()
    })
}

/// `Level.clip` with `ClipContext.Block.COLLIDER` and no fluids, as
/// `BlockGetter.traverseBlocks` walks it.
pub(super) fn clip(level: &mut Level, from: Vec3, to: Vec3) -> Option<Hit> {
    if from == to {
        return None;
    }
    let lerp = |t: f64, a: f64, b: f64| a + t * (b - a);
    let frac = |v: f64| v - v.floor();
    let sign = |v: f64| {
        if v == 0.0 {
            0
        } else if v > 0.0 {
            1
        } else {
            -1
        }
    };
    let ex = lerp(-1.0E-7, to.x, from.x);
    let ey = lerp(-1.0E-7, to.y, from.y);
    let ez = lerp(-1.0E-7, to.z, from.z);
    let sx = lerp(-1.0E-7, from.x, to.x);
    let sy = lerp(-1.0E-7, from.y, to.y);
    let sz = lerp(-1.0E-7, from.z, to.z);
    let (mut x, mut y, mut z) = (floor(sx), floor(sy), floor(sz));
    if let Some(h) = block_clip(level, Pos::new(x, y, z), from, to) {
        return Some(h);
    }
    let (m, n, o) = (ex - sx, ey - sy, ez - sz);
    let (p, q, r) = (sign(m), sign(n), sign(o));
    let s = if p == 0 { f64::MAX } else { p as f64 / m };
    let t = if q == 0 { f64::MAX } else { q as f64 / n };
    let u = if r == 0 { f64::MAX } else { r as f64 / o };
    let mut v = s * if p > 0 { 1.0 - frac(sx) } else { frac(sx) };
    let mut w = t * if q > 0 { 1.0 - frac(sy) } else { frac(sy) };
    let mut xx = u * if r > 0 { 1.0 - frac(sz) } else { frac(sz) };
    while v <= 1.0 || w <= 1.0 || xx <= 1.0 {
        if v < w {
            if v < xx {
                x += p;
                v += s;
            } else {
                z += r;
                xx += u;
            }
        } else if w < xx {
            y += q;
            w += t;
        } else {
            z += r;
            xx += u;
        }
        if let Some(h) = block_clip(level, Pos::new(x, y, z), from, to) {
            return Some(h);
        }
    }
    None
}

/// `VoxelShape.clip` for the block at `p`: the nearest face of its
/// collision boxes the segment enters through.
fn block_clip(level: &mut Level, p: Pos, from: Vec3, to: Vec3) -> Option<Hit> {
    let state = level.get(p);
    let shape = collision_shape(level, p, state);
    if shape.is_empty() {
        return None;
    }
    let d = to - from;
    if d.length_sqr() < 1.0E-7 {
        return None;
    }
    let near = from + d.scale(0.001);
    let local = [near.x - p.x as f64, near.y - p.y as f64, near.z - p.z as f64];
    if shape.iter().any(|b| (0..3).all(|a| b.min[a] <= local[a] && local[a] < b.max[a])) {
        level.unsupported("a projectile starting inside a block");
        return None;
    }
    let from = [from.x, from.y, from.z];
    let d = [d.x, d.y, d.z];
    let mut scale = 1.0;
    let mut axis = None;
    for b in &shape {
        let b = b.offset(p.x as f64, p.y as f64, p.z as f64);
        for a in 0..3 {
            let plane = if d[a] > 1.0E-7 {
                b.min[a]
            } else if d[a] < -1.0E-7 {
                b.max[a]
            } else {
                continue;
            };
            let t = (plane - from[a]) / d[a];
            let (i, j) = ((a + 1) % 3, (a + 2) % 3);
            let pi = from[i] + t * d[i];
            let pj = from[j] + t * d[j];
            if 0.0 < t
                && t < scale
                && b.min[i] - 1.0E-7 < pi
                && pi < b.max[i] + 1.0E-7
                && b.min[j] - 1.0E-7 < pj
                && pj < b.max[j] + 1.0E-7
            {
                scale = t;
                axis = Some(a as u8);
            }
        }
    }
    let axis = axis?;
    let at = Vec3::new(from[0] + scale * d[0], from[1] + scale * d[1], from[2] + scale * d[2]);
    let toward = d[axis as usize] > 0.0;
    let face = match axis {
        0 => {
            if toward {
                Dir::West
            } else {
                Dir::East
            }
        }
        1 => {
            if toward {
                Dir::Down
            } else {
                Dir::Up
            }
        }
        _ => {
            if toward {
                Dir::North
            } else {
                Dir::South
            }
        }
    };
    Some(Hit { at, pos: p, face })
}
