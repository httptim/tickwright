//! Small fireballs (`SmallFireball`), only for as long as they stay still.
//! A summoned one without motion never accelerates, hits nothing and sets
//! nothing on fire, but pistons push it and it presses pressure plates. A
//! moving one would need projectile hits and fire, which the engine doesn't
//! have.

use super::{Entity, apply_effects_from_blocks, base_tick};
use crate::level::Level;
use crate::physics::Vec3;

/// `AbstractHurtingProjectile.tick`, for a fireball that isn't moving.
pub fn tick(level: &mut Level, e: &mut Entity) {
    if e.motion != Vec3::ZERO {
        level.unsupported("a moving fireball");
        return;
    }
    base_tick(level, e);
    let old = e.pos;
    apply_effects_from_blocks(level, e, old);
}
