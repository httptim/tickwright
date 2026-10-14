//! `JukeboxBlock`, `JukeboxBlockEntity` and its `JukeboxSongPlayer`.
//!
//! A jukebox is a one-slot container that takes only music discs, from a
//! hopper or dropper (`canPlaceItem`) and gives its disc to a hopper below
//! that has an empty slot (`canTakeItem`). While a disc plays the jukebox
//! gives a redstone signal of 15 to every side; a comparator reads the
//! disc's own value whether or not it still plays. Its block entity ticks
//! only while `has_record` is set, counting the song's ticks, and every 20
//! of them it draws from the level's random source for the note particle's
//! colour. A song stops a second after its length.
//!
//! Removing a jukebox that holds a disc is refused: the game pops the disc
//! out and updates the jukebox's neighbours while its block is already gone,
//! which the engine does not do.

use crate::game_event::GameEvent;
use crate::items::{self, JukeboxSong};
use crate::level::{Level, flags};
use crate::physics::Vec3;
use crate::pos::Pos;
use crate::registry::StateId;

use super::BlockEntity;
use super::container::{self, Inv, Stack};

/// `JukeboxBlockEntity`: the disc in its one slot, and its
/// `JukeboxSongPlayer`.
#[derive(Clone, Debug)]
pub struct Jukebox {
    /// The one slot, as a list so it reads like any container's.
    pub items: Vec<Stack>,
    song: Option<JukeboxSong>,
    ticks_since_song_started: i64,
}

impl Jukebox {
    pub fn new() -> Jukebox {
        Jukebox { items: vec![Stack::default()], song: None, ticks_since_song_started: 0 }
    }
}

impl Default for Jukebox {
    fn default() -> Self {
        Jukebox::new()
    }
}

fn jukebox(level: &Level, pos: Pos) -> Option<&Jukebox> {
    match level.block_entities.get(&pos) {
        Some(BlockEntity::Jukebox(j)) => Some(j),
        _ => None,
    }
}

fn jukebox_mut(level: &mut Level, pos: Pos) -> Option<&mut Jukebox> {
    match level.block_entities.get_mut(&pos) {
        Some(BlockEntity::Jukebox(j)) => Some(j),
        _ => None,
    }
}

/// Whether the jukebox at `pos` holds a disc.
pub fn has_item(level: &Level, pos: Pos) -> bool {
    jukebox(level, pos).is_some_and(|j| !j.items[0].is_empty())
}

/// `ownSignal`: 15 to every side while a song plays.
pub fn signal(level: &Level, pos: Pos) -> i32 {
    if jukebox(level, pos).is_some_and(|j| j.song.is_some()) { 15 } else { 0 }
}

/// `getComparatorOutput`: the disc's value, playing or not.
pub fn analog_output(level: &Level, pos: Pos) -> i32 {
    jukebox(level, pos).and_then(|j| items::jukebox_song(&j.items[0].id)).map_or(0, |s| s.comparator_output)
}

/// `canPlaceItem`: a disc, into the empty slot.
pub fn can_place_item(level: &Level, pos: Pos, slot: usize, stack: &Stack) -> bool {
    slot == 0 && items::jukebox_song(&stack.id).is_some() && !has_item(level, pos)
}

/// `canTakeItem`: only into a container with an empty slot.
pub fn can_take_item(level: &Level, into: Inv) -> bool {
    container::items(level, into).is_some_and(|i| i.iter().any(Stack::is_empty))
}

/// `setTheItem`: the slot takes `stack`, `has_record` follows it, and the
/// song it plays starts, or the one playing stops.
pub fn set_the_item(level: &mut Level, pos: Pos, stack: Stack) {
    let inserted = !stack.is_empty();
    let song = items::jukebox_song(&stack.id).filter(|_| inserted);
    let Some(j) = jukebox_mut(level, pos) else { return };
    j.items[0] = if inserted { stack } else { Stack::default() };
    // `notifyItemChangedInJukebox`: the block event goes out even when
    // `has_record` already had the value.
    let state = level.get(pos);
    level.set_block(pos, level.with_bool(state, "has_record", inserted), flags::CLIENTS);
    level.game_event(GameEvent::BlockChange, pos);
    match song {
        Some(song) => play(level, pos, song),
        None => stop(level, pos),
    }
}

/// `splitTheItem`: the whole disc comes out, whatever was asked for.
pub fn split_the_item(level: &mut Level, pos: Pos) -> Stack {
    let item = jukebox(level, pos).map(|j| j.items[0].clone()).unwrap_or_default();
    set_the_item(level, pos, Stack::default());
    item
}

/// `JukeboxSongPlayer.play`. The record event (1010) is only sound.
fn play(level: &mut Level, pos: Pos, song: JukeboxSong) {
    let Some(j) = jukebox_mut(level, pos) else { return };
    j.song = Some(song);
    j.ticks_since_song_started = 0;
    on_song_changed(level, pos);
}

/// `JukeboxSongPlayer.stop`. Only allays hear `jukebox_stop_play`, and the
/// stop event (1011) is only sound.
fn stop(level: &mut Level, pos: Pos) {
    let Some(j) = jukebox_mut(level, pos) else { return };
    if j.song.take().is_none() {
        return;
    }
    j.ticks_since_song_started = 0;
    on_song_changed(level, pos);
}

/// `onSongChanged`: the signal changed, and so did the block entity.
fn on_song_changed(level: &mut Level, pos: Pos) {
    let block = level.block(level.get(pos));
    level.update_neighbors_at(pos, block);
    container::set_changed(level, pos);
}

/// The block entity's ticker, `JukeboxSongPlayer.tick`, which runs while
/// `has_record` is set.
pub fn tick_entity(level: &mut Level, pos: Pos) {
    let Some(j) = jukebox_mut(level, pos) else { return };
    let Some(song) = j.song else { return };
    if song.has_finished(j.ticks_since_song_started) {
        stop(level, pos);
        return;
    }
    // `jukebox_play` reaches only allays; `spawnMusicParticles` picks the
    // note's colour with the level's random source.
    let emit = j.ticks_since_song_started % 20 == 0;
    j.ticks_since_song_started += 1;
    if emit {
        level.random.next_int_bounded(4);
    }
}

/// `useWithoutItem`: a jukebox with a record gives it back.
pub fn use_without_item(level: &mut Level, pos: Pos, state: StateId) {
    if level.flag(state, "has_record") && jukebox(level, pos).is_some() {
        pop_out_the_item(level, pos);
    }
}

/// `popOutTheItem`: the disc comes out on top of the jukebox, as an item
/// entity in levels that simulate entities.
fn pop_out_the_item(level: &mut Level, pos: Pos) {
    if !has_item(level, pos) {
        return;
    }
    let item = split_the_item(level, pos);
    if level.simulate_entities {
        // `Vec3.atLowerCornerWithOffset(pos, 0.5, 1.01, 0.5)
        //     .offsetRandomXZ(random, 0.7F)`.
        let r = &mut level.random;
        let dx = (r.next_float() - 0.5) * 0.7;
        let dz = (r.next_float() - 0.5) * 0.7;
        let at = Vec3::new(pos.x as f64 + 0.5 + dx as f64, pos.y as f64 + 1.01, pos.z as f64 + 0.5 + dz as f64);
        // `new ItemEntity` and `setDefaultPickUpDelay`, as `spawnAtLocation`.
        crate::entity::item::spawn_at_location(level, at, item);
    }
    on_song_changed(level, pos);
}
