//! The clipboard, through arboard: the Wayland clipboard where the compositor offers the
//! data-control protocol, else the X11 one (XWayland's, on a Wayland desktop), as Slint's
//! own copy and paste reach it.
//!
//! On X11 and Wayland the program that copied serves the text to whoever pastes, so the
//! clipboard handle lives as long as the app: what featherplay copied stays pasteable until
//! something else is copied or the app quits (or a clipboard manager keeps it).

use std::sync::{Mutex, MutexGuard};

use arboard::{Clipboard, SetExtLinux};

pub use crate::common::clipboard::*;

/// The process's clipboard, made on first use.
static CLIPBOARD: Mutex<Option<Clipboard>> = Mutex::new(None);

/// The clipboard, made first if need be.
fn clipboard() -> Result<MutexGuard<'static, Option<Clipboard>>, String> {
    let mut slot = CLIPBOARD.lock().unwrap_or_else(|e| e.into_inner());
    if slot.is_none() {
        *slot = Some(Clipboard::new().map_err(|e| format!("clipboard: {e}"))?);
    }
    Ok(slot)
}

fn set(text: &str, secret: bool) -> Result<(), String> {
    let mut slot = clipboard()?;
    let Some(c) = slot.as_mut() else {
        return Err("clipboard: not available".into());
    };
    let set = c.set();
    let set = if secret {
        set.exclude_from_history()
    } else {
        set
    };
    set.text(text).map_err(|e| format!("clipboard: {e}"))
}

/// Puts a secret (the local API's token) on the clipboard as text, marked for clipboard
/// managers to leave out of their history (KDE's password hint).
pub fn copy_secret(text: &str) -> Result<(), String> {
    set(text, true)
}

/// Puts `text` on the clipboard, as Ctrl+C in a text field does.
pub fn copy_text(text: &str) -> Result<(), String> {
    set(text, false)
}

/// What a paste would get. It reads the text to know there is some, but never keeps it.
pub fn state() -> ClipboardState {
    let Ok(mut slot) = clipboard() else {
        return ClipboardState::Closed { error: 0 };
    };
    match slot.as_mut().map(Clipboard::get_text) {
        Some(Ok(_)) => ClipboardState::Text,
        Some(Err(arboard::Error::ContentNotAvailable)) => ClipboardState::NoText,
        _ => ClipboardState::Closed { error: 0 },
    }
}
