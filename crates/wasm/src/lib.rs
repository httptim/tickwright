//! The engine as a WebAssembly module with a plain C ABI, so a page can use
//! it with nothing but `WebAssembly.instantiate`.
//!
//! The host copies a resolved fixture's JSON into memory from [`alloc`],
//! calls [`run`], and reads the result: a little-endian `u32` length, a
//! status byte (0 for a trace, 1 for an error message), then that many bytes
//! of UTF-8. It hands both buffers back with [`dealloc`].
//!
//! [`session`] drives an interactive simulation for the editor the same way:
//! the host writes a JSON command and reads back a JSON answer (see
//! [`session`] for the commands).

use std::cell::RefCell;
use std::collections::HashMap;

use serde::Deserialize;
use serde_json::{Value, json};
use tickwright_engine::Pos;
use tickwright_engine::session::{Placement, Session};
use tickwright_trace::ResolvedFixture;

/// Allocate `len` bytes for the host to write into.
#[unsafe(no_mangle)]
pub extern "C" fn alloc(len: usize) -> *mut u8 {
    let mut buf = Vec::<u8>::with_capacity(len);
    let ptr = buf.as_mut_ptr();
    std::mem::forget(buf);
    ptr
}

/// Free a buffer from [`alloc`] or [`run`].
///
/// # Safety
/// `ptr` and `len` must describe a buffer this module handed out.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn dealloc(ptr: *mut u8, len: usize) {
    drop(unsafe { Vec::from_raw_parts(ptr, 0, len) });
}

/// Run the resolved fixture in `ptr[..len]` with the rules of `version`
/// (`version_ptr[..version_len]`) and return the result buffer.
///
/// # Safety
/// Both ranges must be initialised memory inside this module's heap.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn run(ptr: *const u8, len: usize, version_ptr: *const u8, version_len: usize) -> *mut u8 {
    let input = unsafe { std::slice::from_raw_parts(ptr, len) };
    let version = unsafe { std::slice::from_raw_parts(version_ptr, version_len) };
    respond(simulate(input, version))
}

/// The result buffer: length, status byte, then the body.
fn respond(result: anyhow::Result<String>) -> *mut u8 {
    let (status, body) = match result {
        Ok(json) => (0u8, json),
        Err(e) => (1u8, format!("{e:#}")),
    };
    let mut out = Vec::with_capacity(5 + body.len());
    out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    out.push(status);
    out.extend_from_slice(body.as_bytes());
    let ptr = out.as_mut_ptr();
    std::mem::forget(out);
    ptr
}

/// Run the resolved fixture without recording, for benchmarks. Returns 0 on
/// success and 1 if the fixture could not run.
///
/// # Safety
/// As for [`run`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bench(ptr: *const u8, len: usize, version_ptr: *const u8, version_len: usize) -> u32 {
    let input = unsafe { std::slice::from_raw_parts(ptr, len) };
    let version = unsafe { std::slice::from_raw_parts(version_ptr, version_len) };
    let go = || -> anyhow::Result<()> {
        let fixture = serde_json::from_str::<ResolvedFixture>(std::str::from_utf8(input)?)?;
        tickwright_engine::run::run_unrecorded(&fixture, std::str::from_utf8(version)?)
    };
    u32::from(go().is_err())
}

fn simulate(input: &[u8], version: &[u8]) -> anyhow::Result<String> {
    let fixture = serde_json::from_str::<ResolvedFixture>(std::str::from_utf8(input)?)?;
    let trace = tickwright_engine::run::run(&fixture, std::str::from_utf8(version)?)?;
    Ok(serde_json::to_string(&trace)? + "\n")
}

thread_local! {
    static SESSIONS: RefCell<(u32, HashMap<u32, Session>)> = RefCell::new((0, HashMap::new()));
}

#[derive(Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
enum Command {
    /// Start a session: an empty world, or a resolved fixture placed as the
    /// harness places it. Answers `{"session": id}`.
    Open {
        version: String,
        fixture: Option<Box<ResolvedFixture>>,
    },
    Close {
        session: u32,
    },
    /// Write a block; `updates` (default false) sends neighbor updates.
    Set {
        session: u32,
        pos: [i32; 3],
        state: String,
        #[serde(default)]
        updates: bool,
    },
    Destroy {
        session: u32,
        pos: [i32; 3],
    },
    /// Right-click with an empty hand.
    Use {
        session: u32,
        pos: [i32; 3],
    },
    Item {
        session: u32,
        pos: [i32; 3],
        slot: usize,
        id: String,
        count: i32,
    },
    /// Run `count` ticks (default 1). Answers the last tick and every event
    /// of the ticks run, so the host can redraw what changed.
    Step {
        session: u32,
        #[serde(default = "one")]
        count: u32,
    },
    Rewind {
        session: u32,
        tick: u32,
    },
    Events {
        session: u32,
        tick: u32,
    },
    /// Every non-air block as `[x, y, z, state]`.
    Blocks {
        session: u32,
    },
    Block {
        session: u32,
        pos: [i32; 3],
    },
    Power {
        session: u32,
        pos: [i32; 3],
    },
    Pending {
        session: u32,
    },
    Items {
        session: u32,
        pos: [i32; 3],
    },
}

fn one() -> u32 {
    1
}

fn pos(p: [i32; 3]) -> Pos {
    Pos::new(p[0], p[1], p[2])
}

/// Run one editor command (JSON in `ptr[..len]`) and return the result
/// buffer, whose body is the JSON answer or an error message.
///
/// # Safety
/// The range must be initialised memory inside this module's heap.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn session(ptr: *const u8, len: usize) -> *mut u8 {
    let input = unsafe { std::slice::from_raw_parts(ptr, len) };
    respond(command(input).map(|v| v.to_string()))
}

fn get(sessions: &mut HashMap<u32, Session>, id: u32) -> anyhow::Result<&mut Session> {
    sessions.get_mut(&id).ok_or_else(|| anyhow::anyhow!("no session {id}"))
}

fn command(input: &[u8]) -> anyhow::Result<Value> {
    let cmd: Command = serde_json::from_slice(input)?;
    SESSIONS.with(|cell| {
        let (next, sessions) = &mut *cell.borrow_mut();
        Ok(match cmd {
            Command::Open { version, fixture } => {
                let s = match fixture {
                    Some(f) => Session::from_fixture(&f, &version)?,
                    None => Session::new(&version)?,
                };
                *next += 1;
                sessions.insert(*next, s);
                json!({ "session": *next })
            }
            Command::Close { session } => {
                sessions.remove(&session);
                json!({})
            }
            Command::Set { session, pos: p, state, updates } => {
                let placement = if updates { Placement::Updates } else { Placement::Quiet };
                get(sessions, session)?.set_block(pos(p), &state, placement)?;
                json!({})
            }
            Command::Destroy { session, pos: p } => {
                get(sessions, session)?.destroy(pos(p));
                json!({})
            }
            Command::Use { session, pos: p } => {
                get(sessions, session)?.use_block(pos(p));
                json!({})
            }
            Command::Item { session, pos: p, slot, id, count } => {
                get(sessions, session)?.set_item(pos(p), slot, &id, count)?;
                json!({})
            }
            Command::Step { session, count } => {
                let s = get(sessions, session)?;
                let mut events = Vec::new();
                for _ in 0..count {
                    events.extend_from_slice(s.step());
                }
                json!({ "tick": s.tick(), "events": events })
            }
            Command::Rewind { session, tick } => {
                get(sessions, session)?.rewind(tick)?;
                json!({})
            }
            Command::Events { session, tick } => json!({ "events": get(sessions, session)?.events(tick) }),
            Command::Blocks { session } => {
                let s = get(sessions, session)?;
                let blocks: Vec<Value> = s.blocks().into_iter().map(|(p, st)| json!([p.x, p.y, p.z, st])).collect();
                json!({ "tick": s.tick(), "blocks": blocks })
            }
            Command::Block { session, pos: p } => json!({ "state": get(sessions, session)?.block(pos(p)) }),
            Command::Power { session, pos: p } => {
                let power = get(sessions, session)?.power(pos(p));
                json!({ "weak": power.weak, "strong": power.strong })
            }
            Command::Pending { session } => {
                let ticks: Vec<Value> = get(sessions, session)?
                    .pending_ticks()
                    .into_iter()
                    .map(|t| json!({ "pos": [t.pos.x, t.pos.y, t.pos.z], "block": t.block, "due": t.due, "priority": t.priority }))
                    .collect();
                json!({ "ticks": ticks })
            }
            Command::Items { session, pos: p } => json!({ "items": get(sessions, session)?.items(pos(p)) }),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::command;

    fn ok(cmd: &str) -> serde_json::Value {
        command(cmd.as_bytes()).unwrap()
    }

    #[test]
    fn a_session_runs_a_lever_and_lamp() {
        let id = ok(r#"{"op":"open","version":"26.3"}"#)["session"].as_u64().unwrap();
        ok(&format!(r#"{{"op":"set","session":{id},"pos":[0,0,0],"state":"stone"}}"#));
        ok(&format!(
            r#"{{"op":"set","session":{id},"pos":[0,1,0],"state":"lever[face=floor,facing=north,powered=false]"}}"#
        ));
        ok(&format!(r#"{{"op":"set","session":{id},"pos":[1,1,0],"state":"redstone_lamp[lit=false]"}}"#));
        ok(&format!(r#"{{"op":"use","session":{id},"pos":[0,1,0]}}"#));
        let step = ok(&format!(r#"{{"op":"step","session":{id}}}"#));
        assert_eq!(step["tick"], 1);
        let lamp = ok(&format!(r#"{{"op":"block","session":{id},"pos":[1,1,0]}}"#));
        assert_eq!(lamp["state"], "minecraft:redstone_lamp[lit=true]");
        assert!(command(br#"{"op":"step","session":999}"#).is_err());
    }
}
