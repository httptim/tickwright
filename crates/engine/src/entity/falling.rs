//! `FallingBlockEntity`: sand, gravel and other blocks that fall.

use super::{DRAG, Data, Entity, apply_gravity, is_free, item, move_entity, replaceable};
use crate::level::{Level, flags};
use crate::physics::Vec3;
use crate::pos::Pos;
use crate::registry::StateId;

#[derive(Clone, Debug)]
pub struct Falling {
    pub state: StateId,
    pub time: i32,
    pub drop_item: bool,
    pub cancel_drop: bool,
}

/// `FallingBlockEntity.fall`: the block becomes an entity at the bottom
/// centre of its space, and its space becomes what fluid it held.
pub fn fall(level: &mut Level, pos: Pos, state: StateId) {
    let falling = Falling { state, time: 0, drop_item: true, cancel_drop: false };
    let at = Vec3::new(pos.x as f64 + 0.5, pos.y as f64, pos.z as f64 + 0.5);
    let e = Entity::new(level, Data::FallingBlock(falling), at);
    if level.reg.value(state, "waterlogged") == Some("true") {
        level.unsupported("a waterlogged block falling");
    }
    level.set_block(pos, level.air, flags::ALL);
    level.add_entity(e);
}

fn falling(e: &mut Entity) -> &mut Falling {
    match &mut e.data {
        Data::FallingBlock(f) => f,
        _ => unreachable!("not a falling block"),
    }
}

/// `FallingBlockEntity.tick`.
pub fn tick(level: &mut Level, e: &mut Entity) {
    let state = falling(e).state;
    if level.is_air(state) {
        e.discard();
        return;
    }
    falling(e).time += 1;
    apply_gravity(e, 0.04);
    move_entity(level, e, e.motion);
    super::apply_effects_from_blocks(level, e);
    let pos = e.block_position();
    if !e.on_ground {
        let time = falling(e).time;
        if time > 100 && (pos.y < level.min_y() || pos.y > level.max_y()) || time > 600 {
            drop_self(level, e, state);
            e.discard();
        }
    } else {
        let here = level.get(pos);
        e.motion = e.motion.multiply(0.7, -0.5, 0.7);
        if level.kind(here) != crate::blocks::Kind::MovingPiston {
            if falling(e).cancel_drop {
                e.discard();
            } else {
                let can_replace = replaceable(level, here);
                let free = is_free(level, level.get(pos.below()));
                let survives = crate::blocks::placement::state_can_survive(level, pos, state) && !free;
                if can_replace && survives {
                    if level.set_block(pos, state, flags::ALL) {
                        e.discard();
                    } else {
                        e.discard();
                        drop_self(level, e, state);
                    }
                } else {
                    e.discard();
                    drop_self(level, e, state);
                }
            }
        }
    }
    e.motion = e.motion.scale(DRAG);
}

/// `spawnAtLocation(level, block)`, when the entity can't become a block.
fn drop_self(level: &mut Level, e: &Entity, state: StateId) {
    let Data::FallingBlock(f) = &e.data else { return };
    if !f.drop_item {
        return;
    }
    let name = level.reg.name(state).to_string();
    item::spawn_at_location(level, e.pos, item::Stack::new(name.strip_prefix("minecraft:").unwrap_or(&name), 1));
}
