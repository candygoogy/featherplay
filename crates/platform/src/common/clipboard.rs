//! What the clipboard functions report on every OS.

/// What a paste would find on the system clipboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardState {
    /// It opens and holds text.
    Text,
    /// It opens but holds no text (an image, files, or nothing).
    NoText,
    /// It doesn't open: another program holds it, or the OS denies this process (on
    /// Windows the Win32 error, 5 for access denied).
    Closed { error: u32 },
}
