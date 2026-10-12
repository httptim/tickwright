//! How water moves entities: `Entity.updateFluidHeightAndDoFluidPushing`
//! and the flow it reads, `FlowingFluid.getFlow`.

use super::{Entity, floor};
use crate::blocks::water::{self, FluidState};
use crate::level::Level;
use crate::physics::Vec3;
use crate::pos::{Dir, Pos};

/// `FluidState.getHeight`: full when more water sits on top.
fn height(level: &Level, pos: Pos, fs: FluidState) -> f32 {
    if !water::fluid_state(level, level.get(pos.above())).is_empty() { 1.0 } else { fs.own_height() }
}

/// `FlowingFluid.isSolidFace`.
fn is_solid_face(level: &Level, pos: Pos, dir: Dir) -> bool {
    let state = level.get(pos);
    if !water::fluid_state(level, state).is_empty() {
        return false;
    }
    if dir == Dir::Up {
        return true;
    }
    let name = level.reg.name(state);
    if name == "minecraft:ice" || name == "minecraft:frosted_ice" {
        return false;
    }
    crate::blocks::is_face_sturdy(level, pos, state, dir)
}

/// `FlowingFluid.getFlow`: which way water at `pos` runs, as a unit vector.
pub fn flow(level: &Level, pos: Pos, fs: FluidState) -> Vec3 {
    let (mut dx, mut dz) = (0.0f64, 0.0f64);
    for dir in Dir::HORIZONTAL {
        let n = pos.offset(dir);
        let nfs = water::fluid_state(level, level.get(n));
        // `affectsFlow`: empty, or water.
        let mut f = nfs.own_height();
        let mut f1 = 0.0f32;
        if f == 0.0 {
            if !water::is_solid(level, level.get(n)) {
                let below = water::fluid_state(level, level.get(n.below()));
                f = below.own_height();
                if f > 0.0 {
                    f1 = fs.own_height() - (f - 0.8888889);
                }
            }
        } else if f > 0.0 {
            f1 = fs.own_height() - f;
        }
        if f1 != 0.0 {
            let [sx, _, sz] = dir.step();
            dx += (sx as f32 * f1) as f64;
            dz += (sz as f32 * f1) as f64;
        }
    }
    let mut v = Vec3::new(dx, 0.0, dz);
    if fs.is_falling() {
        for dir in Dir::HORIZONTAL {
            let n = pos.offset(dir);
            if is_solid_face(level, n, dir) || is_solid_face(level, n.above(), dir) {
                v = v.normalize() + Vec3::new(0.0, -6.0, 0.0);
                break;
            }
        }
    }
    v.normalize()
}

/// `Entity.updateInWaterStateAndDoWaterCurrentPushing`, with the push of
/// `updateFluidHeightAndDoFluidPushing(WATER, 0.014)`.
pub fn update_in_water(level: &Level, e: &mut Entity) {
    let b = e.bounding_box().deflate(0.001);
    let (x0, x1) = (floor(b.min[0]), ceil(b.max[0]));
    let (y0, y1) = (floor(b.min[1]), ceil(b.max[1]));
    let (z0, z1) = (floor(b.min[2]), ceil(b.max[2]));
    let mut d0 = 0.0f64;
    let mut found = false;
    let mut v = Vec3::ZERO;
    let mut k = 0;
    for x in x0..x1 {
        for y in y0..y1 {
            for z in z0..z1 {
                let p = Pos::new(x, y, z);
                let fs = water::fluid_state(level, level.get(p));
                if fs.is_empty() {
                    continue;
                }
                let h = y as f64 + height(level, p, fs) as f64;
                if h >= b.min[1] {
                    found = true;
                    // Fitted from 26.3 captures: depth counts from the
                    // entity's feet, not the shrunken box.
                    d0 = d0.max(h - e.pos.y);
                    let mut f = flow(level, p, fs);
                    if d0 < 0.4 {
                        f = f.scale(d0);
                    }
                    v = v + f;
                    k += 1;
                }
            }
        }
    }
    if v.length() > 0.0 {
        if k > 0 {
            v = v.scale(1.0 / k as f64);
        }
        v = v.normalize();
        let m = e.motion;
        v = v.scale(0.014);
        if m.x.abs() < 0.003 && m.z.abs() < 0.003 && v.length() < 0.0045000000000000005 {
            v = v.normalize().scale(0.0045000000000000005);
        }
        e.motion = m + v;
    }
    e.fluid_height = d0;
    e.was_touching_water = found;
}

/// `Mth.ceil`.
fn ceil(v: f64) -> i32 {
    let i = v as i32;
    if v > i as f64 { i + 1 } else { i }
}
