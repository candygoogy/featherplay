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
//! actions, so those areas extend the controller in their own files. `interface.rs` has the
//! types the window and the controller share; `startup.rs`, `effects.rs` and `messages.rs`
//! the start, the player's effects and the results of background work.

mod account;
mod effects;
mod interface;
mod local_api;
mod messages;
mod notice;
mod queue;
mod reconnect;
mod silences;
mod song_versions;
mod startup;
mod streams;

use std::collections::{HashMap, VecDeque};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use featherplay_audio::{AudioEngine, Compressor, StreamSource};
use featherplay_core::model::{LikeStatus, Track, TrackId};
use featherplay_core::player::{Input, ListEnd, PlaybackPrefs, Player, Status, StreamInfo};
use featherplay_core::scrobble::Alternates;
use featherplay_core::tr;
use featherplay_integrations::sponsorblock::{self, Segment};
use featherplay_integrations::{NowPlaying, UiCommand};
use featherplay_store::{Loudness, Paths, Settings, SkipSilence, StreamingQuality, save_session};
use featherplay_streams::{Quality, ResolvedStream, YtDlpResolver};
use featherplay_ytm::YtMusic;
use tokio::sync::mpsc;

pub use account::{
    AccountAction, AccountEvent, ApiKeys, Change, Cookies, LastfmStatus, ListenBrainzStatus, Token,
};
pub use interface::{Action, Options, PlayerState, UiEvent, UiLink};
pub use local_api::{LocalApiNotice, LocalApiStatus};
pub use queue::{
    PlayingFrom, QueueAction, QueueInfo, add_input, back_to_inputs, playing_from, queue_edit,
    queue_info, with_likes,
};
pub use silences::Mark;

use messages::Msg;
use startup::cookie_header;

use crate::presence::Presence;
use crate::scrobbler::Scrobblers;

/// How often the player gets the engine's position and the time, while it
/// `needs_ticks()` (playing or loading). Paused, the timer is off.
const TICK: Duration = Duration::from_millis(250);
/// How often the offline scrobble queue is retried (the first run is at startup).
const FLUSH_SCROBBLES: Duration = Duration::from_secs(5 * 60);

/// The rows on screen resolved ahead of a click at most (`Action::Prepare`), the top first.
pub const PREPARE_ROWS: usize = 12;

/// Time since the process started; the first call starts the clock.
pub use crate::diagnostics::uptime;

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
    /// The performance harness's stand-in stream (`Options::fixture_stream`).
    fixture_stream: Option<fn() -> Option<ResolvedStream>>,
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

/// A future boxed in this crate.
type Boxed<T> = Pin<Box<dyn Future<Output = T>>>;

impl Controller {
    /// Loads the settings and the session; `run` then plays. Both are boxed here so the
    /// loop's code is compiled once, in this crate: a future is compiled in each crate that
    /// polls it, and the app's copies of the loop and its tasks cost ~150 KB of the exe.
    #[inline(never)]
    pub fn start(opts: Options, ui: UiLink) -> Boxed<Result<Controller, String>> {
        Box::pin(Self::start_here(opts, ui))
    }

    /// Runs until Ctrl+C or `UiCommand::Quit`, then saves the session. Boxed as `start` is.
    #[inline(never)]
    pub fn run(self) -> Boxed<()> {
        Box::pin(self.run_here())
    }

    async fn run_here(mut self) {
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
        let signed_in = signed_in || self.fixture_stream.is_some();
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
        self.restore_refused();
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
