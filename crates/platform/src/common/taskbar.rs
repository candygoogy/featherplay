//! The taskbar buttons' events.

/// A thumb button under the taskbar thumbnail, as the user clicked it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ThumbButton {
    Previous,
    PlayPause,
    Next,
}
