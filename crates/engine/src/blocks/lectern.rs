//! `LecternBlock` and `LecternBlockEntity`, for a lectern holding a book.
//!
//! Turning to another page powers the lectern for 2 game ticks: it powers
//! its neighbors weakly and the block below it strongly. A comparator reads
//! how far through the book the open page is.

use crate::level::{Level, flags};
use crate::pos::{Dir, Pos};
use crate::registry::StateId;

use super::BlockEntity;

/// The book's page count and the open page.
#[derive(Clone, Debug, Default)]
pub struct Lectern {
    pub pages: i32,
    pub page: i32,
}

fn lectern(level: &Level, pos: Pos) -> Option<&Lectern> {
    match level.block_entities.get(&pos) {
        Some(BlockEntity::Lectern(l)) => Some(l),
        _ => None,
    }
}

/// `setBook`: a book of `pages` pages, open at the first.
pub fn set_book(level: &mut Level, pos: Pos, pages: i32) {
    if let Some(BlockEntity::Lectern(l)) = level.block_entities.get_mut(&pos) {
        *l = Lectern { pages, page: 0 };
        super::container::set_changed(level, pos);
    }
}

/// `setPage`, as turning a page in the book's menu does.
pub fn set_page(level: &mut Level, pos: Pos, page: i32) {
    let Some(BlockEntity::Lectern(l)) = level.block_entities.get_mut(&pos) else { return };
    let page = page.clamp(0, (l.pages - 1).max(0));
    if page == l.page {
        return;
    }
    l.page = page;
    super::container::set_changed(level, pos);
    signal_page_change(level, pos, level.get(pos));
}

/// `signalPageChange`.
fn signal_page_change(level: &mut Level, pos: Pos, state: StateId) {
    change_powered(level, pos, state, true);
    level.schedule_tick(pos, level.block(state), 2, 0);
}

fn change_powered(level: &mut Level, pos: Pos, state: StateId, powered: bool) {
    let state = level.with_bool(state, "powered", powered);
    level.set_block(pos, state, flags::ALL);
    update_below(level, pos, state);
}

fn update_below(level: &mut Level, pos: Pos, state: StateId) {
    let orientation = level.initial_orientation(Some(Dir::Down), Some(Dir::Up));
    level.update_neighbors_at_oriented(pos.below(), level.block(state), orientation);
}

pub fn tick(level: &mut Level, pos: Pos, state: StateId) {
    change_powered(level, pos, state, false);
}

pub fn affect_neighbors_after_removal(level: &mut Level, pos: Pos, state: StateId) {
    if level.flag(state, "powered") {
        update_below(level, pos, state);
    }
}

pub fn get_signal(level: &Level, state: StateId) -> i32 {
    if level.flag(state, "powered") { 15 } else { 0 }
}

pub fn get_direct_signal(level: &Level, state: StateId, dir: Dir) -> i32 {
    if dir == Dir::Up { get_signal(level, state) } else { 0 }
}

/// `getAnalogOutputSignal`: 1 on the first page up to 15 on the last.
pub fn analog_output(level: &Level, pos: Pos, state: StateId) -> i32 {
    if !level.flag(state, "has_book") {
        return 0;
    }
    let Some(l) = lectern(level, pos) else { return 0 };
    let f = if l.pages > 1 { l.page as f32 / (l.pages as f32 - 1.0) } else { 1.0 };
    (f * 14.0).floor() as i32 + 1
}
