//! Taskbar buttons: none yet (phase 5), so `attach` reports `Unsupported`.

pub use crate::common::taskbar::*;

use crate::{Feature, Unsupported};

/// Thumb buttons, which this OS doesn't have yet.
pub struct TaskbarButtons {
    _none: (),
}

impl TaskbarButtons {
    /// Always an error: `Feature::TaskbarButtons` isn't supported here.
    pub fn attach(_hwnd: isize, _on_click: impl Fn(ThumbButton) + 'static) -> Result<Self, String> {
        Err(Unsupported(Feature::TaskbarButtons).to_string())
    }

    /// Does nothing (there are no buttons).
    pub fn set_playing(&self, _playing: bool) {}
}

/// Whether the taskbar (the panel) is light. Dark until phase 5 reads the desktop's theme.
pub fn light_taskbar() -> bool {
    false
}
