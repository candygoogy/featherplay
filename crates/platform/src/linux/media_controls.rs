//! The media controls: none yet (phase 5), so `attach` reports `Unsupported`.

pub use crate::common::media_controls::*;

use crate::{Feature, Unsupported};

/// The system's media controls, which this OS doesn't have yet.
pub struct MediaControls {
    _none: (),
}

impl MediaControls {
    /// Always an error: `Feature::MediaControls` isn't supported here.
    pub fn attach(
        _hwnd: isize,
        _on_event: impl Fn(MediaEvent) + Send + 'static,
    ) -> Result<MediaControls, String> {
        Err(Unsupported(Feature::MediaControls).to_string())
    }

    /// Does nothing (there are no controls).
    pub fn update(&mut self, _np: MediaState) {}
}
