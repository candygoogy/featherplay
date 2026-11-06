//! The player state machine. Pure: inputs in, effects out, no I/O and no clock
//! (`Input::Tick` brings the time). The app's controller runs the effects and feeds back
//! what happened.
//!
//! Track ids in `Resolve`, `Load` and `Preload` are the ids actually played: a music
//! video's song version when "prefer song versions" is on, so `Resolved` answers with that
//! id too.

use std::time::{Duration, SystemTime};

use crate::failure::{FailKind, Failure, StopCause, stop_cause};
pub use crate::model::ListEnd;
use crate::model::{LikeStatus, RepeatMode, Session, Track, TrackId, VideoType};
use crate::queue::{BackAction, Queue};
use crate::scrobble::{ScrobbleTracker, eligible};

/// Playback stops after this many tracks in a row fail to resolve or play.
const MAX_FAILURES: usize = 3;
/// A URL this close to its expiry is resolved again before it's loaded or resumed.
const EXPIRY_MARGIN: Duration = Duration::from_secs(30);
/// While playing or loading, a changed session is saved this often (from `Tick`). Paused
/// or stopped, a change is saved on the next tick.
const PERSIST_EVERY: Duration = Duration::from_secs(10);
/// An expired URL is re-resolved at most this many times in a row before the track counts
/// as failed, so a URL that never works can't loop.
const MAX_EXPIRED_RETRIES: u32 = 2;
/// Play time counted for one tick at most (ticks come every 250 ms), so a suspended
/// process or a clock jump doesn't count as listening.
const MAX_TICK_DELTA: Duration = Duration::from_secs(2);

#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    PlayTracks {
        tracks: Vec<Track>,
        start: usize,
        radio: bool,
    },
    Toggle,
    Next,
    Previous,
    SeekTo(Duration),
    /// From the controller's 250 ms timer: the engine's position and the wall clock. The
    /// timer runs only while `Player::needs_ticks()`; otherwise the controller sends one
    /// tick before and one after each other input, so the clock is current and a change
    /// (a pause, a seek while paused) is saved at once.
    Tick {
        position: Duration,
        now: SystemTime,
    },
    Resolved {
        id: TrackId,
        result: Result<StreamInfo, Failure>,
    },
    AudioStarted,
    AudioEnded,
    /// The current stream's URL stopped working (HTTP 403): resolve again at the current
    /// position. Not counted as a failure, up to `MAX_EXPIRED_RETRIES` in a row.
    StreamExpired,
    /// The current stream can't be opened, decoded or downloaded. Counts as a failure.
    AudioFailed(String),
    DeviceLost,
    AuthExpired,
    /// Signed in again: the next `AuthExpired` prompts again.
    SignedIn,
    /// The answer to `FetchContinuation(seed)`: `(seed, tracks)`, empty on failure.
    /// Ignored unless `seed` is the request still pending.
    ContinuationLoaded(TrackId, Vec<Track>),
    Restore(Session),
    /// The bar's toggles. Shuffle reorders only what's upcoming; off restores the order.
    SetShuffle(bool),
    SetRepeat(RepeatMode),
    /// The playback settings changed.
    SetPrefs(PlaybackPrefs),
    /// Up next edits. Indices are into `Queue::upcoming()`; out of range does nothing.
    MoveUpcoming {
        from: usize,
        to: usize,
    },
    RemoveUpcoming(usize),
    ClearUpcoming,
    /// Right after the current track. On an empty queue it plays.
    PlayNext(Track),
    /// Right after the current track, in their order (a whole playlist). On an empty queue
    /// the first one plays.
    PlayNextAll(Vec<Track>),
    /// At the end of the queue. On an empty queue the first one plays.
    Enqueue(Vec<Track>),
    /// Play the upcoming track at this index now; the ones before it go to history.
    SkipTo(usize),
    /// The account took a rating: every queue entry of the track gets it, so one disliked
    /// now is passed over when it comes round.
    SetLike(TrackId, LikeStatus),
}

/// What core needs from a resolved stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamInfo {
    pub expires_at: SystemTime,
    pub duration: Option<Duration>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Effect {
    Resolve(TrackId),
    /// Load the last stream resolved for `id` at `start`, paused.
    Load {
        id: TrackId,
        start: Duration,
    },
    /// Buffer the last stream resolved for `id`.
    Preload(TrackId),
    Play,
    Pause,
    Seek(Duration),
    /// Fetch more radio tracks following `TrackId`; answer with `ContinuationLoaded` for
    /// the same id (empty on failure).
    FetchContinuation(TrackId),
    /// A queue entry became audible for the first time (also each Repeat One loop).
    TrackStarted(Track),
    /// The track, which started at the given time, has played past its scrobble threshold.
    /// Emitted once per `TrackStarted`, and only for tracks `scrobble::eligible` allows.
    Scrobble(Track, SystemTime),
    /// Playback stopped after failures; the window says why in plain words.
    ShowError(Stop),
    ShowReauth,
    /// Save `Player::session()`.
    PersistSession,
    /// Status, current track or queue changed. Not sent for position updates.
    StateChanged,
}

/// Why playback stopped: the cause, for the window's words, and the raw text of the last
/// failure, for the log only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stop {
    pub cause: StopCause,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Idle,
    Loading,
    Playing,
    Paused,
    Stopped(String),
}

/// From `Settings`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlaybackPrefs {
    pub prefer_song_versions: bool,
    pub skip_non_music_in_radio: bool,
    pub scrobble_other_media: bool,
    /// What a list does when it runs out.
    pub list_end: ListEnd,
    /// Moving on (a track ends, Next, a continuation) passes over disliked tracks. One the
    /// user starts directly still plays, and Repeat One keeps playing it.
    pub skip_disliked: bool,
}

/// Where the current track is in being made playable.
#[derive(Clone, Debug, PartialEq)]
enum Phase {
    /// Nothing loaded in the engine for the current track.
    Empty,
    /// Waiting for `Resolved { id }`, then load at `start`.
    Resolving { id: TrackId, start: Duration },
    /// `Load` was emitted. `started` once the engine reported `Started` for that load.
    Loaded { id: TrackId, started: bool },
}

/// The next track's stream, resolved ahead of time.
#[derive(Clone, Debug, PartialEq)]
enum Preload {
    /// Not requested for the current track yet.
    Idle,
    Requested(TrackId),
    Ready(TrackId, StreamInfo),
    /// Requested for this id and failed, or it's the current track (Repeat One). Not
    /// retried while that id stays next.
    Done(TrackId),
}

pub struct Player {
    prefs: PlaybackPrefs,
    queue: Queue,
    status: Status,
    position: Duration,
    /// The latest `Tick` time.
    now: Option<SystemTime>,
    phase: Phase,
    /// The stream loaded (or last loaded) for the current track.
    stream: Option<(TrackId, StreamInfo)>,
    /// The stream of the track played before the current one, so Previous back to it
    /// needs no resolve. One entry only.
    spare: Option<(TrackId, StreamInfo)>,
    /// Play once loaded; false when paused or while restoring.
    play_intent: bool,
    /// `TrackStarted` was sent for the current queue entry.
    announced: bool,
    /// The failures in a row since a track last started, oldest first.
    failures: Vec<FailKind>,
    expired_retries: u32,
    preload: Preload,
    /// A preloaded stream whose track stopped being next after a queue edit, kept in case
    /// it comes back (Repeat One and off again, an undone move). One entry only.
    parked: Option<(TrackId, StreamInfo)>,
    /// Shuffles so far, mixed into the seed so turning shuffle on again reorders anew.
    shuffles: u64,
    continuation_pending: bool,
    /// The last track a continuation was requested for, so an empty answer isn't re-asked.
    continuation_for: Option<TrackId>,
    /// The queue ran out while a continuation was on its way; advance when it lands.
    waiting_for_continuation: bool,
    shuffle: bool,
    repeat: RepeatMode,
    auth_expired: bool,
    last_persist: Option<SystemTime>,
    /// The session changed since the last `PersistSession`.
    unsaved: bool,
    scrobble: ScrobbleTracker,
    /// The current track started before any tick; `start` the tracker on the next one.
    scrobble_start_pending: bool,
}

impl Player {
    pub fn new(prefs: PlaybackPrefs) -> Player {
        let mut queue = Queue::new();
        queue.set_list_end(prefs.list_end);
        Player {
            prefs,
            queue,
            status: Status::Idle,
            position: Duration::ZERO,
            now: None,
            phase: Phase::Empty,
            stream: None,
            spare: None,
            play_intent: false,
            announced: false,
            failures: Vec::new(),
            expired_retries: 0,
            preload: Preload::Idle,
            parked: None,
            shuffles: 0,
            continuation_pending: false,
            continuation_for: None,
            waiting_for_continuation: false,
            shuffle: false,
            repeat: RepeatMode::Off,
            auth_expired: false,
            last_persist: None,
            unsaved: false,
            scrobble: ScrobbleTracker::default(),
            scrobble_start_pending: false,
        }
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    pub fn queue(&self) -> &Queue {
        &self.queue
    }

    pub fn position(&self) -> Duration {
        self.position
    }

    pub fn shuffle(&self) -> bool {
        self.shuffle
    }

    pub fn repeat(&self) -> RepeatMode {
        self.repeat
    }

    /// Whether to show the player as playing: it's audible, or loading with the intent to
    /// play (after Next, Previous or a new queue). A pause during loading clears it.
    pub fn wants_play(&self) -> bool {
        match self.status {
            Status::Playing => true,
            Status::Loading => self.play_intent,
            _ => false,
        }
    }

    /// Whether the controller's timer must keep sending `Tick`: the position advances, a
    /// track is being loaded, or the next one is still to be preloaded. Paused, idle or
    /// stopped, nothing changes between inputs, so no ticks are needed.
    pub fn needs_ticks(&self) -> bool {
        matches!(self.status, Status::Playing | Status::Loading)
    }

    /// The state to persist: the whole queue (history included), the position and modes.
    pub fn session(&self) -> Session {
        let mut walk = self.queue.clone();
        let mut history = Vec::new();
        while walk.back(Duration::ZERO) == BackAction::Previous {
            history.extend(walk.current().cloned());
        }
        let index = history.len();
        history.reverse();
        let mut queue = history;
        queue.extend(self.queue.current().cloned());
        queue.extend(self.queue.upcoming().iter().cloned());
        Session {
            queue,
            index,
            position_ms: self.position.as_millis() as u64,
            radio: self.is_radio(),
            shuffle: self.shuffle,
            repeat: self.repeat,
        }
    }

    pub fn handle(&mut self, input: Input) -> Vec<Effect> {
        let mut out = Vec::new();
        let position = self.position;
        // A restored session is the one on disk. Any other input that did something may
        // have changed it (on the safe side: a spurious save is cheap); a tick changes it
        // only by moving the position.
        let restore = matches!(input, Input::Restore(_));
        let may_change = !matches!(input, Input::Tick { .. });
        match input {
            Input::PlayTracks {
                tracks,
                start,
                radio,
            } => self.play_tracks(tracks, start, radio, &mut out),
            Input::Toggle => self.toggle(&mut out),
            Input::Next => self.next(&mut out),
            Input::Previous => self.previous(&mut out),
            Input::SeekTo(to) => self.seek(to, &mut out),
            Input::Tick { position, now } => self.tick(position, now, &mut out),
            Input::Resolved { id, result } => self.resolved(id, result, &mut out),
            Input::AudioStarted => self.audio_started(&mut out),
            Input::AudioEnded => self.audio_ended(&mut out),
            Input::StreamExpired => self.stream_expired(&mut out),
            Input::AudioFailed(msg) => {
                if matches!(self.phase, Phase::Loaded { .. }) {
                    self.fail(Failure::other(msg), &mut out);
                }
            }
            Input::DeviceLost => self.device_lost(&mut out),
            Input::AuthExpired => {
                if !self.auth_expired {
                    self.auth_expired = true;
                    out.push(Effect::ShowReauth);
                }
            }
            Input::SignedIn => self.auth_expired = false,
            Input::ContinuationLoaded(seed, tracks) => {
                self.continuation_loaded(seed, tracks, &mut out)
            }
            Input::Restore(session) => self.restore(session, &mut out),
            Input::SetShuffle(on) => {
                self.shuffle = on;
                let seed = self.shuffle_seed();
                self.queue.set_shuffle(on, seed);
                self.queue_edited(&mut out);
            }
            Input::SetRepeat(mode) => {
                self.repeat = mode;
                self.queue.set_repeat(mode);
                self.stop_continuing();
                self.queue_edited(&mut out);
            }
            Input::SetPrefs(prefs) => {
                let old = self.prefs;
                self.prefs = prefs;
                if prefs.list_end != old.list_end {
                    self.queue.set_list_end(prefs.list_end);
                    self.stop_continuing();
                    self.queue_edited(&mut out);
                } else if prefs.skip_disliked != old.skip_disliked {
                    // The next track may be another one now.
                    self.replan_preload(&mut out);
                }
            }
            Input::MoveUpcoming { from, to } => {
                let len = self.queue.upcoming().len();
                if from < len && to < len && from != to {
                    self.queue.move_item(from, to);
                    self.queue_edited(&mut out);
                }
            }
            Input::RemoveUpcoming(index) => {
                if index < self.queue.upcoming().len() {
                    self.queue.remove(index);
                    self.queue_edited(&mut out);
                }
            }
            Input::ClearUpcoming => {
                if !self.queue.upcoming().is_empty() {
                    self.queue.clear_upcoming();
                    self.queue_edited(&mut out);
                }
            }
            Input::PlayNext(track) => {
                let empty = self.queue.current().is_none();
                self.queue.play_next(track);
                self.added(empty, &mut out);
            }
            Input::PlayNextAll(tracks) => {
                if !tracks.is_empty() {
                    let empty = self.queue.current().is_none();
                    self.queue.play_next_all(tracks);
                    self.added(empty, &mut out);
                }
            }
            Input::Enqueue(tracks) => {
                if !tracks.is_empty() {
                    let empty = self.queue.current().is_none();
                    self.queue.enqueue(tracks);
                    self.added(empty, &mut out);
                }
            }
            Input::SkipTo(index) => self.skip_to(index, &mut out),
            Input::SetLike(id, like) => {
                if self.queue.set_like(&id, like) {
                    self.queue_edited(&mut out);
                }
            }
        }
        if self.position != position || (may_change && !out.is_empty()) {
            self.unsaved = true;
        }
        if restore || out.contains(&Effect::PersistSession) {
            self.unsaved = false;
        }
        out
    }

    /// Repeat All or another list end turned autoplay off: the autoplay tracks not played
    /// yet go, and an answer on its way is ignored. Turned on again, the list asks again.
    fn stop_continuing(&mut self) {
        if self.queue.continues() {
            return;
        }
        self.queue.drop_upcoming_continuation();
        self.continuation_pending = false;
        self.continuation_for = None;
        if self.waiting_for_continuation {
            self.waiting_for_continuation = false;
            self.play_intent = false;
            self.status = Status::Idle;
        }
    }

    /// Tracks were added; on an empty queue the new current one plays.
    fn added(&mut self, was_empty: bool, out: &mut Vec<Effect>) {
        if was_empty && self.queue.current().is_some() {
            self.failures.clear();
            self.play_intent = true;
            self.enter_track(Duration::ZERO, out);
            out.push(Effect::PersistSession);
        } else {
            self.queue_edited(out);
        }
    }

    /// The upcoming tracks or the modes changed, and the current track didn't.
    fn queue_edited(&mut self, out: &mut Vec<Effect>) {
        self.replan_preload(out);
        self.maybe_continue(out);
        out.push(Effect::PersistSession);
        out.push(Effect::StateChanged);
    }

    /// Keeps the preload on the track that's next now. A preload for a track that's no
    /// longer next is dropped (a late answer for it is ignored, and `enter_track` only
    /// uses a preload for the track it enters), and the new next track is resolved at once
    /// if the current one has already started.
    fn replan_preload(&mut self, out: &mut Vec<Effect>) {
        let next = self.next_track().map(|t| self.play_id(t));
        let planned = match &self.preload {
            Preload::Idle => None,
            Preload::Requested(id) | Preload::Ready(id, _) | Preload::Done(id) => Some(id),
        };
        if planned.is_some() && planned == next.as_ref() {
            return;
        }
        if let Preload::Ready(id, info) = std::mem::replace(&mut self.preload, Preload::Idle) {
            self.parked = Some((id, info));
        }
        if matches!(self.phase, Phase::Loaded { started: true, .. }) {
            self.maybe_preload(out);
        }
    }

    fn skip_to(&mut self, index: usize, out: &mut Vec<Effect>) {
        if index >= self.queue.upcoming().len() {
            return;
        }
        // Straight down the queue: no Repeat One loop and no Repeat All wrap on the way.
        self.queue.set_repeat(RepeatMode::Off);
        for _ in 0..=index {
            self.queue.advance();
        }
        self.queue.set_repeat(self.repeat);
        self.silence_current(out);
        self.failures.clear();
        self.play_intent = true;
        self.enter_track(Duration::ZERO, out);
    }

    /// The player has no clock or randomness of its own: the latest tick's time, mixed
    /// with a counter.
    fn shuffle_seed(&mut self) -> u64 {
        self.shuffles += 1;
        let nanos = self
            .now
            .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
            .map_or(0, |d| d.as_nanos() as u64);
        nanos ^ self.shuffles.wrapping_mul(0x9E37_79B9_7F4A_7C15)
    }

    fn play_tracks(
        &mut self,
        tracks: Vec<Track>,
        start: usize,
        radio: bool,
        out: &mut Vec<Effect>,
    ) {
        let (tracks, start) = if radio && self.prefs.skip_non_music_in_radio {
            // The track the user picked always plays; the rest of a radio batch is filtered.
            let mut kept_start = 0;
            let mut kept = Vec::with_capacity(tracks.len());
            for (i, t) in tracks.into_iter().enumerate() {
                if i == start {
                    kept_start = kept.len();
                    kept.push(t);
                } else if is_music(&t) {
                    kept.push(t);
                }
            }
            (kept, kept_start)
        } else {
            (tracks, start)
        };
        self.silence_current(out);
        if self.queue.is_shuffled() != self.shuffle {
            // A restored session keeps its saved order, so its queue isn't shuffled yet.
            let seed = self.shuffle_seed();
            self.queue.set_shuffle(self.shuffle, seed);
        }
        self.queue.replace(tracks, start, radio);
        self.failures.clear();
        self.preload = Preload::Idle;
        self.parked = None;
        self.continuation_pending = false;
        self.continuation_for = None;
        self.waiting_for_continuation = false;
        self.play_intent = true;
        self.enter_track(Duration::ZERO, out);
    }

    fn toggle(&mut self, out: &mut Vec<Effect>) {
        match self.status {
            Status::Playing => {
                self.play_intent = false;
                out.push(Effect::Pause);
                self.set_status(Status::Paused, out);
            }
            Status::Loading => {
                self.play_intent = false;
                if matches!(self.phase, Phase::Loaded { .. }) {
                    out.push(Effect::Pause);
                }
                self.set_status(Status::Paused, out);
            }
            Status::Paused => {
                self.play_intent = true;
                match &self.phase {
                    Phase::Loaded { started, .. } if self.stream_fresh() => {
                        let status = if *started {
                            Status::Playing
                        } else {
                            Status::Loading
                        };
                        out.push(Effect::Play);
                        self.set_status(status, out);
                    }
                    Phase::Resolving { .. } => self.set_status(Status::Loading, out),
                    // Nothing loaded (restored session) or the URL expired while paused.
                    _ => {
                        self.stream = None;
                        self.load_current(self.position, out);
                    }
                }
            }
            Status::Idle | Status::Stopped(_) => {
                if self.queue.current().is_some() {
                    self.failures.clear();
                    self.play_intent = true;
                    let start = if self.status == Status::Idle {
                        Duration::ZERO
                    } else {
                        self.position
                    };
                    self.enter_track(start, out);
                }
            }
        }
    }

    fn next(&mut self, out: &mut Vec<Effect>) {
        // "Next" leaves a repeated track.
        if self.repeat == RepeatMode::One {
            self.queue.set_repeat(RepeatMode::Off);
        }
        let moved = self.move_on();
        self.queue.set_repeat(self.repeat);
        if moved {
            self.silence_current(out);
            self.play_intent = true;
            self.enter_track(Duration::ZERO, out);
        } else if self.continuation_pending {
            // The last track: go on once the continuation lands.
            self.silence_current(out);
            self.play_intent = true;
            self.waiting_for_continuation = true;
            self.set_status(Status::Loading, out);
        }
    }

    fn previous(&mut self, out: &mut Vec<Effect>) {
        match self.queue.back(self.position) {
            BackAction::Restart => match self.phase {
                Phase::Loaded { .. } if self.status != Status::Idle => {
                    self.position = Duration::ZERO;
                    out.push(Effect::Seek(Duration::ZERO));
                }
                // No seek goes out, so the state does: the bar shows the restart.
                Phase::Resolving { ref mut start, .. } => {
                    *start = Duration::ZERO;
                    self.position = Duration::ZERO;
                    out.push(Effect::StateChanged);
                }
                _ => {
                    self.position = Duration::ZERO;
                    out.push(Effect::StateChanged);
                }
            },
            BackAction::Previous => {
                self.silence_current(out);
                self.play_intent = true;
                self.enter_track(Duration::ZERO, out);
            }
        }
    }

    fn seek(&mut self, to: Duration, out: &mut Vec<Effect>) {
        self.position = to;
        match self.phase {
            Phase::Loaded { .. } => out.push(Effect::Seek(to)),
            Phase::Resolving { ref mut start, .. } => *start = to,
            Phase::Empty => {}
        }
    }

    fn tick(&mut self, position: Duration, now: SystemTime, out: &mut Vec<Effect>) {
        // Wall-clock time since the last tick, so a seek doesn't count as play time.
        let delta = self
            .now
            .replace(now)
            .and_then(|last| now.duration_since(last).ok())
            .unwrap_or_default()
            .min(MAX_TICK_DELTA);
        if matches!(self.phase, Phase::Loaded { .. }) && self.position != position {
            self.position = position;
            self.unsaved = true;
        }
        if self.scrobble_start_pending {
            // The track started before the first tick: start counting from now.
            self.scrobble_start_pending = false;
            if let Some(t) = self.queue.current() {
                self.scrobble.start(t, now);
            }
        } else if self.status == Status::Playing
            && let Some((track, started)) = self.scrobble.played(delta)
            && eligible(track.video_type, self.prefs.scrobble_other_media)
        {
            out.push(Effect::Scrobble(track, started));
        }
        if self.status == Status::Playing {
            self.maybe_preload(out);
        }
        let last = *self.last_persist.get_or_insert(now);
        let due = !self.needs_ticks()
            || now.duration_since(last).is_ok_and(|d| d >= PERSIST_EVERY)
            // The clock went backwards: start over.
            || now < last;
        if due && self.unsaved && self.queue.current().is_some() {
            self.last_persist = Some(now);
            out.push(Effect::PersistSession);
        }
    }

    /// Resolves and buffers the next track, once per track: when the current one starts,
    /// so Next is instant. Re-checked on each tick while playing, for a radio queue whose
    /// next track is still on its way.
    fn maybe_preload(&mut self, out: &mut Vec<Effect>) {
        if self.preload != Preload::Idle {
            return;
        }
        // In radio the next track may still be on its way; check again on the next tick.
        let Some(next) = self.next_track() else {
            return;
        };
        let next = self.play_id(next);
        let current = self.queue.current().map(|t| self.play_id(t));
        if Some(&next) == current.as_ref() {
            // Repeat One: the loaded stream is reused.
            self.preload = Preload::Done(next);
            return;
        }
        if let Some((sid, info)) = &self.spare
            && *sid == next
            && self.fresh(info)
        {
            // Back on the track before the one played last: that one's stream is the spare.
            out.push(Effect::Preload(next.clone()));
            self.preload = Preload::Ready(next, *info);
            return;
        }
        if let Some((pid, info)) = &self.parked
            && *pid == next
            && self.fresh(info)
        {
            // Next again after a queue edit had moved it away.
            out.push(Effect::Preload(next.clone()));
            self.preload = Preload::Ready(next, *info);
            self.parked = None;
            return;
        }
        self.preload = Preload::Requested(next.clone());
        out.push(Effect::Resolve(next));
    }

    fn resolved(
        &mut self,
        id: TrackId,
        result: Result<StreamInfo, Failure>,
        out: &mut Vec<Effect>,
    ) {
        if let Phase::Resolving { id: want, start } = &self.phase
            && *want == id
        {
            let start = *start;
            match result {
                Ok(info) => {
                    self.stream = Some((id.clone(), info));
                    self.load(id, start, out);
                }
                Err(failure) => self.fail(failure, out),
            }
            return;
        }
        if self.preload == Preload::Requested(id.clone()) {
            match result {
                Ok(info) => {
                    out.push(Effect::Preload(id.clone()));
                    if self.learn_duration(&id, &info) {
                        // Up next shows it.
                        out.push(Effect::StateChanged);
                    }
                    self.preload = Preload::Ready(id, info);
                }
                // The track gets a normal resolve, and its failure counted, when it's reached.
                Err(_) => self.preload = Preload::Done(id),
            }
        }
        // Otherwise a stale answer for a track that's no longer wanted.
    }

    fn audio_started(&mut self, out: &mut Vec<Effect>) {
        let Phase::Loaded { started, .. } = &mut self.phase else {
            return;
        };
        if *started {
            return;
        }
        *started = true;
        self.failures.clear();
        if self.play_intent {
            self.status = Status::Playing;
        }
        if !self.announced {
            self.announced = true;
            if let Some(t) = self.queue.current() {
                // The scrobble timestamp is the latest tick, at most 250 ms early.
                match self.now {
                    Some(now) => self.scrobble.start(t, now),
                    None => self.scrobble_start_pending = true,
                }
                out.push(Effect::TrackStarted(t.clone()));
            }
        }
        self.maybe_preload(out);
        out.push(Effect::StateChanged);
    }

    fn audio_ended(&mut self, out: &mut Vec<Effect>) {
        if !matches!(self.phase, Phase::Loaded { .. }) {
            return;
        }
        self.expired_retries = 0;
        if self.move_on() {
            self.enter_track(Duration::ZERO, out);
        } else if self.continuation_pending {
            self.waiting_for_continuation = true;
            self.set_status(Status::Loading, out);
        } else {
            // End of the queue: the last track stays current, and play starts it over.
            self.play_intent = false;
            self.set_status(Status::Idle, out);
        }
    }

    fn stream_expired(&mut self, out: &mut Vec<Effect>) {
        if !matches!(self.phase, Phase::Loaded { .. }) {
            return;
        }
        self.expired_retries += 1;
        if self.expired_retries > MAX_EXPIRED_RETRIES {
            self.fail(Failure::other("the stream URL keeps expiring"), out);
            return;
        }
        self.stream = None;
        self.load_current(self.position, out);
    }

    fn device_lost(&mut self, out: &mut Vec<Effect>) {
        // The engine has already paused and moved to the default device.
        if matches!(self.status, Status::Playing | Status::Loading) {
            self.play_intent = false;
            out.push(Effect::Pause);
            self.set_status(Status::Paused, out);
        }
    }

    fn continuation_loaded(&mut self, seed: TrackId, tracks: Vec<Track>, out: &mut Vec<Effect>) {
        if !self.continuation_pending || self.continuation_for.as_ref() != Some(&seed) {
            // An answer for a request that's no longer current (the queue was replaced).
            return;
        }
        self.continuation_pending = false;
        let tracks: Vec<Track> = if self.prefs.skip_non_music_in_radio {
            tracks.into_iter().filter(is_music).collect()
        } else {
            tracks
        };
        self.queue.append_continuation(tracks);
        out.push(Effect::StateChanged);
        if self.waiting_for_continuation {
            self.waiting_for_continuation = false;
            if self.move_on() {
                self.enter_track(Duration::ZERO, out);
            } else {
                self.play_intent = false;
                self.set_status(Status::Idle, out);
            }
        }
        self.maybe_continue(out);
    }

    fn restore(&mut self, session: Session, out: &mut Vec<Effect>) {
        let mut queue = Queue::new();
        queue.set_repeat(session.repeat);
        queue.set_list_end(self.prefs.list_end);
        queue.replace(session.queue, session.index, session.radio);
        // The saved order is already the play order, so it isn't shuffled again; the flag
        // is kept for the UI and the next save.
        self.queue = queue;
        self.shuffle = session.shuffle;
        self.repeat = session.repeat;
        self.position = Duration::from_millis(session.position_ms);
        self.phase = Phase::Empty;
        self.stream = None;
        self.spare = None;
        self.play_intent = false;
        self.announced = false;
        self.scrobble = ScrobbleTracker::default();
        self.scrobble_start_pending = false;
        self.failures.clear();
        self.expired_retries = 0;
        self.preload = Preload::Idle;
        self.parked = None;
        self.continuation_pending = false;
        self.continuation_for = None;
        self.waiting_for_continuation = false;
        let status = if self.queue.current().is_some() {
            Status::Paused
        } else {
            Status::Idle
        };
        self.status = status;
        out.push(Effect::StateChanged);
    }

    /// A track failed to resolve or play: skip it, or stop after `MAX_FAILURES` in a row (at
    /// once when the helpers couldn't be downloaded).
    fn fail(&mut self, failure: Failure, out: &mut Vec<Effect>) {
        self.failures.push(failure.kind);
        self.phase = Phase::Empty;
        self.stream = None;
        if failure.kind == FailKind::Setup {
            // The next songs would fail the same way: stay on this one, so Play tries again.
            self.stop(failure.detail, out);
            return;
        }
        if self.failures.len() >= MAX_FAILURES {
            let detail = format!(
                "{MAX_FAILURES} tracks in a row couldn't play: {}",
                failure.detail
            );
            self.stop(detail, out);
            return;
        }
        if self.move_on() {
            self.enter_track(Duration::ZERO, out);
        } else if self.continuation_pending {
            self.waiting_for_continuation = true;
            self.set_status(Status::Loading, out);
        } else {
            self.stop(failure.detail, out);
        }
    }

    fn stop(&mut self, detail: String, out: &mut Vec<Effect>) {
        self.play_intent = false;
        self.waiting_for_continuation = false;
        out.push(Effect::ShowError(Stop {
            cause: stop_cause(&self.failures),
            detail: detail.clone(),
        }));
        self.set_status(Status::Stopped(detail), out);
    }

    /// The user moved off a track that may still be audible: pause it now, since the next
    /// one may take a resolve (~10 s) before its `Load` replaces it in the engine.
    fn silence_current(&self, out: &mut Vec<Effect>) {
        if matches!(self.phase, Phase::Loaded { .. }) {
            out.push(Effect::Pause);
        }
    }

    /// The queue's current entry changed: reset per-entry state and load it.
    fn enter_track(&mut self, start: Duration, out: &mut Vec<Effect>) {
        self.announced = false;
        self.expired_retries = 0;
        // The previous entry's play time stops counting; the new one starts on `AudioStarted`.
        self.scrobble = ScrobbleTracker::default();
        self.scrobble_start_pending = false;
        let preload = std::mem::replace(&mut self.preload, Preload::Idle);
        let Some(track) = self.queue.current() else {
            self.phase = Phase::Empty;
            self.play_intent = false;
            self.set_status(Status::Idle, out);
            return;
        };
        let id = self.play_id(track);
        self.swap_spare(&id);
        if matches!(&self.parked, Some((pid, _)) if *pid == id)
            && !matches!(&self.stream, Some((sid, _)) if *sid == id)
        {
            // Skipped to a track whose preload a queue edit had set aside.
            self.stream = self.parked.take();
        }
        let have_stream =
            matches!(&self.stream, Some((sid, _)) if *sid == id) && self.stream_fresh();
        match preload {
            Preload::Ready(pid, info) if pid == id => self.stream = Some((pid, info)),
            Preload::Requested(pid) if pid == id && !have_stream => {
                // Its resolve is already running; wait for that answer.
                self.position = start;
                self.phase = Phase::Resolving { id, start };
                self.set_status(Status::Loading, out);
                self.maybe_continue(out);
                return;
            }
            _ => {}
        }
        self.load_current(start, out);
    }

    /// Entering `id`: the stream of the track being left becomes the spare, and the spare
    /// comes back if it's for `id`. `load_current` re-resolves it if it's near expiry.
    fn swap_spare(&mut self, id: &TrackId) {
        if matches!(&self.stream, Some((sid, _)) if sid == id) {
            // Repeat One, or the same track again.
            return;
        }
        let left = self.stream.take();
        if matches!(&self.spare, Some((sid, _)) if sid == id) {
            self.stream = self.spare.take();
        }
        if left.is_some() {
            self.spare = left;
        }
    }

    /// Loads the current track at `start`, reusing a fresh stream for it or resolving one.
    fn load_current(&mut self, start: Duration, out: &mut Vec<Effect>) {
        let Some(track) = self.queue.current() else {
            self.set_status(Status::Idle, out);
            return;
        };
        let id = self.play_id(track);
        self.position = start;
        let reusable = matches!(&self.stream, Some((sid, _)) if *sid == id) && self.stream_fresh();
        if reusable {
            self.load(id, start, out);
        } else {
            self.stream = None;
            out.push(Effect::Resolve(id.clone()));
            self.phase = Phase::Resolving { id, start };
            self.set_status(Status::Loading, out);
        }
        self.maybe_continue(out);
    }

    fn load(&mut self, id: TrackId, start: Duration, out: &mut Vec<Effect>) {
        if let Some((sid, info)) = &self.stream
            && *sid == id
        {
            // Before the state goes out: the bar and the integrations read the queue.
            let info = *info;
            self.learn_duration(&id, &info);
        }
        out.push(Effect::Load {
            id: id.clone(),
            start,
        });
        self.phase = Phase::Loaded { id, started: false };
        self.position = start;
        if self.play_intent {
            out.push(Effect::Play);
            self.set_status(Status::Loading, out);
        } else {
            self.set_status(Status::Paused, out);
        }
    }

    /// The stream's length is the real one for the entries that play `id`: it fills in a
    /// length the catalog didn't give (Home shelves, artist top songs), and replaces a music
    /// video's when its song version plays. Returns whether the queue changed.
    fn learn_duration(&mut self, id: &TrackId, info: &StreamInfo) -> bool {
        let Some(d) = info.duration else {
            return false;
        };
        let prefs = self.prefs;
        self.queue.set_duration(|t| play_id(prefs, t) == *id, d)
    }

    fn maybe_continue(&mut self, out: &mut Vec<Effect>) {
        if self.continuation_pending || !self.queue.needs_continuation() {
            return;
        }
        let last = self
            .queue
            .upcoming()
            .last()
            .or(self.queue.current())
            .map(|t| t.id.clone());
        let Some(last) = last else { return };
        if self.continuation_for.as_ref() == Some(&last) {
            return;
        }
        self.continuation_pending = true;
        self.continuation_for = Some(last.clone());
        out.push(Effect::FetchContinuation(last));
    }

    fn stream_fresh(&self) -> bool {
        self.stream.as_ref().is_some_and(|(_, s)| self.fresh(s))
    }

    fn fresh(&self, s: &StreamInfo) -> bool {
        self.now
            .is_none_or(|now| s.expires_at > now + EXPIRY_MARGIN)
    }

    /// The id actually played for `t` (its song version when preferred): what `Resolve`,
    /// `Load` and `Preload` name, so a stream resolved ahead for `t` is found by it.
    pub fn play_id(&self, t: &Track) -> TrackId {
        play_id(self.prefs, t)
    }

    fn is_radio(&self) -> bool {
        self.queue.is_radio()
    }

    /// Moves on to the next track (the end of one, Next, a failure, a continuation), past
    /// the ones `passed_over` picks. Returns whether it moved.
    fn move_on(&mut self) -> bool {
        let prefs = self.prefs;
        self.queue.advance_past(|t| passed_over(prefs, t)).is_some()
    }

    /// The play ids of the next `n` tracks, in the order they'll play (shuffled, repeated,
    /// past the ones passed over), the next one first.
    pub fn ahead(&self, n: usize) -> Vec<TrackId> {
        let prefs = self.prefs;
        self.queue
            .peek_ahead_past(n, |t| passed_over(prefs, t))
            .into_iter()
            .map(|t| play_id(prefs, t))
            .collect()
    }

    /// The track `move_on` would go to.
    fn next_track(&self) -> Option<&Track> {
        let prefs = self.prefs;
        self.queue.peek_next_past(|t| passed_over(prefs, t))
    }

    fn set_status(&mut self, status: Status, out: &mut Vec<Effect>) {
        self.status = status;
        out.push(Effect::StateChanged);
    }
}

/// The id actually played for `t`: its song version when preferred.
fn play_id(prefs: PlaybackPrefs, t: &Track) -> TrackId {
    match &t.song_version {
        Some(song) if prefs.prefer_song_versions && t.video_type == VideoType::MusicVideo => {
            song.clone()
        }
        _ => t.id.clone(),
    }
}

/// Moving on passes over `t`: the user disliked it, and "Skip songs I've disliked" is on.
fn passed_over(prefs: PlaybackPrefs, t: &Track) -> bool {
    prefs.skip_disliked && t.like == LikeStatus::Dislike
}

/// Radio and continuations drop user uploads and podcast episodes.
fn is_music(t: &Track) -> bool {
    !matches!(t.video_type, VideoType::UserVideo | VideoType::Episode)
}
