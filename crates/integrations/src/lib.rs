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

pub mod discord;
mod hotkeys;
mod icons;
pub mod identity;
pub mod local_api;
mod notify;
mod smtc;
pub mod sponsorblock;
mod taskbar;
mod tray;
mod tray_menu;

pub use discord::Discord;
pub use hotkeys::{Hotkeys, parse_hotkey};
pub use notify::{notify, notify_track};
pub use smtc::Smtc;
pub use taskbar::{TaskbarButtons, light_taskbar};
pub use tray::{TOOLTIP_LINE_UTF16, TOOLTIP_MAX_UTF16, Tray, tooltip_text};
pub use tray_menu::{MenuState, TrayChoice};

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

/// What the tray icon reports to the UI thread itself (the window procedure calls the
/// handler; it should only post the event on).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrayEvent {
    /// A left click, released at this point of the screen (physical pixels).
    Click { x: i32, y: i32 },
    /// A right click: Windows' menu opens.
    Menu,
    /// The pointer came onto the icon or moved over it: the hover text may show.
    Hover,
    /// An entry of the right-click menu was chosen.
    Chose(TrayChoice),
}

/// The current track and playback state, as the integrations show it.
#[derive(Clone, Debug, Default, PartialEq)]
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
    pub position: Duration,
    pub duration: Option<Duration>,
}
