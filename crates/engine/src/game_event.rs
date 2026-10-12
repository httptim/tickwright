//! Game events (`GameEvent`), as far as vibration listeners hear them.
//!
//! The game posts these from many places; only sculk sensors listen in the
//! engine, so posting is a no-op in a world without them.

/// The game events the engine posts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GameEvent {
    BlockActivate,
    BlockAttach,
    BlockChange,
    BlockClose,
    BlockDeactivate,
    BlockDestroy,
    BlockDetach,
    BlockOpen,
    BlockPlace,
    NoteBlockPlay,
    SculkSensorTendrilsClicking,
}

impl GameEvent {
    /// `VibrationSystem.getGameEventFrequency`, or `None` for an event
    /// outside the `vibrations` tag.
    pub fn frequency(self) -> Option<i32> {
        use GameEvent::*;
        Some(match self {
            BlockClose | BlockDeactivate | BlockDetach => 9,
            BlockOpen | BlockActivate | BlockAttach | NoteBlockPlay => 10,
            BlockChange => 11,
            BlockDestroy => 12,
            BlockPlace => 13,
            SculkSensorTendrilsClicking => return None,
        })
    }
}
