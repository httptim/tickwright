//! Runs every fixture that has a ground-truth trace from the game and
//! compares the engine's trace with it.
//!
//! `tests/expected.txt` lists the fixtures the engine must match, one
//! `name level` per line; a fixture listed there must match in all eight
//! orientations. Fixtures not listed are run and reported but do not fail
//! the test, so the list can only grow as the engine improves.
//!
//! `TICKWRIGHT_SHOW=<fixture id>` prints the first divergence for one
//! orientation in full.

use std::collections::BTreeMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use tickwright_trace::{FixtureSource, Level, Trace, compare};

const VERSION: &str = "26.3";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn fixtures() -> BTreeMap<String, FixtureSource> {
    let mut out = BTreeMap::new();
    let mut dirs = vec![root().join("fixtures")];
    while let Some(d) = dirs.pop() {
        for e in fs::read_dir(d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                dirs.push(p);
            } else if p.extension().is_some_and(|x| x == "json") {
                let f = FixtureSource::from_json(&fs::read_to_string(&p).unwrap()).unwrap();
                out.insert(f.name.clone(), f);
            }
        }
    }
    out
}

fn load_trace(path: &Path) -> Trace {
    let mut text = String::new();
    flate2::read::GzDecoder::new(fs::File::open(path).unwrap()).read_to_string(&mut text).unwrap();
    Trace::from_json(&text).unwrap()
}

fn level_name(l: Level) -> &'static str {
    match l {
        Level::State => "state",
        Level::Phase => "phase",
        Level::Order => "order",
    }
}

#[test]
fn engine_matches_the_game() {
    let fixtures = fixtures();
    let mut traces: Vec<PathBuf> = fs::read_dir(root().join("traces").join(VERSION))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.to_string_lossy().ends_with(".json.gz"))
        .collect();
    traces.sort();
    assert!(!traces.is_empty(), "no ground-truth traces");

    let expected: BTreeMap<String, Level> =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/expected.txt"))
            .unwrap()
            .lines()
            .map(|l| l.trim())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .map(|l| {
                let (name, level) = l.split_once(' ').expect("`name level`");
                let level = match level.trim() {
                    "state" => Level::State,
                    "phase" => Level::Phase,
                    "order" => Level::Order,
                    other => panic!("unknown level {other}"),
                };
                (name.to_string(), level)
            })
            .collect();

    let show = std::env::var("TICKWRIGHT_SHOW").ok();
    // `TICKWRIGHT_ONLY=<substring>` runs only the fixtures whose name contains it.
    let only = std::env::var("TICKWRIGHT_ONLY").ok();
    // fixture name -> per orientation, the highest level matched (None = not even state).
    let mut results: BTreeMap<String, Vec<(String, Option<Level>)>> = BTreeMap::new();
    let mut unsupported: BTreeMap<String, String> = BTreeMap::new();
    for path in &traces {
        let game = load_trace(path);
        let (name, transform) = game.fixture.split_once('@').unwrap();
        if only.as_deref().is_some_and(|o| !game.fixture.contains(o)) {
            continue;
        }
        let fixture = fixtures.get(name).unwrap_or_else(|| panic!("no fixture {name}"));
        let resolved = fixture.resolve(transform.parse().unwrap()).unwrap();
        // Builds with blocks the engine doesn't simulate yet are listed as
        // unsupported; they are ground truth for later phases.
        let ours = match tickwright_engine::run::run(&resolved, VERSION) {
            Ok(trace) => trace,
            Err(e) => {
                unsupported.entry(name.to_string()).or_insert_with(|| format!("{e:#}"));
                continue;
            }
        };
        let mut best = None;
        let mut first_failure = None;
        for level in [Level::State, Level::Phase, Level::Order] {
            match compare(&game, &ours, level) {
                Ok(()) => best = Some(level),
                Err(d) => {
                    first_failure = Some(d);
                    break;
                }
            }
        }
        if show.as_deref().is_some_and(|s| s == game.fixture || s == name) {
            match &first_failure {
                Some(d) => println!("{}: {d}", game.fixture),
                None => println!("{}: matches at every level", game.fixture),
            }
        }
        results.entry(name.to_string()).or_default().push((transform.to_string(), best));
    }

    let mut failures = Vec::new();
    println!("\n{:<36} {:>6} {:>6} {:>6}   (orientations matching, of 8)", "fixture", "state", "phase", "order");
    let mut totals = [0; 3];
    let mut count = 0;
    for (name, runs) in &results {
        let at = |l: Level| runs.iter().filter(|(_, b)| b.is_some_and(|b| b >= l)).count();
        let row = [at(Level::State), at(Level::Phase), at(Level::Order)];
        for i in 0..3 {
            totals[i] += row[i];
        }
        count += runs.len();
        println!("{name:<36} {:>6} {:>6} {:>6}", row[0], row[1], row[2]);
        if let Some(&level) = expected.get(name) {
            for (t, b) in runs {
                if !b.is_some_and(|b| b >= level) {
                    failures.push(format!("{name}@{t} no longer matches at the {} level", level_name(level)));
                }
            }
        }
    }
    println!("{:<36} {:>6} {:>6} {:>6}   of {count}", "total", totals[0], totals[1], totals[2]);
    if !unsupported.is_empty() {
        println!("\nnot simulated yet:");
        for (name, why) in &unsupported {
            let why = why.split_once(": ").map_or(why.as_str(), |(_, w)| w);
            println!("  {name:<36} {why}");
            if expected.contains_key(name) {
                failures.push(format!("{name}: {why}"));
            }
        }
    }
    assert!(failures.is_empty(), "regressions:\n{}", failures.join("\n"));
}
