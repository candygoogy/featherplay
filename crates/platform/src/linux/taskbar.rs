//! Taskbar buttons: none on Linux, so `attach` reports `Unsupported`.

pub use crate::common::taskbar::*;

use crate::window::Handle;
use crate::{Feature, Unsupported};

/// Thumb buttons, which this OS doesn't have yet.
pub struct TaskbarButtons {
    _none: (),
}

impl TaskbarButtons {
    /// Always an error: `Feature::TaskbarButtons` isn't supported here.
    pub fn attach(
        _window: Handle,
        _on_click: impl Fn(ThumbButton) + 'static,
    ) -> Result<Self, String> {
        Err(Unsupported(Feature::TaskbarButtons).to_string())
    }

    /// Does nothing (there are no buttons).
    pub fn set_playing(&self, _playing: bool) {}
}

/// Whether the taskbar (the panel) is light. Desktops share no way to ask, so the window icon
/// is always the one drawn for a dark panel.
pub fn light_taskbar() -> bool {
    false
}
