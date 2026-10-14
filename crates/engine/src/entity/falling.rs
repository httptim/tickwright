//! `FallingBlockEntity`: sand, gravel and other blocks that fall.

use super::{DRAG, Data, Entity, apply_gravity, is_free, item, move_entity, replaceable, snbt};
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

/// The block a falling block's saved `BlockState` names, as
/// `FallingBlockEntity.readAdditionalSaveData` reads it with
/// `BlockState.CODEC`: a block's id alone (`"minecraft:gravel"`) for its
/// default state, or a compound of `id` and `properties`. A property the
/// block has, given as one of its values in a string, is set; any other is
/// left at its default. What the codec can't read (no `id`, a block that
/// doesn't exist, or the `Name` and `Properties` that 26.3 no longer reads,
/// since data version 5006) leaves the entity sand.
pub(crate) fn saved_state(level: &Level, tag: Option<&snbt::Tag>) -> StateId {
    let sand = level.default_state(level.block_id("sand"));
    let (id, properties) = match tag {
        Some(snbt::Tag::String(id)) => (id.as_str(), None),
        Some(t @ snbt::Tag::Compound(_)) => match t.get("id").and_then(snbt::Tag::string) {
            Some(id) => (id, t.get("properties")),
            None => return sand,
        },
        _ => return sand,
    };
    let Some(block) = level.reg.block_id(id) else { return sand };
    let info = level.reg.block(block);
    let mut state = info.default;
    if let Some(given) = properties {
        for p in &info.properties {
            if let Some(v) = given.get(&p.name).and_then(snbt::Tag::string)
                && p.values.iter().any(|value| value == v)
            {
                state = level.reg.with(state, &p.name, v);
            }
        }
    }
    state
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::Rules;

    #[test]
    fn saved_states_read_as_block_state_codec_reads_them() {
        let level = Level::new(Rules::for_version("26.3").unwrap());
        let read = |text: &str| {
            let tag = snbt::parse(text).unwrap();
            level.reg.format(saved_state(&level, tag.get("BlockState")))
        };
        assert_eq!(read(r#"{BlockState:{id:"minecraft:gravel"}}"#), "minecraft:gravel");
        assert_eq!(read(r#"{BlockState:{id:"red_sand"}}"#), "minecraft:red_sand");
        assert_eq!(read(r#"{BlockState:"minecraft:red_sand"}"#), "minecraft:red_sand");
        assert_eq!(
            read(r#"{BlockState:{id:"minecraft:anvil",properties:{facing:"east",bogus:"x"}}}"#),
            "minecraft:anvil[facing=east]"
        );
        // A value the property doesn't have stays the default.
        assert_eq!(
            read(r#"{BlockState:{id:"minecraft:anvil",properties:{facing:"up"}}}"#),
            "minecraft:anvil[facing=north]"
        );
        // 26.3 no longer reads `Name` and `Properties`: the entity is sand.
        assert_eq!(read(r#"{BlockState:{Name:"minecraft:gravel"}}"#), "minecraft:sand");
        assert_eq!(read(r#"{BlockState:{Name:"minecraft:gravel",id:"minecraft:red_sand"}}"#), "minecraft:red_sand");
        assert_eq!(read(r#"{BlockState:{id:"minecraft:no_such_block"}}"#), "minecraft:sand");
        assert_eq!(read(r#"{BlockState:{}}"#), "minecraft:sand");
        assert_eq!(read(r#"{BlockState:[]}"#), "minecraft:sand");
        assert_eq!(read("{}"), "minecraft:sand");
    }
}
