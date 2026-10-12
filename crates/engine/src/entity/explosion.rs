//! `ServerExplosion`: rays that break blocks, knockback and damage for
//! entities nearby, and the drops.

use super::{Data, Entity, floor, item};
use crate::blocks::Kind;
use crate::game_event::GameEvent;
use crate::level::{Level, flags};
use crate::physics::{Aabb, Vec3};
use crate::pos::Pos;

/// `Level.explode` for an explosion that breaks blocks and drops all of
/// them (`ExplosionInteraction.TNT` without decay). `source` is the entity
/// that exploded, already removed from the level's list while it ticks.
pub fn explode(level: &mut Level, source: Option<&Entity>, center: Vec3, radius: f32, fire: bool) {
    if fire {
        level.unsupported("an explosion that starts fires");
    }
    level.game_event(GameEvent::Explode, Pos::new(floor(center.x), floor(center.y), floor(center.z)));
    let blocks = exploded_positions(level, center, radius);
    hurt_entities(level, source, center, radius);
    interact_with_blocks(level, blocks);
}

/// `ServerExplosion.calculateExplodedPositions`: 1352 rays from the
/// surface of a 16³ grid, each with a random strength, losing strength to
/// every block they pass.
fn exploded_positions(level: &mut Level, center: Vec3, radius: f32) -> Vec<Pos> {
    let physics = level.physics();
    let mut set = JavaHashSet::default();
    for i in 0..16 {
        for j in 0..16 {
            for k in 0..16 {
                if !(i == 0 || i == 15 || j == 0 || j == 15 || k == 0 || k == 15) {
                    continue;
                }
                let mut d = (i as f32 / 15.0 * 2.0 - 1.0) as f64;
                let mut e = (j as f32 / 15.0 * 2.0 - 1.0) as f64;
                let mut f = (k as f32 / 15.0 * 2.0 - 1.0) as f64;
                let g = (d * d + e * e + f * f).sqrt();
                d /= g;
                e /= g;
                f /= g;
                let mut h = radius * (0.7 + level.random.next_float() * 0.6);
                let (mut m, mut n, mut o) = (center.x, center.y, center.z);
                let step = 0.3f32 as f64;
                while h > 0.0 {
                    let p = Pos::new(floor(m), floor(n), floor(o));
                    if !(level.min_y()..=level.max_y()).contains(&p.y) {
                        break;
                    }
                    let state = level.get(p);
                    // Water's resistance is its fluid's, 100, whatever block holds it.
                    let wet = !crate::blocks::water::fluid_state(level, state).is_empty();
                    let resistance = match level.kind(state) {
                        Kind::Air => None,
                        _ if wet => Some(physics.factors(level.block(state)).resistance.max(100.0)),
                        _ => Some(physics.factors(level.block(state)).resistance),
                    };
                    if let Some(r) = resistance {
                        h -= (r + 0.3) * 0.3;
                    }
                    // Air counts too: it is shuffled with the rest and breaks
                    // into nothing.
                    if h > 0.0 {
                        set.insert(p);
                    }
                    m += d * step;
                    n += e * step;
                    o += f * step;
                    h -= 0.22500001;
                }
            }
        }
    }
    if set.treeified {
        level.unsupported("an explosion whose block set would hold a tree bin");
    }
    set.iteration_order()
}

/// `ServerExplosion.hurtEntities`: knockback by how much of each entity the
/// centre can see, and damage that destroys items.
fn hurt_entities(level: &mut Level, source: Option<&Entity>, center: Vec3, radius: f32) {
    let f = (radius * 2.0) as f64;
    let b = Aabb::new(
        floor(center.x - f - 1.0) as f64,
        floor(center.y - f - 1.0) as f64,
        floor(center.z - f - 1.0) as f64,
        floor(center.x + f + 1.0) as f64,
        floor(center.y + f + 1.0) as f64,
        floor(center.z + f + 1.0) as f64,
    );
    let source_id = source.map(|s| s.id);
    let mut hit: Vec<usize> = (0..level.entities.len())
        .filter(|&k| {
            let e = &level.entities[k];
            !e.removed && Some(e.id) != source_id && e.bounding_box().intersects(&b)
        })
        .collect();
    hit.sort_by_key(|&k| level.entities[k].id);
    for k in hit {
        let e = &level.entities[k];
        let d = e.pos.distance_to_sqr(center).sqrt() / f;
        if d > 1.0 {
            continue;
        }
        let eye = match e.data {
            Data::Tnt(_) => e.pos.y,
            Data::Item(_) => e.pos.y + 0.2125f32 as f64,
            Data::Minecart(_) => e.pos.y + (0.7f32 * 0.85) as f64,
            _ => e.pos.y + (0.98f32 * 0.85) as f64,
        };
        let (mut x, mut y, mut z) = (e.pos.x - center.x, eye - center.y, e.pos.z - center.z);
        let len = (x * x + y * y + z * z).sqrt();
        if len == 0.0 {
            continue;
        }
        x /= len;
        y /= len;
        z /= len;
        let seen = seen_percent(level, center, e.bounding_box());
        if let Data::Item(_) = e.data {
            let a = (1.0 - d) * seen as f64;
            let damage = ((a * a + a) / 2.0 * 7.0 * f + 1.0) as f32;
            let e = &mut level.entities[k];
            let Data::Item(i) = &mut e.data else { unreachable!() };
            i.health = (i.health as f32 - damage) as i32;
            if i.health <= 0 {
                e.discard();
            }
        }
        let r = (1.0 - d) * seen as f64;
        let e = &mut level.entities[k];
        e.motion = e.motion + Vec3::new(x * r, y * r, z * r);
    }
}

/// `ServerExplosion.getSeenPercent`: the share of points on a grid over the
/// entity's box with a clear line to the centre.
pub fn seen_percent(level: &Level, center: Vec3, b: Aabb) -> f32 {
    let d = 1.0 / ((b.max[0] - b.min[0]) * 2.0 + 1.0);
    let e = 1.0 / ((b.max[1] - b.min[1]) * 2.0 + 1.0);
    let f = 1.0 / ((b.max[2] - b.min[2]) * 2.0 + 1.0);
    let g = (1.0 - (1.0 / d).floor() * d) / 2.0;
    let h = (1.0 - (1.0 / f).floor() * f) / 2.0;
    if d < 0.0 || e < 0.0 || f < 0.0 {
        return 0.0;
    }
    let (mut seen, mut total) = (0, 0);
    let mut k = 0.0;
    while k <= 1.0 {
        let mut l = 0.0;
        while l <= 1.0 {
            let mut m = 0.0;
            while m <= 1.0 {
                let n = lerp(k, b.min[0], b.max[0]);
                let o = lerp(l, b.min[1], b.max[1]);
                let p = lerp(m, b.min[2], b.max[2]);
                if !clip_hits(level, Vec3::new(n + g, o, p + h), center) {
                    seen += 1;
                }
                total += 1;
                m += f;
            }
            l += e;
        }
        k += d;
    }
    seen as f32 / total as f32
}

/// `Mth.lerp`.
fn lerp(t: f64, a: f64, b: f64) -> f64 {
    a + t * (b - a)
}

fn frac(v: f64) -> f64 {
    v - v.floor()
}

fn sign(v: f64) -> i32 {
    if v == 0.0 {
        0
    } else if v > 0.0 {
        1
    } else {
        -1
    }
}

/// `Level.clip` with `ClipContext.Block.COLLIDER` and no fluids: whether a
/// collision box lies between `from` and `to`.
pub fn clip_hits(level: &Level, from: Vec3, to: Vec3) -> bool {
    if from == to {
        return false;
    }
    let ex = lerp(-1.0E-7, to.x, from.x);
    let ey = lerp(-1.0E-7, to.y, from.y);
    let ez = lerp(-1.0E-7, to.z, from.z);
    let sx = lerp(-1.0E-7, from.x, to.x);
    let sy = lerp(-1.0E-7, from.y, to.y);
    let sz = lerp(-1.0E-7, from.z, to.z);
    let (mut x, mut y, mut z) = (floor(sx), floor(sy), floor(sz));
    if block_clip(level, Pos::new(x, y, z), from, to) {
        return true;
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
        if block_clip(level, Pos::new(x, y, z), from, to) {
            return true;
        }
    }
    false
}

/// `VoxelShape.clip` for the block at `p`.
fn block_clip(level: &Level, p: Pos, from: Vec3, to: Vec3) -> bool {
    let state = level.get(p);
    let shape = level.physics().shape(state);
    if shape.is_empty() {
        return false;
    }
    let v = to - from;
    if v.length_sqr() < 1.0E-7 {
        return false;
    }
    let near = from + v.scale(0.001);
    let local = [near.x - p.x as f64, near.y - p.y as f64, near.z - p.z as f64];
    if shape.iter().any(|b| (0..3).all(|a| b.min[a] <= local[a] && local[a] < b.max[a])) {
        return true;
    }
    shape.iter().any(|b| aabb_clip(b.offset(p.x as f64, p.y as f64, p.z as f64), from, to))
}

/// `AABB.clip`: whether the segment enters the box through a face.
fn aabb_clip(b: Aabb, from: Vec3, to: Vec3) -> bool {
    let d = [to.x - from.x, to.y - from.y, to.z - from.z];
    let s = [from.x, from.y, from.z];
    let mut best = 1.0;
    let mut hit = false;
    for a in 0..3 {
        let face = if d[a] > 1.0E-7 {
            b.min[a]
        } else if d[a] < -1.0E-7 {
            b.max[a]
        } else {
            continue;
        };
        let (o1, o2) = ((a + 1) % 3, (a + 2) % 3);
        let t = (face - s[a]) / d[a];
        let p1 = s[o1] + t * d[o1];
        let p2 = s[o2] + t * d[o2];
        if 0.0 < t
            && t < best
            && b.min[o1] - 1.0E-7 < p1
            && p1 < b.max[o1] + 1.0E-7
            && b.min[o2] - 1.0E-7 < p2
            && p2 < b.max[o2] + 1.0E-7
        {
            best = t;
            hit = true;
        }
    }
    hit
}

/// `ServerExplosion.interactWithBlocks`: break the blocks in a shuffled
/// order, merge their drops into stacks of up to 16, then drop those.
fn interact_with_blocks(level: &mut Level, mut blocks: Vec<Pos>) {
    // `Util.shuffle`.
    for j in (2..=blocks.len()).rev() {
        let k = level.random.next_int_bounded(j as i32) as usize;
        blocks.swap(j - 1, k);
    }
    let mut drops: Vec<(Pos, item::Stack)> = Vec::new();
    for p in blocks {
        let state = level.get(p);
        if level.is_air(state) {
            continue;
        }
        let kind = level.kind(state);
        if kind != Kind::Tnt {
            for stack in crate::loot::block_drops(level, state) {
                add_or_append(&mut drops, stack, p);
            }
        }
        level.set_block(p, level.air, flags::ALL);
        if kind == Kind::Tnt {
            super::tnt::prime_by_explosion(level, p);
        }
    }
    for (p, stack) in drops {
        item::pop_resource(level, p, stack);
    }
}

/// `ServerExplosion.addOrAppendStack`.
fn add_or_append(drops: &mut Vec<(Pos, item::Stack)>, mut stack: item::Stack, p: Pos) {
    for (_, s) in drops.iter_mut() {
        let max = crate::items::max_stack_size(&s.id);
        if s.id == stack.id && s.count + stack.count <= max {
            let moved = (max.min(16) - s.count).min(stack.count);
            s.count += moved;
            stack.count -= moved;
        }
        if stack.is_empty() {
            return;
        }
    }
    drops.push((p, stack));
}

/// A `java.util.HashSet<BlockPos>`, for its iteration order.
#[derive(Default)]
struct JavaHashSet {
    /// Elements in insertion order, with their spread hashes.
    items: Vec<(Pos, u32)>,
    seen: crate::fxhash::FxHashSet<Pos>,
    /// Nodes per bin at the current capacity.
    bins: crate::fxhash::FxHashMap<usize, usize>,
    capacity: usize,
    treeified: bool,
}

impl JavaHashSet {
    fn bin(&self, spread: u32) -> usize {
        spread as usize & (self.capacity - 1)
    }

    fn resize(&mut self) {
        self.capacity *= 2;
        self.bins.clear();
        for i in 0..self.items.len() {
            let b = self.bin(self.items[i].1);
            *self.bins.entry(b).or_default() += 1;
        }
    }

    fn insert(&mut self, p: Pos) {
        if self.capacity == 0 {
            self.capacity = 16;
        }
        if !self.seen.insert(p) {
            return;
        }
        let h = p.java_hash();
        let spread = (h ^ ((h as u32) >> 16) as i32) as u32;
        self.items.push((p, spread));
        let b = self.bin(spread);
        let n = self.bins.entry(b).or_default();
        *n += 1;
        // `putVal` treeifies a bin that reaches 9 nodes, or resizes instead
        // while the table is under 64.
        if *n >= 9 {
            if self.capacity < 64 {
                self.resize();
            } else {
                self.treeified = true;
            }
        }
        if self.items.len() > self.capacity * 3 / 4 {
            self.resize();
        }
    }

    fn iteration_order(&self) -> Vec<Pos> {
        let mut v: Vec<(usize, usize, Pos)> =
            self.items.iter().enumerate().map(|(i, (p, s))| (self.bin(*s), i, *p)).collect();
        v.sort();
        v.into_iter().map(|(_, _, p)| p).collect()
    }
}
