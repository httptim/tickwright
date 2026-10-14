//! A structure 26.3 saved itself imports as what was built.
//!
//! `tests/data/game_saved_26.3.nbt` comes from the game:
//! `harness/schematics.sh` builds a small build in a 26.3 level and saves it
//! with `StructureTemplate.save`, and fails if a fresh save imports
//! differently from this copy. 26.3 saves block states as `id` and
//! `properties`, and a block's default state alone as its id.

use std::path::Path;

use tickwright_engine::entity::place_start_entity;
use tickwright_engine::level::Level;
use tickwright_engine::physics::Vec3;
use tickwright_engine::rules::Rules;
use tickwright_schematic::{Format, Schematic, nbt::Tag, snbt};
use tickwright_trace::BlockState;

fn saved() -> Schematic {
    let file = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/game_saved_26.3.nbt");
    Schematic::read(&std::fs::read(file).unwrap()).unwrap()
}

#[test]
fn blocks_and_block_entities_import() {
    let s = saved();
    assert_eq!(s.data_version, Some(5023));
    assert_eq!(s.size, [3, 3, 2]);
    let b = |t: &str| BlockState::parse(t).unwrap();
    assert_eq!(
        s.blocks,
        [
            ([0, 0, 0], b("stone")),
            ([1, 0, 0], b("repeater[delay=2,facing=east,locked=false,powered=false]")),
            ([2, 0, 0], b("comparator[facing=west,mode=subtract,powered=true]")),
            ([0, 0, 1], b("chest[facing=north,type=single,waterlogged=false]")),
            ([1, 0, 1], b("moving_piston[facing=up,type=sticky]")),
            ([2, 0, 1], b("gravel")),
        ]
    );
    let containers = s.containers();
    assert_eq!(
        (containers[0].pos, containers[0].items[0].id.as_str(), containers[0].items[0].count),
        ([0, 0, 1], "redstone", 64)
    );
    assert_eq!(s.comparators()[0].output, 5);
    let piston = s.block_entities.iter().find(|be| be.pos == [1, 0, 1]).unwrap();
    assert_eq!(
        snbt::write(piston.data.get("blockState").unwrap()),
        r#"{id:"minecraft:redstone_lamp",properties:{lit:"true"}}"#
    );
}

#[test]
fn entities_keep_the_block_states_the_game_saved() {
    let s = saved();
    let states: Vec<String> = s
        .entities
        .iter()
        .map(|e| {
            let state = e.data.get("BlockState").or_else(|| e.data.get("DisplayState")).unwrap();
            format!("{} {}", e.id, snbt::write(state))
        })
        .collect();
    assert_eq!(
        states,
        [
            r#"minecraft:falling_block "minecraft:gravel""#,
            r#"minecraft:falling_block {id:"minecraft:anvil",properties:{facing:"east"}}"#,
            r#"minecraft:minecart {id:"minecraft:redstone_lamp",properties:{lit:"true"}}"#,
            r#"minecraft:falling_block "minecraft:red_sand""#,
        ]
    );

    // The engine reads them as the game does.
    let mut level = Level::new(Rules::for_version("26.3").unwrap());
    let fixture = s.to_fixture("game_saved", 1).unwrap();
    let mut refused = Vec::new();
    for e in &fixture.start_entities {
        let at = Vec3::new(e.at[0], e.at[1], e.at[2]);
        if let Err(err) = place_start_entity(&mut level, &e.entity, at, None, e.nbt.as_deref()) {
            refused.push(err.to_string());
        }
    }
    let details: Vec<String> = level.entities.iter().filter_map(|e| e.detail(&level)).collect();
    assert_eq!(details, ["minecraft:gravel", "minecraft:red_sand"]);
    // A falling anvil isn't simulated, and is refused as an anvil, not
    // read as sand.
    assert_eq!(refused, ["a falling minecraft:anvil is not supported by the engine yet"]);
}

#[test]
fn written_again_it_keeps_the_games_keys() {
    let s = saved();
    let (_, root) = tickwright_schematic::nbt::read(&s.write(Format::Structure)).unwrap();
    let palette = root.get("palette").unwrap().list();
    assert!(palette.iter().all(|p| p.get("id").is_some() && p.get("Name").is_none()));
    let entities = root.get("entities").unwrap().list();
    let anvil = entities[1].get("nbt").unwrap().get("BlockState").unwrap();
    assert_eq!(anvil.get("id"), Some(&Tag::String("minecraft:anvil".into())));
    assert_eq!(Schematic::read(&s.write(Format::Structure)).unwrap(), s);
}
