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
//! the recorded `fixtures`. `window_place` is where the main window opens, from the place it
//! was left. `playlist_edit` moves songs within a playlist on the page's list, and puts them
//! back if the write fails.

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
pub mod playlist_edit;
pub mod presence;
pub mod scrobbler;
pub mod suggest;
pub mod task;
pub mod updates;
pub mod window_place;

/// A fresh, empty folder for a test, `test-{name}-{process id}` under `FEATHERPLAY_HOME`.
/// `None` when `FEATHERPLAY_HOME` isn't set: the test skips, saying so, and writes nothing
/// anywhere else.
#[cfg(test)]
#[allow(clippy::print_stderr, reason = "a test says why it skips")]
pub(crate) fn test_home(name: &str) -> Option<std::path::PathBuf> {
    let Some(base) = std::env::var_os("FEATHERPLAY_HOME") else {
        eprintln!("skipped: FEATHERPLAY_HOME isn't set");
        return None;
    };
    let dir = std::path::PathBuf::from(base).join(format!("test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the test's folder");
    Some(dir)
}
