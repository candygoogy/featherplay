//! Runs `core::player` effects: owns the music API, the stream resolver, the audio engine
//! and the settings. Background work (resolves, radio fetches, audio events) reports back
//! over one channel as `Msg`, and every answer reaches the player as an `Input` on the
//! controller's loop, so the player is only ever touched from one place.
//!
//! The controller runs on its own thread (the UI owns the main one). Commands from the
//! media card, taskbar, tray and hotkeys arrive as `UiCommand`s, and the window's requests
//! as `Action`s; what's playing goes back as `UiEvent::Player` through `UiLink::events`.
//!
//! SponsorBlock segments are fetched when a music video starts and skipped from the tick
//! handler; Discord presence follows what the UI is shown, and so does the local API
//! (`local_api.rs`), whose controls arrive as `UiCommand`s too.
//!
//! `queue.rs` (Up next) and `account.rs` (sign-in, Last.fm, settings) hold their own
//! actions, so those areas extend the controller in their own files.

mod account;
mod local_api;
mod notice;
mod queue;
mod reconnect;
mod silences;
mod song_versions;
mod streams;

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use featherplay_audio::{AudioEngine, AudioEvent, Compressor, StreamSource};
use featherplay_core::model::{LikeStatus, PlaylistId, RepeatMode, Track, TrackId};
use featherplay_core::player::{Effect, Input, ListEnd, PlaybackPrefs, Player, Status, StreamInfo};
use featherplay_core::scrobble::Alternates;
use featherplay_core::tr;
use featherplay_integrations::sponsorblock::{self, Segment};
use featherplay_integrations::{NowPlaying, UiCommand, notify_track};
use featherplay_store::{
    Loudness, Paths, SecretKey, Settings, SkipSilence, StreamingQuality, get_secret, load_session,
    save_session,
};
use featherplay_streams::{
    Downloads, Quality, ResolveError, ResolvedStream, Sidecars, YtDlpResolver,
};
use featherplay_ytm::{ApiError, YtMusic};
use tokio::sync::mpsc;

pub use account::{
    AccountAction, AccountEvent, ApiKeys, Change, Cookies, LastfmStatus, ListenBrainzStatus, Token,
};
pub use local_api::{LocalApiNotice, LocalApiStatus};
pub use queue::{
    PlayingFrom, QueueAction, QueueInfo, add_input, back_to_inputs, playing_from, queue_edit,
    queue_info, with_likes,
};
pub use silences::Mark;

use crate::catalog::{Catalog, Source};
use crate::presence::Presence;
use crate::scrobbler::{Scrobblers, Sent};
use crate::updates;

/// How often the player gets the engine's position and the time, while it
/// `needs_ticks()` (playing or loading). Paused, the timer is off.
const TICK: Duration = Duration::from_millis(250);
/// How often the offline scrobble queue is retried (the first run is at startup).
const FLUSH_SCROBBLES: Duration = Duration::from_secs(5 * 60);

/// When the yt-dlp and Deno versions are read, after the start.
const TOOL_VERSIONS_AFTER: Duration = Duration::from_secs(10);

/// How often a long session asks for the daily `yt-dlp -U`.
const DAILY_UPDATE: Duration = Duration::from_secs(24 * 3600);

/// The rows on screen resolved ahead of a click at most (`Action::Prepare`), the top first.
pub const PREPARE_ROWS: usize = 12;

/// How long after a failed helpers download a song's start tries it again in the background.
const HELPERS_RETRY: Duration = Duration::from_secs(10 * 60);

/// Where the resolver preprocesses a new player: in a short-lived child of this exe
/// (`main` runs `featherplay_streams::preprocess_stdio` for it and nothing else), so its
/// ~220 MB peak and a crash stay out of the app. Without the exe's own path, here.
fn preprocessor() -> featherplay_streams::Preprocessor {
    match std::env::current_exe() {
        Ok(exe) => featherplay_streams::Preprocessor::Child(exe),
        Err(_) => featherplay_streams::Preprocessor::InProcess,
    }
}

/// Downloads the helpers if they're missing; true when they're there. A failure is logged
/// by the download and kept for Copy diagnostics: no toast, as no song waits for it.
async fn install_helpers(resolver: &YtDlpResolver) -> bool {
    match resolver.install_helpers().await {
        Ok(()) => true,
        Err(e) => {
            if let ResolveError::Sidecar(s) = &e {
                crate::diagnostics::helpers_failed(s);
            }
            false
        }
    }
}

/// Time since the process started; the first call starts the clock.
pub use crate::diagnostics::uptime;

#[derive(Debug, Default)]
pub struct Options {
    /// Play this track's radio instead of restoring the session.
    pub play: Option<TrackId>,
    /// Volume for this run, instead of the one in the settings (which is left alone).
    pub volume: Option<f32>,
    /// Start the first track here.
    pub start_at: Option<Duration>,
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
    pub upcoming: Vec<Track>,
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
    /// What Skip silence drops from the current track, for the seek bar (`silences.rs`);
    /// empty while it's off, and from a track change until the new track is scanned.
    Silences(Vec<Mark>),
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

/// Results of background work.
enum Msg {
    Resolved(TrackId, Result<ResolvedStream, ResolveError>),
    /// This song's resolve started to wait for the helpers' download: its native request
    /// gave no stream, and yt-dlp isn't there yet (a first run).
    HelpersWait(TrackId),
    /// This song's resolve ahead got a URL with ads before the song and waits them out.
    GateWait(TrackId),
    Continuation(TrackId, Result<Vec<Track>, ApiError>),
    Audio(AudioEvent),
    Scrobbled(Sent),
    Segments(TrackId, Result<Vec<Segment>, String>),
    Radio(TrackId, Result<Vec<Track>, ApiError>),
    /// A music video's watch playlist, asked for its song version (`song_versions.rs`).
    SongVersion(TrackId, Result<Vec<Track>, ApiError>),
    Rated(TrackId, LikeStatus, Result<(), ApiError>),
    /// Sign-in, sign-out and Last.fm results.
    Account(account::AccountMsg),
    /// The Discord worker's connection changed.
    Discord(crate::presence::Status),
    /// A reconnect with the stored session; `None`: there's no stored session any more.
    Reconnected(Option<Result<YtMusic, ApiError>>),
    /// The sign-in behind a resumed start (`YtMusic::refresh`): the fresh client version.
    Refreshed(Result<Option<String>, ApiError>),
    /// The link for the "may need an update" toast, and a newer release if one is out.
    Help(Option<updates::Help>, Option<updates::Release>),
    /// The Amuse endpoint started, or why not; the number is the start's (`start_amuse`).
    Amuse(
        u32,
        std::io::Result<featherplay_integrations::local_api::Amuse>,
    ),
}

pub struct Controller {
    player: Player,
    api: Option<YtMusic>,
    resolver: Arc<YtDlpResolver>,
    audio: AudioEngine,
    /// Last.fm and ListenBrainz, each `None` inside when off.
    scrobblers: Scrobblers,
    /// `None` when Discord is off.
    presence: Option<Presence>,
    /// The current track's SponsorBlock segments not skipped yet.
    segments: Option<(TrackId, Vec<Segment>)>,
    settings: Settings,
    paths: Paths,
    volume: Option<f32>,
    /// Inputs to feed once the loop runs: the restored session or the `--play` radio.
    initial: Vec<Input>,
    /// Resolved streams and the resolves running (`streams.rs`).
    streams: streams::Streams,
    /// Music videos' song versions looked up (`song_versions.rs`).
    song_versions: song_versions::SongVersions,
    tx: mpsc::UnboundedSender<Msg>,
    rx: mpsc::UnboundedReceiver<Msg>,
    last_state: Option<(Status, Option<TrackId>)>,
    ui: UiLink,
    /// The last `NowPlaying` shown (Discord), and whether the sound had started then.
    shown: Option<(NowPlaying, bool)>,
    /// The last state sent to the window.
    shown_state: Option<PlayerState>,
    /// The engine's output is starved (`AudioEvent::Buffering`): the position stands still.
    buffering: bool,
    /// Likes changed this session. The queue takes them (`Input::SetLike`), and so do the
    /// tracks that come into it later (`with_likes`).
    likes: HashMap<TrackId, LikeStatus>,
    /// Where the queue came from, for the Up next panel and Now playing's header.
    playing_from: PlayingFrom,
    /// Sign-in and Last.fm state.
    account: account::AccountState,
    /// The stored session couldn't connect (network or parse failure): the reconnects.
    unreachable: Option<reconnect::Reconnect>,
    /// `api` is the stored session resumed at startup, and its sign-in behind the start
    /// hasn't answered yet (`reconnect::stored_session`, `refresh_session`).
    resumed: bool,
    /// The link for the "may need an update" toast, looked up once a session (`notice.rs`).
    lookup: notice::Lookup,
    /// The local API's server, when it's on.
    local_api: local_api::ApiServer,
    /// When the daily `yt-dlp -U` was last asked for: at startup, then when a song starts
    /// playing a day later (`update_daily`). No timer.
    daily_update: Instant,
    /// When the helpers' download was last started in the background: at startup, then
    /// when a song starts while they're still missing (`retry_helpers`). No timer.
    helpers_tried: Instant,
    /// The current track's silences, once scanned (`silences.rs`).
    silences: Option<silences::TrackSilences>,
    /// The songs scanned before, read on first use.
    silence_cache: Option<featherplay_store::SilenceCache>,
    /// The seek bar's marks last sent.
    shown_marks: Option<Vec<Mark>>,
}

impl Controller {
    pub async fn start(opts: Options, ui: UiLink) -> Result<Controller, String> {
        let paths = Paths::resolve();
        let settings = Settings::load(&paths);
        log!(
            "config {}, cache {}",
            paths.config.display(),
            paths.cache.display()
        );
        // The proxy the HTTP clients and yt-dlp go through, and where it came from. A proxy
        // script or auto-detection is asked from here on, in the background, and logs its
        // answer itself (`featherplay_proxy`).
        log!("proxy: {}", featherplay_proxy::summary());

        // A dev build may point at helpers elsewhere.
        let sidecars = std::env::var_os("FEATHERPLAY_SIDECARS_DIR")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| paths.cache.join("sidecars"));
        let probe = Sidecars {
            dir: sidecars.clone(),
        };
        let (tx, rx) = mpsc::unbounded_channel();
        let (waits, gates) = (tx.clone(), tx.clone());
        let resolver = Arc::new(
            YtDlpResolver::new(Sidecars { dir: sidecars }, None)
                .with_downloads(Downloads::pinned())
                .with_preprocessor(preprocessor())
                .on_helpers_wait(move |id| {
                    let _ = waits.send(Msg::HelpersWait(id.clone()));
                })
                .on_gate_wait(move |id| {
                    let _ = gates.send(Msg::GateWait(id.clone()));
                }),
        );
        resolver.set_quality(stream_quality(settings.streaming_quality));
        log!("player challenges: {}", featherplay_streams::js_engine());
        // The performance harness's fixtures play nothing: no helpers to fetch or start.
        #[cfg(feature = "perf-harness")]
        let helpers_wanted = !crate::perf::fixtures();
        #[cfg(not(feature = "perf-harness"))]
        let helpers_wanted = true;
        if helpers_wanted {
            // A first run fetches and preprocesses YouTube's player now (seconds), so the
            // native request is ready for the first click, signed in or not.
            let native = resolver.clone();
            crate::task::spawn_detached(async move { native.prepare().await });
            let helpers = resolver.clone();
            crate::task::spawn_detached(async move {
                // A first run also downloads yt-dlp and deno now, in the background: only a
                // song the native request can't give waits for them (`HelpersWait`). It logs
                // its own start and end; a failure is tried again later (`retry_helpers`).
                let installed = install_helpers(&helpers).await;
                // After the startup's own work: the daily `yt-dlp -U` (skipped when the last one,
                // or the download, is under a day old), then once, for the log and Copy
                // diagnostics, the versions. Short processes, off this task.
                tokio::time::sleep(TOOL_VERSIONS_AFTER).await;
                if installed {
                    helpers.update_daily().await;
                }
                let (ytdlp, deno) = probe.versions().await;
                crate::diagnostics::set_tool_versions(ytdlp, deno);
            });
        }

        let mut initial = Vec::new();
        let mut unreachable = None;
        let mut resumed = false;
        let api = match cookie_header() {
            None => {
                log!("no cookies: signed out");
                None
            }
            Some(cookies) => {
                let identity = account::saved_identity(&settings);
                let (result, at_once) =
                    reconnect::stored_session(&cookies, &identity, &paths).await;
                match reconnect::outcome(result) {
                    reconnect::Outcome::Live(api) => {
                        resumed = at_once;
                        if resumed {
                            log!("signed in with the kept client version");
                        } else {
                            log!("signed in");
                        }
                        Some(*api)
                    }
                    reconnect::Outcome::Expired => {
                        initial.push(Input::AuthExpired);
                        None
                    }
                    reconnect::Outcome::Unreachable => {
                        unreachable = Some(reconnect::Reconnect::after_failure(Instant::now()));
                        None
                    }
                }
            }
        };

        if let Some(api) = &api {
            ui.catalog.set(Source::Live(api.clone()));
        } else if unreachable.is_some() {
            ui.catalog.set(Source::Unreachable);
        }

        // A crash or a killed process leaves buffered audio behind.
        let cleared = featherplay_audio::clear_stale_temp(&paths.cache.join("tmp"));
        if cleared > 0 {
            log!("removed {cleared} stale temp file(s)");
        }
        // And a browser sign-in's temporary profile.
        crate::pages::sign_in::sweep_browser_profiles(&paths.cache);
        let audio = AudioEngine::new(settings.output_device.as_deref(), paths.cache.join("tmp"))?;
        let player = Player::new(prefs(&settings));

        if let Some(seed) = opts.play {
            let api = api.as_ref().ok_or("--play needs a signed-in session")?;
            let tracks = api
                .radio(&seed)
                .await
                .map_err(|e| format!("radio for {}: {e}", seed.0))?;
            let start = tracks.iter().position(|t| t.id == seed);
            log!(
                "radio for {}: {} tracks, seed at {start:?}",
                seed.0,
                tracks.len()
            );
            if tracks.is_empty() {
                return Err(format!("radio for {} is empty", seed.0));
            }
            initial.push(Input::PlayTracks {
                tracks,
                start: start.unwrap_or(0),
                radio: true,
            });
        } else if let Some(session) = load_session(&paths) {
            log!(
                "restoring {} tracks at index {}",
                session.queue.len(),
                session.index
            );
            initial.push(Input::Restore(session));
        }
        if let Some(at) = opts.start_at {
            initial.push(Input::SeekTo(at));
        }

        let events = audio.events();
        let audio_tx = tx.clone();
        std::thread::Builder::new()
            .name("featherplay-audio-events".into())
            .spawn(move || {
                // Ends when the engine (the sender) is dropped.
                while let Ok(ev) = events.recv() {
                    if audio_tx.send(Msg::Audio(ev)).is_err() {
                        break;
                    }
                }
            })
            .map_err(|e| format!("audio event thread: {e}"))?;

        let scrobble_tx = tx.clone();
        let scrobblers = Scrobblers::new(
            &settings,
            &paths,
            Arc::new(move |sent| {
                let _ = scrobble_tx.send(Msg::Scrobbled(sent));
            }),
        );

        let presence = account::start_presence(&settings, &tx);
        let playing_from = queue::initial_playing_from(&initial);

        Ok(Controller {
            player,
            api,
            resolver,
            audio,
            scrobblers,
            presence,
            segments: None,
            settings,
            paths,
            volume: opts.volume,
            initial,
            streams: Default::default(),
            song_versions: Default::default(),
            tx,
            rx,
            last_state: None,
            ui,
            shown: None,
            shown_state: None,
            buffering: false,
            likes: HashMap::new(),
            playing_from,
            account: Default::default(),
            unreachable,
            resumed,
            lookup: Default::default(),
            local_api: Default::default(),
            daily_update: Instant::now(),
            helpers_tried: Instant::now(),
            silences: None,
            silence_cache: None,
            shown_marks: None,
        })
    }

    /// Runs until Ctrl+C or `UiCommand::Quit`, then saves the session.
    pub async fn run(mut self) {
        self.audio.set_volume(self.slider_volume());
        self.audio.set_equalizer(equalizer_bands(&self.settings));
        self.audio
            .set_compressor(compressor(self.settings.loudness));
        self.audio.set_match_volumes(self.settings.match_volumes);
        self.audio
            .set_skip_silences(skip_silences(self.settings.skip_silence));
        self.audio.set_crossfade(self.settings.crossfade > 0);
        self.audio.set_speed(playback_speed(&self.settings));
        self.restore_streams();
        // Unreachable keeps the signed-in look: the session is likely fine, and signed out
        // would send the window to Sign in.
        let signed_in = self.api.is_some() || self.unreachable.is_some();
        // The performance harness's fixtures stand for an account.
        #[cfg(feature = "perf-harness")]
        let signed_in = signed_in || crate::perf::fixtures();
        (self.ui.events)(UiEvent::Account { signed_in });
        self.check_identity();
        self.refresh_session();
        if self.presence.is_none() {
            (self.ui.events)(UiEvent::Discord(crate::presence::off_status()));
        }
        for input in std::mem::take(&mut self.initial) {
            self.input(input);
        }
        // The restored track, so Play starts it at once.
        if let Some(t) = self.player.queue().current().cloned() {
            self.prepare_restored(&t);
        }
        // The volume, even with nothing to play.
        self.push_state();
        if self.settings.local_api {
            self.start_local_api();
        }
        if self.settings.amuse {
            self.start_amuse();
        }
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut ticking = false;
        // Fires at once, so the offline queue is flushed at startup.
        let mut flush = tokio::time::interval(FLUSH_SCROBBLES);
        flush.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let ctrl_c = tokio::signal::ctrl_c();
        tokio::pin!(ctrl_c);
        // A handler that couldn't be installed reports it once; it mustn't read as Ctrl+C
        // (a release build has no console to press it in).
        let mut ctrl_c_live = true;
        loop {
            // Paused, idle or stopped, nothing wakes this loop but its inputs.
            let wanted = self.player.needs_ticks();
            if wanted && !ticking {
                tick.reset();
            }
            ticking = wanted;
            // The next automatic reconnect; no timer otherwise.
            let reconnect_due = self
                .unreachable
                .as_ref()
                .and_then(reconnect::Reconnect::due);
            let reconnect_at = reconnect_due.map_or_else(tokio::time::Instant::now, Into::into);
            tokio::select! {
                pressed = &mut ctrl_c, if ctrl_c_live => {
                    if let Err(e) = pressed {
                        log!("Ctrl+C handler: {e}");
                        ctrl_c_live = false;
                    } else {
                        log!("Ctrl+C");
                        break;
                    }
                }
                _ = tick.tick(), if ticking => {
                    let position = self.tick();
                    self.skip_segment(position);
                }
                _ = flush.tick(), if self.scrobblers.is_on() => self.scrobblers.flush(),
                _ = tokio::time::sleep_until(reconnect_at), if reconnect_due.is_some() => {
                    self.reconnect();
                }
                Some(msg) = self.rx.recv() => self.message(msg),
                cmd = self.ui.commands.recv() => {
                    // `None`: every sender is gone, so the UI has shut down.
                    let cmd = cmd.unwrap_or(UiCommand::Quit);
                    log!("ui command {cmd:?}");
                    match cmd {
                        UiCommand::PlayPause => self.input(Input::Toggle),
                        UiCommand::Next => self.input(Input::Next),
                        UiCommand::Previous => self.input(Input::Previous),
                        UiCommand::Show => (self.ui.events)(UiEvent::ShowWindow),
                        UiCommand::Quit => break,
                        other => self.api_command(other),
                    }
                }
                Some(action) = self.ui.actions.recv() => {
                    if matches!(action, Action::Quit) {
                        log!("quit from the window");
                        break;
                    }
                    self.action(action);
                }
            }
        }
        self.audio.pause();
        self.persist();
        self.save_streams(true);
        if let Some(presence) = self.presence.take() {
            presence.quit();
        }
    }

    fn action(&mut self, action: Action) {
        match action {
            Action::Toggle => self.input(Input::Toggle),
            Action::Next => self.input(Input::Next),
            Action::Previous => self.input(Input::Previous),
            Action::Play {
                tracks,
                start,
                radio,
            } => {
                log!("play {} tracks from {start}", tracks.len());
                self.playing_from = playing_from(&tracks, start, radio);
                self.input(Input::PlayTracks {
                    tracks,
                    start,
                    radio,
                });
            }
            Action::Radio(seed) => {
                log!("fetch radio for {}", seed.0);
                // The catalog's source: the account's, or signed out the visitor's.
                let source = self.ui.catalog.source();
                let tx = self.tx.clone();
                crate::task::spawn_detached(async move {
                    let result = source.radio(&seed).await;
                    let _ = tx.send(Msg::Radio(seed, result));
                });
            }
            Action::SeekTo(to) => self.input(Input::SeekTo(to)),
            Action::BackTo { track, at } => {
                let inputs = back_to_inputs(self.player.queue(), &track, at);
                self.inputs(inputs);
            }
            Action::SetVolume { value, save } => self.set_volume(value, save),
            Action::Rate(id, status) => {
                let Some(api) = self.api.clone() else {
                    log!("rate: signed out");
                    self.push_state();
                    return;
                };
                log!("rate {} {status:?}", id.0);
                let tx = self.tx.clone();
                crate::task::spawn_detached(async move {
                    let result = api.rate(&id, status).await;
                    let _ = tx.send(Msg::Rated(id, status, result));
                });
            }
            Action::Rated(id, status) => {
                self.input(Input::SetLike(id.clone(), status));
                self.likes.insert(id, status);
                self.shown_state = None;
                self.push_state();
            }
            // The song menu's Play next and Add to queue are the Up next panel's edits.
            Action::PlayNext(_) | Action::AddToQueue(_) => {
                if let Some(edit) = queue_edit(action) {
                    self.queue_action(edit);
                }
            }
            Action::AddToPlaylist { playlist, tracks } => {
                let Some(api) = self.api.clone() else {
                    log!("add to playlist: signed out");
                    return;
                };
                log!("add {} track(s) to playlist {}", tracks.len(), playlist.0);
                crate::task::spawn_detached(async move {
                    match api.add_to_playlist(&playlist, &tracks).await {
                        Ok(()) => log!("added to playlist {}", playlist.0),
                        Err(e) => log!("add to playlist {} failed: {e}", playlist.0),
                    }
                });
            }
            Action::SetShuffle(on) => self.input(Input::SetShuffle(on)),
            Action::SetRepeat(mode) => self.input(Input::SetRepeat(mode)),
            Action::Queue(a) => self.queue_action(a),
            Action::Account(a) => self.account_action(a),
            Action::Reconnect => self.reconnect(),
            Action::Prefetch { id, track } => match track {
                Some(t) => {
                    self.look_up_song_version(&t);
                    self.hint(std::slice::from_ref(&t));
                }
                None => self.hint_id(id),
            },
            Action::Prepare(rows) => self.prepare(rows),
            Action::WakeOutput => self.audio.wake(),
            // `run` stops for it before it gets here.
            Action::Quit => {}
        }
    }

    /// The volume, 0..1. `save` (the slider let go, the local API) also stores it in the
    /// settings and shows it.
    fn set_volume(&mut self, value: f32, save: bool) {
        let value = if value.is_finite() {
            value.clamp(0.0, 1.0)
        } else {
            0.0
        };
        self.audio.set_volume(value);
        self.volume = Some(value);
        if save {
            // The slider replaces a `--volume` override from here on.
            self.volume = None;
            self.settings.volume = value;
            if let Err(e) = self.settings.save(&self.paths) {
                log!("saving the settings failed: {}", e.kind());
            }
            self.push_state();
        }
    }

    /// The local API's controls, which the media card and tray don't send.
    fn api_command(&mut self, cmd: UiCommand) {
        let position = self.player.position();
        let length = self.player.queue().current().and_then(|t| t.duration);
        match cmd {
            UiCommand::Play if !self.player.wants_play() => self.input(Input::Toggle),
            UiCommand::Pause if self.player.wants_play() => self.input(Input::Toggle),
            UiCommand::SeekTo(to) => {
                self.input(Input::SeekTo(length.map_or(to, |end| to.min(end))))
            }
            UiCommand::SeekBack(by) => {
                self.input(Input::SeekTo(local_api::skip(position, length, by, false)))
            }
            UiCommand::SeekForward(by) => {
                self.input(Input::SeekTo(local_api::skip(position, length, by, true)))
            }
            UiCommand::SetVolume(v) => self.set_volume(f32::from(v.min(100)) / 100.0, true),
            UiCommand::ToggleShuffle => self.input(Input::SetShuffle(!self.player.shuffle())),
            UiCommand::CycleRepeat(n) => {
                let mode = local_api::cycle_repeat(self.player.repeat(), n);
                self.input(Input::SetRepeat(mode));
            }
            // Play while playing, pause while paused, and the commands `run` handles.
            _ => {}
        }
    }

    /// The volume slider's value for this run.
    fn slider_volume(&self) -> f32 {
        self.volume.unwrap_or(self.settings.volume)
    }

    /// Feeds `first` to the player and runs the effects, including any inputs they produce.
    /// Without the tick timer, a tick before brings the player's clock up to date (stream
    /// expiry, scrobble time), and a tick after reads the engine's position and saves a
    /// changed session: a pause gets its final tick here.
    fn input(&mut self, first: Input) {
        if !self.player.needs_ticks() {
            self.tick();
        }
        self.feed(first);
        if !self.player.needs_ticks() {
            self.tick();
        }
    }

    /// As `input`, for inputs that go in together: no tick between them reads the engine's
    /// position back over a seek they made.
    fn inputs(&mut self, inputs: Vec<Input>) {
        if inputs.is_empty() {
            return;
        }
        if !self.player.needs_ticks() {
            self.tick();
        }
        for input in inputs {
            self.feed(input);
        }
        if !self.player.needs_ticks() {
            self.tick();
        }
    }

    /// One `Tick` with the engine's position; returns that position.
    fn tick(&mut self) -> Duration {
        let head = self.audio.playhead();
        self.feed(Input::Tick {
            position: head.position,
            now: SystemTime::now(),
            music_end: head.music_end,
        });
        head.position
    }

    fn feed(&mut self, first: Input) {
        let mut inputs = VecDeque::from([first]);
        while let Some(input) = inputs.pop_front() {
            let input = if self.settings.prefer_song_versions {
                let now = SystemTime::now();
                song_versions::with_song_versions(
                    input,
                    &self.song_versions,
                    self.player.queue().current().is_none(),
                    |id| self.streams.readiness(id, now),
                )
            } else {
                input
            };
            for effect in self.player.handle(with_likes(input, &self.likes)) {
                inputs.extend(self.effect(effect));
            }
        }
    }

    fn effect(&mut self, effect: Effect) -> Option<Input> {
        match effect {
            Effect::Resolve(id) => return self.resolve(id),
            Effect::Load { id, start } => {
                let Some(src) = self.source(&id) else {
                    // Dropped from the cache (a long session of hints, or a refused
                    // preload): resolve it again.
                    log!("load {}: no stream kept, resolving again", id.0);
                    return Some(Input::StreamExpired);
                };
                log!("load {} at {:.1}s", id.0, start.as_secs_f64());
                self.track_loaded(&id, &src.url);
                self.audio.load(src.clone(), start);
                self.loaded(src);
            }
            Effect::Crossfade { id, fade } => {
                let Some(src) = self.source(&id) else {
                    log!("crossfade to {}: no stream kept, resolving again", id.0);
                    return Some(Input::StreamExpired);
                };
                log!("crossfade to {} over {:.1}s", id.0, fade.as_secs_f64());
                self.track_loaded(&id, &src.url);
                self.audio.crossfade_to(src.clone(), fade);
                self.loaded(src);
            }
            Effect::FadeOut(fade) => self.audio.fade_out(fade),
            Effect::Preload(id) => {
                if let Some(src) = self.source(&id) {
                    log!("preload {} ({})", id.0, self.time_left());
                    let _ = self.known_silences(&id, &src.url);
                    self.audio.preload(src);
                }
            }
            Effect::Play => self.audio.play(),
            Effect::Pause => self.audio.pause(),
            Effect::Seek(to) => {
                self.audio.seek(to);
                // Always resent: a restart from Previous seeks to 0, which equals the
                // position shown when the track started, and the media card and Discord
                // timers run on from the last update.
                self.shown = None;
                self.shown_state = None;
                self.push_now_playing();
            }
            Effect::FetchContinuation(last) => {
                if let Some(tracks) = self.song_versions.radio(&last, Instant::now()) {
                    log!(
                        "radio continuation after {}: the song version lookup's, {} tracks",
                        last.0,
                        tracks.len()
                    );
                    return Some(Input::ContinuationLoaded(last, tracks));
                }
                log!("fetch radio continuation after {}", last.0);
                // Signed out too: the radio is the catalogue's.
                let source = self.ui.catalog.source();
                let tx = self.tx.clone();
                crate::task::spawn_detached(async move {
                    let result = source.radio(&last).await;
                    let _ = tx.send(Msg::Continuation(last, result));
                });
            }
            Effect::TrackStarted(t) => {
                let artists: Vec<&str> = t.artists.iter().map(|a| a.name.as_str()).collect();
                log!(
                    "now playing {}: {} - {} ({})",
                    t.id.0,
                    artists.join(", "),
                    t.title,
                    t.duration
                        .map(|d| format!("{}s", d.as_secs()))
                        .unwrap_or_else(|| "?".into())
                );
                if self.settings.notifications {
                    let np = now_playing(&t, true, Duration::ZERO);
                    // A few ms of WinRT calls; kept off this loop and the UI thread.
                    crate::task::blocking_detached(&tokio::runtime::Handle::current(), move || {
                        notify_track(&np)
                    });
                }
                let id = self.player.play_id(&t);
                if let Some(s) = self.streams.get(&id) {
                    crate::diagnostics::song_started(&id.0, s.via.label());
                }
                self.fetch_segments(&t);
                let alt = self.alternates(&t.id);
                self.scrobblers.now_playing(&t, alt);
                self.hint_queue();
            }
            Effect::Scrobble(t, started) => {
                log!("scrobble {}: {}", t.id.0, t.title);
                self.scrobblers.scrobble(&t, started);
            }
            Effect::ShowError(stop) => self.stopped(stop),
            Effect::ShowReauth => self.session_expired(),
            Effect::PersistSession => {
                self.persist();
                self.save_streams(false);
            }
            Effect::StateChanged => {
                self.log_state();
                self.push_now_playing();
                // A queue edit may have changed the tracks ahead.
                match self.player.status() {
                    Status::Playing => self.hint_queue(),
                    Status::Loading => {}
                    Status::Idle | Status::Paused | Status::Stopped(_) => self.stop_ahead(),
                }
            }
        }
        None
    }

    /// A song started: in a session that has run a day since the last ask, the daily
    /// `yt-dlp -U` in the background (it skips itself when the last update is fresh).
    fn update_daily(&mut self) {
        if self.daily_update.elapsed() < DAILY_UPDATE {
            return;
        }
        self.daily_update = Instant::now();
        let resolver = self.resolver.clone();
        crate::task::spawn_detached(async move {
            resolver.update_daily().await;
        });
    }

    /// A song started while the helpers are still missing (their download at startup
    /// failed): once more in the background, at most every `HELPERS_RETRY`. No toast, as
    /// nothing waits for it; a song that needs them waits for this same download.
    fn retry_helpers(&mut self) {
        #[cfg(feature = "perf-harness")]
        if crate::perf::fixtures() {
            return;
        }
        if self.helpers_tried.elapsed() < HELPERS_RETRY || self.resolver.helpers_installed() {
            return;
        }
        self.helpers_tried = Instant::now();
        let resolver = self.resolver.clone();
        crate::task::spawn_detached(async move {
            install_helpers(&resolver).await;
        });
    }

    fn message(&mut self, msg: Msg) {
        let input = match msg {
            Msg::HelpersWait(id) => {
                self.helpers_wait(&id);
                return;
            }
            Msg::GateWait(id) => {
                // A row on screen waiting out its gate needs no slot meanwhile.
                self.streams.at_gate(&id);
                self.start_hints();
                return;
            }
            Msg::Resolved(id, Err(ResolveError::Deferred)) => {
                self.deferred(id);
                return;
            }
            Msg::Resolved(id, result) => {
                let keep = self.in_use();
                let (took, ahead) = self.streams.done(&id, result.as_ref().ok(), &keep);
                let took = took.as_secs_f64();
                // A slot for the next hint.
                self.start_hints();
                match result {
                    Ok(stream) => {
                        log!(
                            "resolved {} in {took:.1}s: itag {}, {} kbps, {}, via {}",
                            id.0,
                            stream.itag,
                            stream.bitrate_kbps,
                            client(&stream.url),
                            stream.via.label()
                        );
                        match ahead {
                            // A row on screen: its start is fetched once the pointer rests
                            // on it (`streams.rs`).
                            Some(streams::Hint::Page) => {}
                            Some(_) if keep.first() == Some(&id) => {
                                // The restored track, not loaded yet: buffer all of it, so
                                // Play starts at once at the saved position too.
                                self.audio.preload(source(&stream));
                            }
                            Some(_) => {
                                // Not asked for yet: fetch its start, so a click plays from
                                // memory.
                                self.audio.prefetch(&source(&stream));
                            }
                            None => {}
                        }
                        Input::Resolved {
                            id,
                            result: Ok(stream_info(&stream)),
                        }
                    }
                    Err(e) if ahead == Some(streams::Hint::Page) => {
                        // Nobody waits for it: no failure to count or show.
                        log!("prepare {} in {took:.1}s: {e}", id.0);
                        return;
                    }
                    Err(e) => {
                        let failure = notice::failure(&e);
                        crate::diagnostics::resolve_failed(&id.0, took, &failure.detail);
                        if let ResolveError::Sidecar(s) = &e {
                            crate::diagnostics::helpers_failed(s);
                        }
                        Input::Resolved {
                            id,
                            result: Err(failure),
                        }
                    }
                }
            }
            Msg::Continuation(last, result) => match result {
                Ok(tracks) => {
                    log!("continuation after {}: {} tracks", last.0, tracks.len());
                    self.song_versions.learn(&tracks);
                    Input::ContinuationLoaded(last, tracks)
                }
                Err(e) => {
                    log!("continuation after {} failed: {e}", last.0);
                    if e == ApiError::AuthExpired {
                        self.input(Input::AuthExpired);
                    }
                    // Clears the request; the player won't ask again for the same track.
                    Input::ContinuationLoaded(last, Vec::new())
                }
            },
            Msg::Segments(id, result) => {
                match result {
                    Ok(segments) => {
                        let list: Vec<String> = segments
                            .iter()
                            .map(|s| {
                                format!("{:.1}-{:.1}s", s.start.as_secs_f64(), s.end.as_secs_f64())
                            })
                            .collect();
                        log!(
                            "sponsorblock {}: {} segment(s) {}",
                            id.0,
                            segments.len(),
                            list.join(", ")
                        );
                        let current = self.player.queue().current().map(|t| &t.id);
                        if current == Some(&id) && !segments.is_empty() {
                            self.segments = Some((id, segments));
                        }
                    }
                    // Nothing is skipped for this track.
                    Err(e) => log!("sponsorblock {} failed: {e}", id.0),
                }
                return;
            }
            Msg::Account(m) => {
                self.account_msg(m);
                return;
            }
            Msg::Reconnected(result) => {
                self.reconnected(result);
                return;
            }
            Msg::Refreshed(result) => {
                self.refreshed(result);
                return;
            }
            Msg::Amuse(start, result) => {
                self.amuse_started(start, result);
                return;
            }
            Msg::Help(help, release) => {
                self.helped(help, release);
                return;
            }
            Msg::Discord(status) => {
                // A late word from a worker that was just stopped doesn't count.
                if self.presence.is_some() {
                    (self.ui.events)(UiEvent::Discord(status));
                }
                return;
            }
            Msg::SongVersion(video, result) => {
                self.song_version_found(video, result);
                return;
            }
            Msg::Scrobbled(sent) => {
                self.scrobblers.sent(sent);
                return;
            }
            Msg::Radio(seed, result) => match result {
                Ok(tracks) if !tracks.is_empty() => {
                    self.song_versions.learn(&tracks);
                    let start = tracks.iter().position(|t| t.id == seed).unwrap_or(0);
                    log!("radio for {}: {} tracks", seed.0, tracks.len());
                    self.playing_from = playing_from(&tracks, start, true);
                    Input::PlayTracks {
                        tracks,
                        start,
                        radio: true,
                    }
                }
                Ok(_) => {
                    log!("radio for {} is empty", seed.0);
                    return;
                }
                Err(e) => {
                    log!("radio for {} failed: {e}", seed.0);
                    if e == ApiError::AuthExpired {
                        Input::AuthExpired
                    } else {
                        return;
                    }
                }
            },
            Msg::Rated(id, status, result) => {
                match result {
                    Ok(()) => {
                        // The queue's copy too, so a disliked track is passed over when it
                        // comes round, and the rating is saved with the session.
                        self.input(Input::SetLike(id.clone(), status));
                        self.likes.insert(id, status);
                    }
                    Err(e) => {
                        log!("rate {} failed: {e}", id.0);
                        if e == ApiError::AuthExpired {
                            self.input(Input::AuthExpired);
                        }
                    }
                }
                // Also on failure: the bar flipped at once and goes back.
                self.shown_state = None;
                self.push_state();
                return;
            }
            Msg::Audio(ev) => match ev {
                AudioEvent::Started => {
                    self.update_daily();
                    self.retry_helpers();
                    Input::AudioStarted
                }
                AudioEvent::Ended => {
                    log!("track ended");
                    Input::AudioEnded
                }
                AudioEvent::Buffering(on) => {
                    log!("buffering {}", if on { "started" } else { "ended" });
                    // The bar's and Discord's clocks run on their own between states: stop
                    // the bar while the sound does, and restart both from the engine's
                    // position after.
                    self.buffering = on;
                    self.tick();
                    self.push_now_playing();
                    return;
                }
                AudioEvent::Skipped => {
                    // Skipped silence moved the position on: like a seek, the bar's and
                    // Discord's clocks restart from the engine's position.
                    self.tick();
                    self.shown = None;
                    self.shown_state = None;
                    self.push_now_playing();
                    return;
                }
                AudioEvent::DeviceLost => {
                    log!("audio device lost");
                    Input::DeviceLost
                }
                AudioEvent::NoDevice => {
                    // The engine stayed paused; the player shows it, as for a lost device.
                    log!("play: no audio output device");
                    (self.ui.events)(UiEvent::ToastError(no_sound_output()));
                    Input::DeviceLost
                }
                AudioEvent::Error(msg) if msg.contains("HTTP 403") => {
                    log!("stream URL expired ({msg}), resolving again");
                    // The re-resolve must not get the same URL back from the cache.
                    let current = self
                        .player
                        .queue()
                        .current()
                        .map(|t| self.player.play_id(t));
                    if let Some(id) = current
                        && let Some(s) = self.streams.forget(&id)
                        && !self.streams.is_from_last_session(&id)
                    {
                        self.resolver.refused(&id, &s.url);
                    }
                    Input::StreamExpired
                }
                AudioEvent::PreloadRefused(url) => {
                    self.preload_refused(&url);
                    return;
                }
                AudioEvent::Silences { url, silences } => {
                    self.silences_found(&url, &silences);
                    return;
                }
                AudioEvent::Error(msg) => {
                    log!("audio error: {msg}");
                    Input::AudioFailed(msg)
                }
            },
        };
        self.input(input);
    }

    /// Looks up the non-music segments of a music video that just started.
    fn fetch_segments(&mut self, t: &Track) {
        self.segments = None;
        if !self.settings.sponsorblock || !sponsorblock::applies(t.video_type) {
            return;
        }
        let (id, tx) = (t.id.clone(), self.tx.clone());
        crate::task::spawn_detached(async move {
            let result = sponsorblock::fetch_segments(&id).await;
            let _ = tx.send(Msg::Segments(id, result));
        });
    }

    /// Seeks past the segment `position` is in, once per segment.
    fn skip_segment(&mut self, position: Duration) {
        let Some((id, segments)) = &mut self.segments else {
            return;
        };
        if self.player.queue().current().map(|t| &t.id) != Some(&*id)
            || !matches!(self.player.status(), Status::Playing)
        {
            return;
        }
        let Some(target) = sponsorblock::skip_target(segments, position) else {
            return;
        };
        segments.retain(|s| s.end != target);
        log!(
            "sponsorblock: skipping {:.1}s to {:.1}s",
            position.as_secs_f64(),
            target.as_secs_f64()
        );
        self.input(Input::SeekTo(target));
    }

    /// The video's own title and first tag from `id`'s stream, for the scrobblers.
    fn alternates(&self, id: &TrackId) -> Alternates {
        self.streams
            .get(id)
            .map(|s| Alternates::new(s.video_title.as_deref(), s.first_tag.as_deref()))
            .unwrap_or_default()
    }

    fn source(&self, id: &TrackId) -> Option<StreamSource> {
        self.streams.get(id).map(source)
    }

    /// The last session's streams that are still good (`Streams::restore`), so a click on
    /// a song played or resolved ahead then needs no resolve (nor its ads' wait).
    fn restore_streams(&mut self) {
        let path = self.paths.cache.join(streams::SAVED_STREAMS);
        if let Ok(text) = std::fs::read_to_string(&path) {
            self.streams.restore(&text, SystemTime::now());
        }
    }

    fn persist(&self) {
        if self.player.queue().current().is_none() {
            return;
        }
        let session = self.player.session();
        match save_session(&session, &self.paths) {
            Ok(()) => log!(
                "session saved: {} tracks, index {}, at {:.1}s",
                session.queue.len(),
                session.index,
                session.position_ms as f64 / 1000.0
            ),
            Err(e) => log!("saving the session failed: {e}"),
        }
    }

    fn log_state(&mut self) {
        let state = (
            self.player.status().clone(),
            self.player.queue().current().map(|t| t.id.clone()),
        );
        if self.last_state.as_ref() != Some(&state) {
            log!(
                "state {:?}, current {}, {} up next",
                state.0,
                state.1.as_ref().map_or("-", |id| id.0.as_str()),
                self.player.queue().upcoming().len()
            );
            self.last_state = Some(state);
        }
    }

    /// Sends the current track and state to the UI, and to Discord, when they changed.
    fn push_now_playing(&mut self) {
        self.push_state();
        let Some(track) = self.player.queue().current() else {
            return;
        };
        // Loading with the intent to play shows playing, so the play/pause buttons offer
        // pause. Sent again when the sound starts: Discord's timer runs on from the last
        // update.
        let np = NowPlaying {
            speed: playback_speed(&self.settings),
            ..now_playing(track, self.player.wants_play(), self.player.position())
        };
        let audible = matches!(self.player.status(), Status::Playing);
        if self.shown.as_ref() != Some(&(np.clone(), audible)) {
            if let Some(presence) = &self.presence {
                presence.show(&np);
            }
            self.shown = Some((np, audible));
        }
    }

    /// Sends the whole player state to the window when it changed. The window derives the
    /// media card and taskbar state from it (`bar.rs`).
    fn push_state(&mut self) {
        let like = |t: &Track| {
            let mut t = t.clone();
            if let Some(l) = self.likes.get(&t.id) {
                t.like = *l;
            }
            t
        };
        let queue = self.player.queue();
        let state = PlayerState {
            current: queue.current().map(like),
            upcoming: queue.upcoming().iter().map(like).collect(),
            playing: self.player.wants_play(),
            audible: matches!(self.player.status(), Status::Playing) && !self.buffering,
            loading: matches!(self.player.status(), Status::Loading) && self.player.wants_play(),
            position: self.player.position(),
            shuffle: self.player.shuffle(),
            repeat: self.player.repeat(),
            radio: queue.is_radio(),
            volume: self.slider_volume(),
            queue: self.queue_info(),
            speed: playback_speed(&self.settings),
        };
        // Every time, not only on a change: the API takes the position's progress too.
        self.publish_api(&state);
        if self.shown_state.as_ref() != Some(&state) {
            self.shown_state = Some(state.clone());
            (self.ui.events)(UiEvent::Player(Box::new(state)));
        }
        // A new track's clear the last one's.
        self.push_marks();
    }

    /// "at 180.2s of 200s" for the current track, for the log.
    fn time_left(&self) -> String {
        let at = self.player.position().as_secs_f64();
        match self.player.queue().current().and_then(|t| t.duration) {
            Some(d) => format!(
                "current at {at:.1}s of {}s, {:.1}s left",
                d.as_secs(),
                d.as_secs_f64() - at
            ),
            None => format!("current at {at:.1}s"),
        }
    }
}

/// The toast for Play with no output device (`AudioEvent::NoDevice`).
pub fn no_sound_output() -> &'static str {
    tr!("No sound output found. Connect speakers or headphones.")
}

/// The integrations' view of a track (media card, taskbar, tray, Discord, toasts).
pub fn now_playing(t: &Track, playing: bool, position: Duration) -> NowPlaying {
    NowPlaying {
        track_id: Some(t.id.0.clone()),
        title: t.display_title().into_owned(),
        artist_id: t
            .artists
            .first()
            .and_then(|a| a.id.as_ref())
            .map(|id| id.0.clone()),
        artist: t
            .artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        album: t.album.as_ref().map(|a| a.title.clone()),
        album_id: t.album.as_ref().map(|a| a.id.0.clone()),
        art_url: t.thumbnail.clone(),
        playing,
        position,
        duration: t.duration,
        speed: 1.0,
    }
}

fn source(s: &ResolvedStream) -> StreamSource {
    StreamSource {
        url: s.url.clone(),
        mime: s.mime.clone(),
        content_length: s.content_length,
        duration: s.duration,
        loudness_db: s.loudness_db,
    }
}

fn stream_info(s: &ResolvedStream) -> StreamInfo {
    StreamInfo {
        expires_at: s.expires_at,
        duration: s.duration,
    }
}

/// The innertube client a stream URL came from (`c=`), for the log.
fn client(url: &str) -> &str {
    url.split(['?', '&'])
        .find_map(|p| p.strip_prefix("c="))
        .unwrap_or("?")
}

fn prefs(s: &Settings) -> PlaybackPrefs {
    PlaybackPrefs {
        prefer_song_versions: s.prefer_song_versions,
        skip_non_music_in_radio: s.skip_non_music_in_radio,
        scrobble_other_media: s.scrobble_other_media,
        list_end: if s.autoplay {
            ListEnd::Autoplay
        } else {
            ListEnd::Restart
        },
        skip_disliked: s.skip_disliked,
        crossfade: (s.crossfade > 0).then(|| Duration::from_secs(u64::from(s.crossfade))),
        fade_on_skip: s.fade_on_skip,
        speed_percent: u16::from(s.playback_speed),
    }
}

/// The Playback speed setting as the engine and the integrations take it (1.0 is normal).
fn playback_speed(s: &Settings) -> f32 {
    f32::from(s.playback_speed) / 100.0
}

/// The resolver's format choice for the Streaming quality setting.
fn stream_quality(q: StreamingQuality) -> Quality {
    match q {
        StreamingQuality::High => Quality::High,
        StreamingQuality::Standard => Quality::Standard,
    }
}

/// The bands the audio bus plays for the Equalizer setting, in dB; `None` when off.
fn equalizer_bands(s: &Settings) -> Option<[f32; 5]> {
    s.equalizer
        .bands(s.equalizer_bands)
        .map(|b| b.map(f32::from))
}

/// What the audio engine drops for the "Skip silence" setting.
fn skip_silences(s: SkipSilence) -> featherplay_audio::SkipSilence {
    match s {
        SkipSilence::Off => featherplay_audio::SkipSilence::Off,
        SkipSilence::StartAndEnd => featherplay_audio::SkipSilence::StartAndEnd,
        SkipSilence::Everywhere => featherplay_audio::SkipSilence::Everywhere,
    }
}

/// The audio bus's compressor for the "Even out loudness" setting.
fn compressor(l: Loudness) -> Compressor {
    match l {
        Loudness::Off => Compressor::Off,
        Loudness::Light => Compressor::Light,
        Loudness::Strong => Compressor::Strong,
    }
}

/// The dev override: a plaintext cookie header file, read instead of the credential store.
const COOKIES_FILE_VAR: &str = "FEATHERPLAY_COOKIES_FILE";

/// Whether this build reads `FEATHERPLAY_COOKIES_FILE`: debug builds, or a release build
/// with the `dev-cookies-file` feature. A published build never does.
const COOKIES_FILE_ALLOWED: bool = cfg!(any(debug_assertions, feature = "dev-cookies-file"));

/// The cookie header: from the credential store, or `FEATHERPLAY_COOKIES_FILE` in a dev
/// build. Its contents are never logged.
fn cookie_header() -> Option<String> {
    // The performance harness's fixtures: no account is read.
    #[cfg(feature = "perf-harness")]
    if crate::perf::fixtures() {
        return None;
    }
    if let Some(path) = std::env::var_os(COOKIES_FILE_VAR).filter(|v| !v.is_empty()) {
        if !COOKIES_FILE_ALLOWED {
            log!("{COOKIES_FILE_VAR} is ignored: this is a release build");
        } else {
            match std::fs::read_to_string(&path) {
                Ok(text) if !text.trim().is_empty() => return Some(text.trim().to_string()),
                Ok(_) => log!("{COOKIES_FILE_VAR} is empty"),
                Err(e) => log!("can't read {COOKIES_FILE_VAR}: {}", e.kind()),
            }
        }
    }
    get_secret(SecretKey::YtCookies)
}
