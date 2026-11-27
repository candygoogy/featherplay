//! What the window and the controller send each other: the `Options` at the start, the
//! window's `Action`s, the `UiEvent`s back with the `PlayerState`, and the `UiLink` that
//! carries them.

use std::ops::Deref;
use std::sync::Arc;
use std::time::Duration;

use featherplay_core::model::{LikeStatus, PlaylistId, RepeatMode, Track, TrackId};
use featherplay_integrations::UiCommand;
use featherplay_streams::ResolvedStream;
use tokio::sync::mpsc;

use super::{
    AccountAction, LocalApiNotice, Mark, QueueAction, QueueInfo, SleepAction, SleepState, account,
};
use crate::catalog::Catalog;
use crate::place::Place;
use crate::updates;

#[derive(Debug, Default)]
pub struct Options {
    /// Play this track's radio instead of restoring the session.
    pub play: Option<TrackId>,
    /// Volume for this run, instead of the one in the settings (which is left alone).
    pub volume: Option<f32>,
    /// Start the first track here.
    pub start_at: Option<Duration>,
    /// Stand-in data instead of an account, set by the app's performance harness: no
    /// account is read and nothing is asked of YouTube (no helpers, no resolves); a song
    /// plays the stream this gives (a local file), or nothing.
    pub fixture_stream: Option<fn() -> Option<ResolvedStream>>,
}

/// What the window asks for (pages, the bar). The integrations still send `UiCommand`s.
#[derive(Debug)]
pub enum Action {
    /// The transport, as the media keys send it.
    Toggle,
    Next,
    Previous,
    /// Play `tracks` from `start`; `radio` marks a radio queue (it continues by itself).
    Play {
        tracks: Vec<Track>,
        start: usize,
        radio: bool,
    },
    /// Fetch the radio of a track and play it.
    Radio(TrackId),
    SeekTo(Duration),
    /// The 0..1 slider value. `save` (on release) also stores it in the settings.
    SetVolume {
        value: f32,
        save: bool,
    },
    /// Like or unlike: one write to the account.
    Rate(TrackId, LikeStatus),
    /// A rating the window wrote itself (the album menu's bulk job, `album_actions`): kept as
    /// a `Rate` that went through is, for the bar and the queue. Nothing is sent.
    Rated(TrackId, LikeStatus),
    /// Undo after "Not for me": back to `track`, at `at` (`queue::back_to_inputs`).
    BackTo {
        track: Track,
        at: Duration,
    },
    /// The song menu: play this track after the current one (`QueueAction::PlayNext`).
    PlayNext(Track),
    /// The song menu: add this track at the end of the queue (`QueueAction::Enqueue`).
    AddToQueue(Track),
    /// The song menu: one write to the account, adding the tracks to a playlist.
    AddToPlaylist {
        playlist: PlaylistId,
        tracks: Vec<TrackId>,
    },
    SetShuffle(bool),
    SetRepeat(RepeatMode),
    /// The Up next panel's edits.
    Queue(QueueAction),
    /// Sign-in, sign-out and settings.
    Account(AccountAction),
    /// Resolve this track ahead of a click: a row the pointer rests on, Home's featured
    /// song. `track` when the window has it (it gives the song version to play).
    Prefetch {
        id: TrackId,
        track: Option<Track>,
    },
    /// The pointer rests on a song: while paused, the audio output starts if Windows may have
    /// let its device go idle, so a click then plays at once (`AudioEngine::wake`).
    WakeOutput,
    /// The song rows on screen, the likeliest click first, each with its track when the
    /// window has it: resolved ahead while a song plays (`streams.rs`). Replaces the last;
    /// empty when none shows (a page without songs, the window minimised or hidden).
    Prepare(Vec<(TrackId, Option<Track>)>),
    /// The sleep timer's popup and tray submenu: start it, add to it, turn it off.
    Sleep(SleepAction),
    /// A page's Try again while the stored session can't connect (`Source::Unreachable`):
    /// connect with it again (`reconnect.rs`).
    Reconnect,
    /// Quit, as the tray's Quit does. It comes after the actions sent before it (a setting
    /// the close dialog saved), so it travels on this channel rather than as a `UiCommand`.
    Quit,
}

/// The player as the window shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct PlayerState {
    /// With likes the user changed this session applied.
    pub current: Option<Track>,
    pub upcoming: SharedTracks,
    /// Audible, or loading with the intent to play: what the buttons show.
    pub playing: bool,
    /// Sound is coming out; the progress line advances only then (not while the stream
    /// stalls).
    pub audible: bool,
    /// Resolving or buffering the current track to play it (not while paused): the
    /// progress line shimmers.
    pub loading: bool,
    pub position: Duration,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    pub radio: bool,
    /// The 0..1 slider value.
    pub volume: f32,
    /// "Playing from …" and the Autoplay divider.
    pub queue: QueueInfo,
    /// The playback speed (1.0 is normal): the position advances this much per second.
    pub speed: f32,
    /// The current track came in by a crossfade, this long, that's still on: from the
    /// crossfade's start until a load, a seek or a pause. `None` after a cut. The window
    /// turns the colours at its middle (`turn` in the app).
    pub crossfade: Option<Duration>,
}

/// Tracks shared by every copy of the state that has them (each one sent, the window's,
/// `Ctx::player`'s): a clone counts a reference instead of copying a long queue. Reads as a
/// `&[Track]`. A song change keeps the copy too: what's left of it after the tracks played
/// (`tail`).
#[derive(Clone)]
pub struct SharedTracks {
    all: Arc<[Track]>,
    /// Where these start in `all`.
    from: usize,
}

impl SharedTracks {
    fn new(all: Arc<[Track]>) -> Self {
        SharedTracks { all, from: 0 }
    }

    /// The last `len` of these, sharing them (`len` at most `self.len()`).
    pub(super) fn tail(&self, len: usize) -> SharedTracks {
        SharedTracks {
            all: self.all.clone(),
            from: self.from + (self.len() - len),
        }
    }
}

impl Default for SharedTracks {
    fn default() -> Self {
        SharedTracks::new(Arc::new([]))
    }
}

impl Deref for SharedTracks {
    type Target = [Track];

    fn deref(&self) -> &[Track] {
        &self.all[self.from..]
    }
}

/// The same tracks, as one when they're shared.
impl PartialEq for SharedTracks {
    fn eq(&self, other: &Self) -> bool {
        (Arc::ptr_eq(&self.all, &other.all) && self.from == other.from) || **self == **other
    }
}

impl Eq for SharedTracks {}

impl std::fmt::Debug for SharedTracks {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("SharedTracks").field(&&**self).finish()
    }
}

impl<'a> IntoIterator for &'a SharedTracks {
    type Item = &'a Track;
    type IntoIter = std::slice::Iter<'a, Track>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl From<Vec<Track>> for SharedTracks {
    fn from(tracks: Vec<Track>) -> Self {
        SharedTracks::new(tracks.into())
    }
}

impl From<&[Track]> for SharedTracks {
    fn from(tracks: &[Track]) -> Self {
        SharedTracks::new(tracks.into())
    }
}

impl FromIterator<Track> for SharedTracks {
    fn from_iter<I: IntoIterator<Item = Track>>(tracks: I) -> Self {
        SharedTracks::new(tracks.into_iter().collect())
    }
}

/// Sent to the UI thread through `UiLink::events`.
#[derive(Clone, Debug)]
pub enum UiEvent {
    Player(Box<PlayerState>),
    /// `signed_in` is also true while a stored session can't connect
    /// (`Source::Unreachable`): the window keeps its signed-in look and pages offer Try
    /// again.
    Account {
        signed_in: bool,
    },
    /// A reconnect with the stored session failed again; the source stays `Unreachable`.
    Unreachable,
    /// The tray icon or its menu asked for the window.
    ShowWindow,
    /// Sign-in, session and Last.fm news for the account pages.
    AccountNotice(account::AccountEvent),
    /// The Discord connection, for Settings.
    Discord(crate::presence::Status),
    /// The local API's state and token, for Settings.
    LocalApi(LocalApiNotice),
    /// The app acts as another identity of the session (the account menu, the first
    /// sign-in's picker): the window reloads what belongs to the account. Also sent with the
    /// identity unchanged when a switch fails, so a picker waiting for it carries on.
    Identity(featherplay_ytm::Identity),
    /// A new sign-in, just before its `Account { signed_in: true }`: the window asks which
    /// identity to act as when the session has several (`account_picker`).
    AskIdentity,
    /// That new sign-in works for this run, but Windows wouldn't store it for the next
    /// start; sent just before its `Account { signed_in: true }`. The window says so once
    /// the sign-in shows.
    SignInNotSaved,
    /// Playback stopped after failures: what to say, and a link to offer.
    Stopped {
        text: &'static str,
        help: Option<updates::Help>,
    },
    /// Playback stopped because the helpers couldn't be downloaded: what to say, with a Copy
    /// diagnostics button.
    HelpersFailed(&'static str),
    /// A fresh update check found a newer release (About's banner).
    Release(updates::Release),
    /// A plain toast: a song waits for the helpers' first download.
    Toast(&'static str),
    /// A toast saying something went wrong: no sound output.
    ToastError(&'static str),
    /// YouTube refuses songs while the DNS sends its hosts abroad: an error toast whose
    /// button turns on "Look up YouTube directly" (`Change::DnsOverHttps`).
    RefusedByDns(&'static str),
    /// What Skip silence drops from the current track, for the seek bar (`silences.rs`);
    /// empty while it's off, and from a track change until the new track is scanned.
    Silences(Vec<Mark>),
    /// The sleep timer started, changed its minutes or fade, or stopped (`None`).
    Sleep(Option<SleepState>),
    /// A song played long enough to count (`scrobble::play_threshold`) from an album, a
    /// playlist or an artist (`place`, which Up next calls `label`): Home's Shortcuts
    /// suggest what's played.
    Played {
        place: Place,
        label: String,
        track: Box<Track>,
    },
    /// The songs YouTube refuses to play now, all of them, each time one comes or goes:
    /// their rows show greyed (`streams.rs`).
    Refused(Vec<TrackId>),
}

/// The controller's side of the UI thread.
pub struct UiLink {
    pub commands: mpsc::UnboundedReceiver<UiCommand>,
    /// A sender on `commands`, for the local API's controls. Weak, so `commands` still ends
    /// when the UI's senders are gone, unless the local API is running.
    pub controls: mpsc::WeakUnboundedSender<UiCommand>,
    pub actions: mpsc::UnboundedReceiver<Action>,
    /// Hands an event to the UI thread; must not block.
    pub events: Box<dyn Fn(UiEvent) + Send>,
    /// Set to the signed-in API once there is one.
    pub catalog: Catalog,
}
