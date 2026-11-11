//! featherplay's logic that needs no window (ARCHITECTURE.md, "Phase 3"). The app crate
//! keeps the pages, the UI bridge, the windows and startup, and calls in here.
//!
//! `controller` is the loop that plays: the window's `Action`s and the integrations'
//! `UiCommand`s come in on channels, with `Options` at the start, and every `UiEvent` goes
//! out through one callback (`UiLink`). Beside it run `scrobbler` (Last.fm, ListenBrainz)
//! and `presence` (Discord, on a thread of its own).
//!
//! `diagnostics` has the log file, the crash files and the Copy diagnostics text; `log!`
//! writes to the log, here and in the app. `link` parses a link pasted into the search
//! field, and `suggest` paces the suggestions under it. `place` is where a pick leads or a
//! queue came from, which the app maps to its pages. `updates` checks for a newer release
//! and downloads its setup. `task` has the detached task helpers everything here spawns
//! with. `catalog` is the data source the pages and the controller share: the music API, or
//! the recorded `fixtures`.

/// Writes a line to the log file (`diagnostics`), prefixed with the time and the seconds
/// since start; debug builds print it to stderr too.
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        $crate::diagnostics::write(format_args!($($arg)*))
    };
}

pub mod catalog;
pub mod controller;
pub mod diagnostics;
pub mod fixtures;
pub mod link;
pub mod place;
pub mod presence;
pub mod scrobbler;
pub mod suggest;
pub mod task;
pub mod updates;
