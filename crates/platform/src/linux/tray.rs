//! The tray icon and its menu: none yet (phase 5), so `Tray::new` reports `Unsupported`.

pub use crate::common::tray::*;

use crate::{Feature, Unsupported};

/// A tray icon, which this OS doesn't have yet.
pub struct Tray {
    _none: (),
}

impl Tray {
    /// Always an error: `Feature::Tray` isn't supported here.
    pub fn new(
        _tip: &str,
        _on_event: impl Fn(TrayEvent) + Send + Sync + 'static,
    ) -> Result<Tray, String> {
        Err(Unsupported(Feature::Tray).to_string())
    }

    /// Does nothing (there is no icon).
    pub fn refresh_icon(&self) {}

    /// Does nothing (there is no icon).
    pub fn set_tooltip(&self, _s: &str) {}

    /// Does nothing (there is no menu).
    pub fn set_menu(&self, _s: &MenuState) {}
}
