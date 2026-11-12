//! MPRIS (`org.mpris.MediaPlayer2` on the session bus), which the desktop's media keys, its
//! "now playing" widget (GNOME Shell, KDE Plasma) and tools like `playerctl` talk to. Served
//! with zbus on the run's session connection (`bus`), whose own thread waits on the socket:
//! nothing wakes while nothing changes.
//!
//! It shows the title, artist, album, cover URL, length, playback status, rate (the playback
//! speed) and position; the position moves on by itself between updates, at the rate, and a
//! jump says so with `Seeked`. Play, Pause, PlayPause, Stop, Next and Previous reach the app as
//! the Windows media keys do, and so do Seek, SetPosition, Raise and Quit, which only MPRIS
//! sends. `DesktopEntry` names the desktop entry, so the widget shows the app's icon.
//!
//! The bus name carries the process ID (`org.mpris.MediaPlayer2.featherplay.instance<pid>`,
//! as the MPRIS spec asks of a player that can run more than once): a dev build beside the
//! installed app (another `FEATHERPLAY_HOME`) gets a name of its own instead of failing.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use zbus::blocking::Connection;
use zbus::fdo;
use zbus::interface;
use zbus::names::BusName;
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{ObjectPath, Value};

use crate::common::identity::DISPLAY_NAME;
pub use crate::common::media_controls::*;
use crate::linux::bus;
use crate::window::Handle;

const PATH: &str = "/org/mpris/MediaPlayer2";
const PLAYER: &str = "org.mpris.MediaPlayer2.Player";
/// The desktop entry the widget takes the icon and name from.
const DESKTOP_ENTRY: &str = crate::APP_ID;
/// The track ID MPRIS gives "no track".
const NO_TRACK: &str = "/org/mpris/MediaPlayer2/TrackList/NoTrack";
/// The slowest and fastest playback speeds (the store's `Settings::SPEEDS`).
const MIN_RATE: f64 = 0.5;
const MAX_RATE: f64 = 2.0;
/// How far the position may land from where it was heading before an update counts as a
/// jump (`Seeked`) rather than the clock's drift.
const JUMP: Duration = Duration::from_millis(1500);

type OnEvent = Mutex<Box<dyn Fn(MediaEvent) + Send>>;

/// What the player shows, shared by the app's updates and the bus's reads.
#[derive(Default)]
struct State {
    shown: Option<MediaState>,
    /// When `shown.position` was the position.
    at: Option<Instant>,
    /// Counts the tracks shown, for each one's `mpris:trackid`.
    track: u64,
}

impl State {
    fn playing(&self) -> bool {
        self.shown.as_ref().is_some_and(|s| s.playing)
    }

    /// Where the song is now: the last position, moved on at the speed while playing, and
    /// never past the end.
    fn position(&self, now: Instant) -> Duration {
        let Some(s) = &self.shown else {
            return Duration::ZERO;
        };
        let mut at = s.position;
        if s.playing
            && let Some(then) = self.at
        {
            at += now
                .saturating_duration_since(then)
                .mul_f32(s.speed.max(0.0));
        }
        s.duration.map_or(at, |d| at.min(d))
    }

    fn track_id(&self) -> ObjectPath<'static> {
        if self.shown.is_none() {
            return ObjectPath::from_static_str_unchecked(NO_TRACK);
        }
        ObjectPath::try_from(format!("/org/featherplay/track/{}", self.track))
            .unwrap_or_else(|_| ObjectPath::from_static_str_unchecked(NO_TRACK))
    }

    fn metadata(&self) -> HashMap<&'static str, Value<'static>> {
        let mut dict = HashMap::from([("mpris:trackid", Value::from(self.track_id()))]);
        let Some(s) = &self.shown else {
            return dict;
        };
        dict.insert("xesam:title", Value::from(s.title.clone()));
        dict.insert("xesam:artist", Value::from(vec![s.artist.clone()]));
        if let Some(album) = &s.album {
            dict.insert("xesam:album", Value::from(album.clone()));
        }
        if let Some(url) = &s.art_url {
            dict.insert("mpris:artUrl", Value::from(url.clone()));
        }
        if let Some(d) = s.duration {
            dict.insert("mpris:length", Value::from(micros(d)));
        }
        dict
    }

    fn status(&self) -> &'static str {
        match &self.shown {
            None => "Stopped",
            Some(s) if s.playing => "Playing",
            Some(_) => "Paused",
        }
    }

    fn rate(&self) -> f64 {
        self.shown
            .as_ref()
            .map_or(1.0, |s| f64::from(s.speed).clamp(MIN_RATE, MAX_RATE))
    }
}

fn micros(d: Duration) -> i64 {
    i64::try_from(d.as_micros()).unwrap_or(i64::MAX)
}

fn lock(state: &Mutex<State>) -> std::sync::MutexGuard<'_, State> {
    state.lock().unwrap_or_else(PoisonError::into_inner)
}

fn send(events: &OnEvent, event: MediaEvent) {
    (events.lock().unwrap_or_else(PoisonError::into_inner))(event);
}

/// `org.mpris.MediaPlayer2`: the app itself.
struct Root {
    events: Arc<OnEvent>,
}

#[interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    fn raise(&self) {
        send(&self.events, MediaEvent::Show);
    }

    fn quit(&self) {
        send(&self.events, MediaEvent::Quit);
    }

    #[zbus(property)]
    fn can_quit(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_raise(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }

    #[zbus(property)]
    fn identity(&self) -> &str {
        DISPLAY_NAME
    }

    #[zbus(property)]
    fn desktop_entry(&self) -> &str {
        DESKTOP_ENTRY
    }

    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        Vec::new()
    }

    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        Vec::new()
    }
}

/// `org.mpris.MediaPlayer2.Player`: the track and the transport.
struct Player {
    state: Arc<Mutex<State>>,
    events: Arc<OnEvent>,
}

impl Player {
    fn has_track(&self) -> bool {
        lock(&self.state).shown.is_some()
    }
}

#[interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    fn next(&self) {
        send(&self.events, MediaEvent::Next);
    }

    fn previous(&self) {
        send(&self.events, MediaEvent::Previous);
    }

    fn play_pause(&self) {
        if self.has_track() {
            send(&self.events, MediaEvent::PlayPause);
        }
    }

    fn play(&self) {
        let paused = {
            let state = lock(&self.state);
            state.shown.is_some() && !state.playing()
        };
        if paused {
            send(&self.events, MediaEvent::PlayPause);
        }
    }

    fn pause(&self) {
        if lock(&self.state).playing() {
            send(&self.events, MediaEvent::PlayPause);
        }
    }

    /// The app has no stopped state: Stop pauses.
    fn stop(&self) {
        self.pause();
    }

    fn seek(&self, offset: i64) {
        if let Some(event) = seek_by(offset) {
            send(&self.events, event);
        }
    }

    fn set_position(&self, track_id: ObjectPath<'_>, position: i64) {
        let event = {
            let state = lock(&self.state);
            let length = state.shown.as_ref().and_then(|s| s.duration);
            let fits = u64::try_from(position)
                .ok()
                .map(Duration::from_micros)
                .filter(|p| length.is_none_or(|l| *p <= l));
            // A request for a track that's no longer the current one is dropped, as the
            // spec asks.
            fits.filter(|_| state.shown.is_some() && track_id == state.track_id())
        };
        if let Some(at) = event {
            send(&self.events, MediaEvent::SeekTo(at));
        }
    }

    fn open_uri(&self, _uri: String) -> fdo::Result<()> {
        Err(fdo::Error::NotSupported("featherplay opens no URIs".into()))
    }

    /// The position jumped (a seek, or the song restarted). Declared here for the
    /// interface's description; `MediaControls::update` sends it, from the UI thread.
    #[zbus(signal)]
    async fn seeked(emitter: &SignalEmitter<'_>, position: i64) -> zbus::Result<()>;

    #[zbus(property)]
    fn playback_status(&self) -> &'static str {
        lock(&self.state).status()
    }

    #[zbus(property)]
    fn rate(&self) -> f64 {
        lock(&self.state).rate()
    }

    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        MIN_RATE
    }

    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        MAX_RATE
    }

    #[zbus(property)]
    fn metadata(&self) -> HashMap<&'static str, Value<'static>> {
        lock(&self.state).metadata()
    }

    /// Read when asked, never signalled (the spec's rule): a jump sends `Seeked` instead.
    #[zbus(property(emits_changed_signal = "false"))]
    fn position(&self) -> i64 {
        micros(lock(&self.state).position(Instant::now()))
    }

    /// The app's volume isn't shown here.
    #[zbus(property)]
    fn volume(&self) -> f64 {
        1.0
    }

    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        true
    }

    #[zbus(property)]
    fn can_play(&self) -> bool {
        self.has_track()
    }

    #[zbus(property)]
    fn can_pause(&self) -> bool {
        self.has_track()
    }

    #[zbus(property)]
    fn can_seek(&self) -> bool {
        lock(&self.state)
            .shown
            .as_ref()
            .is_some_and(|s| s.duration.is_some())
    }

    #[zbus(property(emits_changed_signal = "const"))]
    fn can_control(&self) -> bool {
        true
    }
}

/// The app's command for MPRIS's Seek: `offset` microseconds ahead (or back, when negative).
fn seek_by(offset: i64) -> Option<MediaEvent> {
    let by = Duration::from_micros(offset.unsigned_abs());
    match offset {
        0 => None,
        o if o > 0 => Some(MediaEvent::SeekForward(by)),
        _ => Some(MediaEvent::SeekBack(by)),
    }
}

/// What an update changed, for the signals it sends.
#[derive(Debug, Default, PartialEq)]
struct Changes {
    /// The Player properties whose `PropertiesChanged` it sends.
    properties: Vec<&'static str>,
    /// Where the song jumped to (`Seeked`).
    seeked: Option<Duration>,
}

/// Shows `np` in `state` as of `now`, and says what changed.
fn apply(state: &mut State, np: MediaState, now: Instant) -> Changes {
    let mut changes = Changes::default();
    let old = state.shown.as_ref();
    let same_track = old.is_some_and(|s| {
        s.title == np.title
            && s.artist == np.artist
            && s.album == np.album
            && s.art_url == np.art_url
            && s.duration == np.duration
    });
    let first = old.is_none();
    let status_changed = old.map(|s| s.playing) != Some(np.playing);
    let rate_changed = old.map(|s| s.speed) != Some(np.speed);
    if first {
        changes.properties.extend(["CanPlay", "CanPause"]);
    }
    if !same_track {
        state.track += 1;
        changes.properties.extend(["Metadata", "CanSeek"]);
    }
    if status_changed {
        changes.properties.push("PlaybackStatus");
    }
    if rate_changed {
        changes.properties.push("Rate");
    }
    if same_track && state.position(now).abs_diff(np.position) > JUMP {
        changes.seeked = Some(np.position);
    }
    state.shown = Some(np);
    state.at = Some(now);
    changes
}

/// featherplay as an MPRIS player on the session bus, for as long as it's kept.
pub struct MediaControls {
    bus: &'static Connection,
    name: String,
    state: Arc<Mutex<State>>,
}

impl MediaControls {
    /// Puts featherplay on the session bus as an MPRIS player. `window` isn't needed here.
    /// An error if there's no session bus. `on_event` is called on zbus's thread.
    pub fn attach(
        _window: Handle,
        on_event: impl Fn(MediaEvent) + Send + 'static,
    ) -> Result<MediaControls, String> {
        let bus = bus::session().ok_or("media controls: no session D-Bus")?;
        let state = Arc::new(Mutex::new(State::default()));
        let events: Arc<OnEvent> = Arc::new(Mutex::new(Box::new(on_event)));
        let name = format!(
            "org.mpris.MediaPlayer2.{DISPLAY_NAME}.instance{}",
            std::process::id()
        );
        let served = (|| {
            let server = bus.object_server();
            server.at(
                PATH,
                Root {
                    events: events.clone(),
                },
            )?;
            server.at(
                PATH,
                Player {
                    state: state.clone(),
                    events,
                },
            )?;
            bus.request_name(name.as_str())
        })();
        let controls = MediaControls { bus, name, state };
        served.map_err(|e| format!("media controls: {e}"))?;
        Ok(controls)
    }

    /// Shows `np` over MPRIS: sends `PropertiesChanged` for what changed, and `Seeked` when
    /// the position jumped.
    pub fn update(&mut self, np: MediaState) {
        let (changes, values) = {
            let mut state = lock(&self.state);
            let changes = apply(&mut state, np, Instant::now());
            let values: HashMap<&str, Value<'_>> = changes
                .properties
                .iter()
                .map(|&p| (p, value(&state, p)))
                .collect();
            (changes, values)
        };
        if !values.is_empty() {
            let no_invalidated: &[&str] = &[];
            let sent = self.bus.emit_signal(
                None::<BusName<'_>>,
                PATH,
                "org.freedesktop.DBus.Properties",
                "PropertiesChanged",
                &(PLAYER, values, no_invalidated),
            );
            if let Err(e) = sent {
                log::warn!("mpris: PropertiesChanged: {e}");
            }
        }
        if let Some(at) = changes.seeked {
            let sent =
                self.bus
                    .emit_signal(None::<BusName<'_>>, PATH, PLAYER, "Seeked", &(micros(at),));
            if let Err(e) = sent {
                log::warn!("mpris: Seeked: {e}");
            }
        }
    }
}

/// The current value of the Player property `name`, for `PropertiesChanged`.
fn value(state: &State, name: &str) -> Value<'static> {
    match name {
        "Metadata" => Value::from(state.metadata()),
        "PlaybackStatus" => Value::from(state.status()),
        "Rate" => Value::from(state.rate()),
        "CanSeek" => Value::from(state.shown.as_ref().is_some_and(|s| s.duration.is_some())),
        // CanPlay and CanPause, once there's a track.
        _ => Value::from(state.shown.is_some()),
    }
}

impl Drop for MediaControls {
    fn drop(&mut self) {
        let _ = self.bus.release_name(self.name.as_str());
        let server = self.bus.object_server();
        let _ = server.remove::<Player, _>(PATH);
        let _ = server.remove::<Root, _>(PATH);
    }
}

#[cfg(test)]
mod tests;
