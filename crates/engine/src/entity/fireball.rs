//! Small fireballs (`SmallFireball`). They speed up along their motion
//! until a ray along it meets a block, then light fire in front of the
//! face they hit and are gone. A target block they hit lights up for 8
//! ticks. Fireballs in water and fireballs hitting entities are refused.

use super::arrow::{clip, hits_an_entity};
use super::{Entity, apply_effects_from_blocks, base_tick};
use crate::blocks::{Kind, fire, plain};
use crate::level::{Level, flags};

/// `AbstractHurtingProjectile.accelerationPower`'s default.
const ACCELERATION: f64 = 0.1;
/// `getInertia`, out of water.
const INERTIA: f64 = 0.95f32 as f64;

/// `AbstractHurtingProjectile.tick`. The motion is sped up and slowed
/// before the move, which uses the new motion (seen in 26.3 captures).
pub fn tick(level: &mut Level, e: &mut Entity) {
    base_tick(level, e);
    if e.was_touching_water {
        level.unsupported("a fireball in water");
        return;
    }
    let m = e.motion;
    e.motion = (m + m.normalize().scale(ACCELERATION)).scale(INERTIA);
    let from = e.pos;
    let to = from + e.motion;
    let hit = clip(level, from, to);
    if level.unsupported.is_some() {
        return;
    }
    if hits_an_entity(level, e, from, hit.as_ref().map_or(to, |h| h.at)) {
        level.unsupported("a fireball hitting an entity");
        return;
    }
    e.pos = hit.as_ref().map_or(to, |h| h.at);
    apply_effects_from_blocks(level, e);
    let Some(h) = hit else { return };
    if e.removed {
        return;
    }
    // `Projectile.onHitBlock`, then `SmallFireball.onHitBlock`.
    let state = level.get(h.pos);
    match level.kind(state) {
        Kind::Target => plain::target_hit(level, h.pos, state, [h.at.x, h.at.y, h.at.z], h.face, false),
        Kind::Bell | Kind::Tnt => level.unsupported("a fireball hitting a bell or TNT"),
        _ => {}
    }
    let front = h.pos.offset(h.face);
    if level.is_air(level.get(front)) {
        let fire = fire::state_at(level, front);
        level.set_block(front, fire, flags::ALL);
    }
    // `Fireball.onHit`.
    e.discard();
}
