//! Windows integrations: media controls (SMTC), taskbar thumb buttons, the tray icon,
//! toasts and global hotkeys, plus SponsorBlock lookups, Discord presence and the local API,
//! and the app's identity for the shell (`identity`: the name and icon the media card shows).
//!
//! Everything here except `notify`, `notify_track`, `parse_hotkey` and `local_api` lives on
//! the UI thread, the one that owns the window and pumps its messages. Events go to the
//! controller as `UiCommand`s over a `Sender`, which never blocks. `sponsorblock`, `discord`
//! and `local_api` are driven by the controller, off the UI thread.
#![cfg_attr(
    test,
    allow(
        clippy::print_stderr,
        clippy::print_stdout,
        reason = "tests report skips and measurements"
    )
)]

use std::time::Duration;

use featherplay_platform::media_controls::{MediaEvent, MediaState};
use featherplay_platform::taskbar::ThumbButton;

pub mod discord;
mod hotkeys;
pub mod identity;
pub mod local_api;
mod notify;
pub mod sponsorblock;
mod tooltip;

pub use discord::Discord;
pub use hotkeys::{Hotkeys, parse_hotkey};
pub use notify::{notify, notify_track};
pub use tooltip::tooltip_text;

/// The channel commands travel on. Unbounded, so sending from a window procedure or a
/// WinRT callback never blocks.
pub type Sender<T> = tokio::sync::mpsc::UnboundedSender<T>;

/// What the user asked for through the media card, the taskbar, the tray, a hotkey or the
/// local API.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UiCommand {
    PlayPause,
    Next,
    Previous,
    Show,
    Quit,
    /// Play, or keep playing (the local API).
    Play,
    /// Pause, or stay paused (the local API).
    Pause,
    SeekTo(Duration),
    SeekBack(Duration),
    SeekForward(Duration),
    /// The volume, 0 to 100.
    SetVolume(u8),
    ToggleShuffle,
    /// Press the repeat button this many times: Off, All, One, then Off again.
    CycleRepeat(u8),
}

impl From<MediaEvent> for UiCommand {
    fn from(event: MediaEvent) -> Self {
        match event {
            MediaEvent::PlayPause => UiCommand::PlayPause,
            MediaEvent::Next => UiCommand::Next,
            MediaEvent::Previous => UiCommand::Previous,
            MediaEvent::Show => UiCommand::Show,
        }
    }
}

impl From<ThumbButton> for UiCommand {
    fn from(button: ThumbButton) -> Self {
        match button {
            ThumbButton::Previous => UiCommand::Previous,
            ThumbButton::PlayPause => UiCommand::PlayPause,
            ThumbButton::Next => UiCommand::Next,
        }
    }
}

/// The current track and playback state, as the integrations show it.
#[derive(Clone, Debug, PartialEq)]
pub struct NowPlaying {
    /// The video ID.
    pub track_id: Option<String>,
    pub title: String,
    pub artist: String,
    /// The channel ID of the first artist, if it has one.
    pub artist_id: Option<String>,
    pub album: Option<String>,
    /// The album's browse ID, if the track has an album.
    pub album_id: Option<String>,
    pub art_url: Option<String>,
    pub playing: bool,
    /// Where the song is, in the song's time.
    pub position: Duration,
    pub duration: Option<Duration>,
    /// The playback speed: the position advances this much per second (1.0 is normal).
    pub speed: f32,
}

impl Default for NowPlaying {
    fn default() -> Self {
        Self {
            track_id: None,
            title: String::new(),
            artist: String::new(),
            artist_id: None,
            album: None,
            album_id: None,
            art_url: None,
            playing: false,
            position: Duration::ZERO,
            duration: None,
            speed: 1.0,
        }
    }
}

impl From<&NowPlaying> for MediaState {
    fn from(np: &NowPlaying) -> Self {
        MediaState {
            title: np.title.clone(),
            artist: np.artist.clone(),
            album: np.album.clone(),
            art_url: np.art_url.clone(),
            duration: np.duration,
            playing: np.playing,
            position: np.position,
            speed: np.speed,
        }
    }
}
