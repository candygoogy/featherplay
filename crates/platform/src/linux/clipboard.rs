//! The clipboard: not reached yet (phase 5), so copying fails and a paste's clipboard
//! reads as closed.

pub use crate::common::clipboard::*;

/// Why copying fails here.
const NOT_YET: &str = "the clipboard isn't supported on this system yet";

/// Always an error: the clipboard isn't reached yet.
pub fn copy_secret(_text: &str) -> Result<(), String> {
    Err(NOT_YET.into())
}

/// Always an error: the clipboard isn't reached yet.
pub fn copy_text(_text: &str) -> Result<(), String> {
    Err(NOT_YET.into())
}

/// Closed, with no error code: the clipboard isn't reached yet.
pub fn state() -> ClipboardState {
    ClipboardState::Closed { error: 0 }
}
