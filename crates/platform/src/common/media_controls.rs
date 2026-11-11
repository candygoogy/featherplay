//! What the media controls show, and what they report.

use std::time::Duration;

/// A press on the system's media controls (the media card, the media keys).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaEvent {
    /// Play or pause. A Play while playing, or a Pause or Stop while paused, isn't reported.
    PlayPause,
    Next,
    Previous,
    /// Bring the app's window forward.
    Show,
}

/// The current track and playback state, as the media controls show them.
#[derive(Clone, Debug, PartialEq)]
pub struct MediaState {
    pub title: String,
    pub artist: String,
    pub album: Option<String>,
    pub art_url: Option<String>,
    pub duration: Option<Duration>,
    pub playing: bool,
    /// Where the song is, in the song's time.
    pub position: Duration,
    /// The playback speed: the position advances this much per second (1.0 is normal).
    pub speed: f32,
}
