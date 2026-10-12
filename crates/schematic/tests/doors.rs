//! Community doors imported from their `.litematic` files run exactly as
//! the same builds did in the game.
//!
//! Each file is turned into a fixture, given the stimuli of the matching
//! Nucleation fixture (whose blocks were captured from the game), and run in
//! every orientation; the trace must match the committed ground truth at
//! the order level.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use tickwright_schematic::Schematic;
use tickwright_trace::{FixtureSource, Level, Trace, compare};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn check(file: &str, fixture: &str) {
    let here = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data").join(file);
    let schematic = Schematic::read(&fs::read(here).unwrap()).unwrap();
    let game_fixture = FixtureSource::from_json(
        &fs::read_to_string(root().join("fixtures/nucleation").join(format!("{fixture}.json"))).unwrap(),
    )
    .unwrap();
    let mut ours = schematic.to_fixture(&game_fixture.name, game_fixture.ticks).unwrap();
    ours.stimuli = game_fixture.stimuli.clone();

    let mut checked = 0;
    for e in fs::read_dir(root().join("traces/26.3")).unwrap() {
        let p = e.unwrap().path();
        let n = p.file_name().unwrap().to_string_lossy().into_owned();
        let Some(transform) =
            n.strip_prefix(&format!("{}@", game_fixture.name)).and_then(|t| t.strip_suffix(".json.gz"))
        else {
            continue;
        };
        let mut text = String::new();
        flate2::read::GzDecoder::new(fs::File::open(&p).unwrap()).read_to_string(&mut text).unwrap();
        let game = Trace::from_json(&text).unwrap();
        let resolved = ours.resolve(transform.parse().unwrap()).unwrap();
        let trace = tickwright_engine::run::run(&resolved, "26.3").unwrap();
        if let Err(d) = compare(&game, &trace, Level::Order) {
            panic!("{file} @{transform}: {d}");
        }
        checked += 1;
    }
    assert_eq!(checked, 8, "{fixture}: expected eight orientations of ground truth");
}

#[test]
fn sliding_door_4x4() {
    check("4x4_sliding_door.litematic", "door_4x4_sliding_cycle");
}

#[test]
fn sliding_door_6x6() {
    check("6x6_sliding_door.litematic", "door_6x6_cycle");
}

#[test]
fn exports_round_trip_in_every_format() {
    use tickwright_schematic::Format;
    use tickwright_trace::Transform;
    let text = fs::read_to_string(root().join("fixtures/nucleation/door_4x4_vault_inworld.json")).unwrap();
    let fixture = FixtureSource::from_json(&text).unwrap().resolve(Transform::IDENTITY).unwrap();
    let s = Schematic::from_fixture(&fixture).unwrap();
    for f in [Format::Litematic, Format::Sponge, Format::Structure] {
        let back = Schematic::read(&s.write(f)).unwrap();
        let again = back.to_fixture(&fixture.name, fixture.ticks).unwrap().resolve(Transform::IDENTITY).unwrap();
        assert_eq!(again.blocks, fixture.blocks, "{f:?}");
        assert_eq!(again.containers, fixture.containers, "{f:?}");
        assert_eq!(again.comparators, fixture.comparators, "{f:?}");
    }
}
