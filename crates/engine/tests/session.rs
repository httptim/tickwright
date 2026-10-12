//! The editor's session must simulate exactly what a fixture run does:
//! driving every fixture through [`Session`] by hand, the events inside the
//! fixture's box must equal the ones `run::run` records.

use std::fs;
use std::path::{Path, PathBuf};

use tickwright_engine::Pos;
use tickwright_engine::session::{Placement, Session};
use tickwright_trace::{Action, FixtureSource, Transform};

const VERSION: &str = "26.3";

fn fixture_files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut dirs = vec![Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")];
    while let Some(d) = dirs.pop() {
        for e in fs::read_dir(d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                dirs.push(p);
            } else if p.extension().is_some_and(|x| x == "json") {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn session_matches_fixture_runs() {
    let mut checked = 0;
    for path in fixture_files() {
        let source = FixtureSource::from_json(&fs::read_to_string(&path).unwrap()).unwrap();
        let fixture = source.resolve(Transform::IDENTITY).unwrap();
        let Ok(trace) = tickwright_engine::run::run(&fixture, VERSION) else {
            continue; // uses blocks the engine refuses
        };
        let [ox, oy, oz] = fixture.origin;
        let m = fixture.margin;
        let [sx, sy, sz] = fixture.size;
        let inside = |p: [i32; 3]| {
            (-m..sx + m).contains(&(p[0] - ox))
                && (-m..sy + m).contains(&(p[1] - oy))
                && (-m..sz + m).contains(&(p[2] - oz))
        };

        let mut session = Session::from_fixture(&fixture, VERSION).unwrap();
        let mut entries = Vec::new();
        for t in 1..=fixture.ticks {
            for s in fixture.stimuli.iter().filter(|s| s.tick == t) {
                let pos = Pos::new(ox + s.pos[0], oy + s.pos[1], oz + s.pos[2]);
                match s.action {
                    Action::Use => session.use_block(pos),
                    Action::Set => {
                        let how = if s.quiet { Placement::Quiet } else { Placement::Updates };
                        session.set_block(pos, s.state.as_deref().unwrap(), how).unwrap()
                    }
                    Action::Destroy => session.destroy(pos),
                    Action::Place => {
                        let p = s.place.as_ref().unwrap();
                        session.place(&p.item, tickwright_engine::run::click(pos, p).unwrap()).unwrap();
                    }
                    Action::Page => session.turn_page(pos, s.page.unwrap() as i32),
                    Action::Time => session.set_day_time(s.time.unwrap()),
                }
            }
            for e in session.step().iter().filter(|e| inside(e.pos)) {
                let mut e = e.clone();
                e.pos = [e.pos[0] - ox, e.pos[1] - oy, e.pos[2] - oz];
                entries.push(e);
            }
        }
        assert!(entries == trace.entries, "{}: the session's events differ from the fixture run's", fixture.id);
        checked += 1;
    }
    assert!(checked > 100, "only {checked} fixtures ran");
}
