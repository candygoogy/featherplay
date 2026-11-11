//! featherplay's logic that needs no window (ARCHITECTURE.md, "Phase 3"). The app crate
//! keeps the pages, the UI bridge, the windows and startup, and calls in here.
//!
//! `diagnostics` has the log file, the crash files and the Copy diagnostics text; `log!`
//! writes to the log, here and in the app. `link` parses a link pasted into the search
//! field, and `suggest` paces the suggestions under it. `place` is where a pick leads or a
//! queue came from, which the app maps to its pages. `task` has the detached task helpers
//! everything here spawns with.

/// Writes a line to the log file (`diagnostics`), prefixed with the time and the seconds
/// since start; debug builds print it to stderr too.
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        $crate::diagnostics::write(format_args!($($arg)*))
    };
}

pub mod diagnostics;
pub mod link;
pub mod place;
pub mod suggest;
pub mod task;
