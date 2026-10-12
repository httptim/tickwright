//! `PrimedTnt`.

use super::{DRAG, Data, Entity, apply_gravity, explosion, move_entity};
use crate::game_event::GameEvent;
use crate::level::Level;
use crate::physics::Vec3;
use crate::pos::Pos;

#[derive(Clone, Debug)]
pub struct Tnt {
    pub fuse: i32,
}

/// `new PrimedTnt(level, x, y, z, owner)`: a small hop in a random
/// direction.
pub fn new_primed(level: &mut Level, at: Vec3) -> Entity {
    let d = level.random.next_double() * 6.2831854820251465;
    let mut e = Entity::new(level, Data::Tnt(Tnt { fuse: 80 }), at);
    e.motion = Vec3::new(-d.sin() * 0.02, 0.2f32 as f64, -d.cos() * 0.02);
    e
}

/// `TntBlock.prime`: lit by redstone, fire or a player.
pub fn prime(level: &mut Level, pos: Pos) {
    let e = new_primed(level, Vec3::new(pos.x as f64 + 0.5, pos.y as f64, pos.z as f64 + 0.5));
    level.add_entity(e);
    level.game_event(GameEvent::PrimeFuse, pos);
}

/// `TntBlock.wasExploded`: caught in another explosion, with a short fuse.
pub fn prime_by_explosion(level: &mut Level, pos: Pos) {
    let mut e = new_primed(level, Vec3::new(pos.x as f64 + 0.5, pos.y as f64, pos.z as f64 + 0.5));
    let Data::Tnt(t) = &mut e.data else { unreachable!() };
    let i = t.fuse;
    t.fuse = (level.random.next_int_bounded(i / 4) + i / 8) as i16 as i32;
    level.add_entity(e);
}

/// `PrimedTnt.tick`.
pub fn tick(level: &mut Level, e: &mut Entity) {
    apply_gravity(e, 0.04);
    let old = e.pos;
    move_entity(level, e, e.motion);
    super::apply_effects_from_blocks(level, e, old);
    e.motion = e.motion.scale(DRAG);
    if e.on_ground {
        e.motion = e.motion.multiply(0.7, 1.0, 0.7);
        if e.motion.y < 0.0 {
            e.motion = e.motion.multiply(1.0, -0.5, 1.0);
        }
    }
    let Data::Tnt(t) = &mut e.data else { unreachable!() };
    t.fuse -= 1;
    if t.fuse <= 0 {
        e.discard();
        let center = Vec3::new(e.pos.x, e.pos.y + (0.98f32 as f64) * 0.0625, e.pos.z);
        explosion::explode(level, Some(e), center, 4.0, false);
    }
}
