//! Pure featherplay logic: the track model, the play queue, the player state machine and
//! the scrobble rules, and what a signed-in session's cookies look like. No I/O.

pub mod failure;
pub mod i18n;
pub mod model;
pub mod player;
pub mod queue;
pub mod scrobble;
pub mod session;
pub mod sync;
pub mod text;
