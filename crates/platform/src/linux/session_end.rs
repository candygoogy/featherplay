//! The end of the session and second launches: not heard yet (phase 5). `listen` starts
//! nothing, so the app ends when the desktop ends its process.

pub use crate::common::session_end::*;

/// Starts nothing yet: neither `on_end` nor `show` is ever called.
pub fn listen(_on_end: impl FnOnce() + Send + 'static, _show: Option<Show>) -> Result<(), String> {
    Ok(())
}

/// Nothing waits for it yet.
pub fn saved() {}
