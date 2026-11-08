//! Resolved streams, kept until they near their expiry, and resolves started ahead of a
//! click: a song row or card hovered for 120 ms or pressed, the first row of a list page
//! that opens, Home's featured song, the restored session's track, and the queue's tracks
//! after the next one (`AHEAD` in all with it, in play order: shuffled, repeated, past the
//! ones passed over). A track whose stream is here starts without a resolve, and with its
//! first 256 KB fetched into memory (`AudioEngine::prefetch`) without a request; a fetch
//! that's refused (HTTP 403) is told, and the track resolved again ahead, once.
//!
//! The track played before the current one gets its first 256 KB fetched again when it's
//! left (its download went with it), so Previous starts from memory too.
//!
//! The streams outlive a restart (`SAVED_STREAMS`, written at quit): the next session uses
//! one until it nears its expiry, once a resolve has shown the same address.
//!
//! The player's own resolves start at once. Resolves ahead ("hints") take at most
//! `HINT_SLOTS` at a time; the newest hints go first, and older row hints are dropped past
//! `HINT_QUEUE`. Their yt-dlp runs take turns (`YtDlpResolver::resolve_ahead`). Nothing
//! resolves twice: a hint for a stream that's here, running or waiting does nothing. The
//! queue's hints follow the queue: a track no longer ahead has its resolve stopped, and
//! paused or stopped, none runs. No timers: hints start when they come in and when a
//! resolve ends.

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use featherplay_audio::StreamSource;
use featherplay_core::failure::getting_ready;
use featherplay_core::model::{Track, TrackId};
use featherplay_core::player::Input;
use featherplay_streams::ResolvedStream;
use serde_json::{Value, json};
use tokio::sync::Notify;
use tokio::task::AbortHandle;

use super::{Controller, Msg};

/// Resolved streams kept, oldest dropped first (but never the ones `done` is told to keep).
const KEEP: usize = 32;
/// Resolves ahead running at once (the player's own come on top): the queue's two and a
/// hovered row.
const HINT_SLOTS: usize = 3;
/// Row hints waiting for a slot.
const HINT_QUEUE: usize = 4;
/// The queue's tracks resolved ahead, the next one (the player's own preload) included.
const AHEAD: usize = 3;
/// Tracks the queue doesn't resolve ahead again for `FAILED_FOR` (their resolve failed, or
/// their stream was refused twice), and refused preloads remembered.
const FAILED_KEPT: usize = 8;
const FAILED_FOR: Duration = Duration::from_secs(10 * 60);
/// A stream that expires sooner than this is resolved again (the player re-resolves at
/// 30 s before expiry, so a stream handed to it must outlive that comfortably).
const FRESH_FOR: Duration = Duration::from_secs(10 * 60);
/// A resolve that takes longer logs its own time (a normal one takes ~0.4 s).
const SLOW_RESOLVE: Duration = Duration::from_secs(1);
/// The streams kept for the next session, in the cache dir (`Streams::saved`).
pub(super) const SAVED_STREAMS: &str = "streams.json";

/// The address a stream URL is for (its `ip=`).
fn address(url: &str) -> Option<&str> {
    url.split_once('?')?
        .1
        .split('&')
        .find_map(|kv| kv.strip_prefix("ip="))
}

/// Why a resolve ahead was asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Hint {
    /// A row, a page's first row, Home's featured song, the restored track, a refused
    /// preload.
    Row,
    /// One of the queue's tracks after the next one.
    Queue,
}

struct Running {
    id: TrackId,
    since: Instant,
    /// Started ahead of a click; `None` once the player asks for it.
    hint: Option<Hint>,
    /// Told when the player asks for a hint's track while it runs.
    urgent: Arc<Notify>,
    /// Stops a queue hint whose track is no longer ahead.
    task: Option<AbortHandle>,
}

#[derive(Default)]
pub(super) struct Streams {
    /// Oldest first, one per id.
    cache: VecDeque<(TrackId, ResolvedStream)>,
    running: Vec<Running>,
    /// Newest last.
    waiting: VecDeque<(TrackId, Hint)>,
    /// Tracks given up on ahead, and when (`give_up`), newest last.
    failed: VecDeque<(TrackId, Instant)>,
    /// The stream loaded last.
    loaded: Option<StreamSource>,
    /// The tracks whose refused preload was resolved again lately (`again_after_refusal`).
    refused_preloads: VecDeque<TrackId>,
    /// Streams of the last session (`restore`) not resolved again since.
    restored: Vec<TrackId>,
    /// The `ip=` of the last stream resolved this session: a restored one answers only for
    /// the same address.
    address: Option<String>,
}

/// How far a track's stream is from playing (`Streams::readiness`), least ready first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Readiness {
    /// A resolve would start from nothing (a hint still waiting for its turn too).
    Unresolved,
    Resolving,
    Here,
}

/// What a resolve the player asks for needs.
#[derive(Debug, PartialEq)]
pub(super) enum Want {
    Cached(Box<ResolvedStream>),
    /// A resolve for it is running; its answer goes to the player.
    Running,
    Start,
}

impl Streams {
    /// The stream last resolved for `id`, fresh or not.
    pub fn get(&self, id: &TrackId) -> Option<&ResolvedStream> {
        self.cache.iter().find(|(s, _)| s == id).map(|(_, s)| s)
    }

    /// The stream resolved for `id`, if it stays valid for `FRESH_FOR` from `now`.
    /// One from the last session, only if a resolve showed that it's for this address.
    fn fresh(&self, id: &TrackId, now: SystemTime) -> Option<&ResolvedStream> {
        self.get(id).filter(|s| {
            s.expires_at > now + FRESH_FOR
                && (!self.restored.contains(id)
                    || self.address.is_some() && address(&s.url) == self.address.as_deref())
        })
    }

    /// The player asks for `id`.
    pub fn want(&mut self, id: &TrackId, now: SystemTime) -> Want {
        self.waiting.retain(|(w, _)| w != id);
        if let Some(s) = self.fresh(id, now) {
            return Want::Cached(Box::new(s.clone()));
        }
        if let Some(r) = self.running.iter_mut().find(|r| r.id == *id) {
            if r.hint.take().is_some() {
                r.urgent.notify_one();
            }
            return Want::Running;
        }
        self.running.push(Running {
            id: id.clone(),
            since: Instant::now(),
            hint: None,
            urgent: Arc::default(),
            task: None,
        });
        Want::Start
    }

    /// How soon the player could load `id` if it asked now.
    pub fn readiness(&self, id: &TrackId, now: SystemTime) -> Readiness {
        if self.fresh(id, now).is_some() {
            Readiness::Here
        } else if self.running.iter().any(|r| r.id == *id) {
            Readiness::Resolving
        } else {
            Readiness::Unresolved
        }
    }

    /// Whether a hint for `id` would do nothing: its stream is here, running or waiting.
    fn covered(&self, id: &TrackId, now: SystemTime) -> bool {
        self.fresh(id, now).is_some()
            || self.running.iter().any(|r| r.id == *id)
            || self.waiting.iter().any(|(w, _)| w == id)
    }

    /// Queues a resolve ahead for `id` (a row) unless it's covered already.
    pub fn hint(&mut self, id: &TrackId, now: SystemTime) {
        if self.covered(id, now) {
            return;
        }
        self.waiting.push_back((id.clone(), Hint::Row));
        if self.waiting.iter().filter(|(_, h)| *h == Hint::Row).count() > HINT_QUEUE
            && let Some(i) = self.waiting.iter().position(|(_, h)| *h == Hint::Row)
        {
            self.waiting.remove(i);
        }
    }

    /// The queue's tracks to resolve ahead are now `ids`, nearest first (none while paused
    /// or stopped). `next`, the player's own preload, isn't hinted, but a queue hint for it
    /// keeps running for the player to take over. Queue hints for other tracks stop,
    /// running or waiting; returns the running ones stopped. A track whose resolve ahead
    /// failed (or whose stream was refused twice) isn't asked again for a while.
    pub fn ahead(
        &mut self,
        next: Option<&TrackId>,
        ids: &[TrackId],
        now: SystemTime,
    ) -> Vec<TrackId> {
        let wanted = |id: &TrackId| ids.contains(id) || next == Some(id);
        self.waiting
            .retain(|(w, h)| *h == Hint::Row || ids.contains(w));
        let mut stopped = Vec::new();
        self.running.retain(|r| {
            let stale = r.hint == Some(Hint::Queue) && !wanted(&r.id);
            if stale {
                if let Some(task) = &r.task {
                    task.abort();
                }
                stopped.push(r.id.clone());
            }
            !stale
        });
        let since = Instant::now().checked_sub(FAILED_FOR);
        self.failed
            .retain(|(_, at)| since.is_none_or(|since| *at > since));
        // The nearest last, so it starts first.
        for id in ids.iter().rev() {
            if !self.covered(id, now) && !self.failed.iter().any(|(f, _)| f == id) {
                self.waiting.push_back((id.clone(), Hint::Queue));
            }
        }
        stopped
    }

    /// `id` isn't resolved ahead again for `FAILED_FOR`.
    fn give_up(&mut self, id: &TrackId) {
        self.failed.retain(|(f, _)| f != id);
        self.failed.push_back((id.clone(), Instant::now()));
        if self.failed.len() > FAILED_KEPT {
            self.failed.pop_front();
        }
    }

    /// The hints that can start now, newest first, each with what tells it the player
    /// asks for its track; they count as running.
    pub fn start_hints(&mut self) -> Vec<(TrackId, Arc<Notify>)> {
        let mut out = Vec::new();
        while self.running.iter().filter(|r| r.hint.is_some()).count() < HINT_SLOTS {
            let Some((id, hint)) = self.waiting.pop_back() else {
                break;
            };
            let urgent = Arc::new(Notify::new());
            self.running.push(Running {
                id: id.clone(),
                since: Instant::now(),
                hint: Some(hint),
                urgent: urgent.clone(),
                task: None,
            });
            out.push((id, urgent));
        }
        out
    }

    /// The hint for `id` runs as `task`, which `ahead` may stop.
    pub fn started(&mut self, id: &TrackId, task: AbortHandle) {
        if let Some(r) = self.running.iter_mut().find(|r| r.id == *id) {
            r.task = Some(task);
        }
    }

    /// A resolve ended: returns how long it took and whether it was a hint the player
    /// still hasn't asked for. A stream is kept; when that makes too many, the oldest
    /// not in `keep` (the player's current and next tracks) goes.
    pub fn done(
        &mut self,
        id: &TrackId,
        stream: Option<&ResolvedStream>,
        keep: &[TrackId],
    ) -> (Duration, bool) {
        let (took, hint) = match self.running.iter().position(|r| r.id == *id) {
            Some(i) => {
                let r = self.running.swap_remove(i);
                (r.since.elapsed(), r.hint)
            }
            None => (Duration::ZERO, None),
        };
        match stream {
            Some(s) => {
                if let Some(ip) = address(&s.url) {
                    self.address = Some(ip.to_string());
                }
                self.restored.retain(|r| r != id);
                self.cache.retain(|(c, _)| c != id);
                self.cache.push_back((id.clone(), s.clone()));
                if self.cache.len() > KEEP
                    && let Some(i) = self.cache.iter().position(|(c, _)| !keep.contains(c))
                {
                    self.cache.remove(i);
                }
            }
            None if hint == Some(Hint::Queue) => self.give_up(id),
            None => {}
        }
        (took, hint.is_some())
    }

    /// Drops the stream of `id` (its URL was refused); returns it.
    pub fn forget(&mut self, id: &TrackId) -> Option<ResolvedStream> {
        let i = self.cache.iter().position(|(c, _)| c == id)?;
        self.cache.remove(i).map(|(_, s)| s)
    }

    /// Whether a refused preload of `id` is resolved again ahead: not when it was one of
    /// the last resolved again, so a URL refused every time can't loop while paused. Nor is
    /// it resolved ahead for the queue again for a while then: the player resolves it.
    pub fn again_after_refusal(&mut self, id: &TrackId) -> bool {
        if self.refused_preloads.contains(id) {
            self.give_up(id);
            return false;
        }
        self.refused_preloads.push_back(id.clone());
        if self.refused_preloads.len() > FAILED_KEPT {
            self.refused_preloads.pop_front();
        }
        true
    }

    /// The streams that stay valid for `FRESH_FOR` from `now`, for `restore` in the next
    /// session (`SAVED_STREAMS`).
    pub fn saved(&self, now: SystemTime) -> String {
        let secs = |t: SystemTime| t.duration_since(UNIX_EPOCH).ok().map(|d| d.as_secs());
        let ms = |d: Duration| u64::try_from(d.as_millis()).ok();
        let streams: Vec<Value> = self
            .cache
            .iter()
            .filter(|(_, s)| s.expires_at > now + FRESH_FOR)
            .map(|(id, s)| {
                json!({
                    "id": id.0,
                    "url": s.url,
                    "mime": s.mime,
                    "itag": s.itag,
                    "kbps": s.bitrate_kbps,
                    "length": s.content_length,
                    "expires": secs(s.expires_at),
                    "duration_ms": s.duration.and_then(ms),
                    "available_at": s.available_at.and_then(secs),
                    "title": s.video_title,
                    "tag": s.first_tag,
                    "loudness_db": s.loudness_db,
                })
            })
            .collect();
        json!({ "streams": streams }).to_string()
    }

    /// Keeps the streams `saved` gave in the last session that stay valid for `FRESH_FOR`
    /// from `now`. Each answers once a resolve shows that its URL is for this address (its
    /// `ip=`): the network may have changed since.
    pub fn restore(&mut self, text: &str, now: SystemTime) {
        let Ok(v) = serde_json::from_str::<Value>(text) else {
            return;
        };
        let at = |v: &Value| v.as_u64().map(|s| UNIX_EPOCH + Duration::from_secs(s));
        let text = |v: &Value| v.as_str().map(str::to_string);
        for s in v["streams"].as_array().into_iter().flatten() {
            let (Some(id), Some(url), Some(expires_at)) =
                (text(&s["id"]), text(&s["url"]), at(&s["expires"]))
            else {
                continue;
            };
            if expires_at <= now + FRESH_FOR || address(&url).is_none() || self.cache.len() >= KEEP
            {
                continue;
            }
            let stream = ResolvedStream {
                url,
                mime: text(&s["mime"]).unwrap_or_default(),
                itag: s["itag"]
                    .as_u64()
                    .and_then(|i| u32::try_from(i).ok())
                    .unwrap_or(0),
                bitrate_kbps: s["kbps"]
                    .as_u64()
                    .and_then(|k| u32::try_from(k).ok())
                    .unwrap_or(0),
                content_length: s["length"].as_u64(),
                expires_at,
                duration: s["duration_ms"].as_u64().map(Duration::from_millis),
                available_at: at(&s["available_at"]),
                video_title: text(&s["title"]),
                first_tag: text(&s["tag"]),
                loudness_db: s["loudness_db"].as_f64().map(|db| db as f32),
            };
            let id = TrackId(id);
            self.cache.retain(|(c, _)| *c != id);
            self.cache.push_back((id.clone(), stream));
            self.restored.push(id);
        }
    }

    /// Whether `id`'s stream (or the one just dropped) came from the last session: its
    /// refusal says nothing of this session's URLs, so the resolver isn't told.
    pub fn is_from_last_session(&self, id: &TrackId) -> bool {
        self.restored.contains(id)
    }

    /// Drops the stream whose URL is `url` (refused while preloaded); returns its id.
    pub fn forget_url(&mut self, url: &str) -> Option<TrackId> {
        let i = self.cache.iter().position(|(_, s)| s.url == url)?;
        self.cache.remove(i).map(|(id, _)| id)
    }
}

impl Controller {
    /// Resolves `tracks` ahead of a click (see the module docs).
    pub(super) fn hint(&mut self, tracks: &[Track]) {
        let now = SystemTime::now();
        for t in tracks {
            let mut t = t.clone();
            self.song_versions.fill(&mut t);
            let id = self.player.play_id(&t);
            self.streams.hint(&id, now);
        }
        self.start_hints();
    }

    /// A hint by video id alone (a row the window has no `Track` for): the queue's track
    /// with that id gives its play id.
    pub(super) fn hint_id(&mut self, id: TrackId) {
        let q = self.player.queue();
        let known = q
            .current()
            .into_iter()
            .chain(q.upcoming())
            .find(|t| t.id == id)
            .map(|t| self.player.play_id(t));
        self.streams.hint(&known.unwrap_or(id), SystemTime::now());
        self.start_hints();
    }

    /// The play ids of the current and the next track, whose streams the player may load
    /// again at any time.
    pub(super) fn in_use(&self) -> Vec<TrackId> {
        let q = self.player.queue();
        q.current()
            .into_iter()
            .chain(q.upcoming().first())
            .map(|t| self.player.play_id(t))
            .collect()
    }

    /// While the current track plays: the tracks after the next one, up to `AHEAD` with it
    /// (the next one is the player's own preload). Called again after each state change,
    /// so a queue edit, shuffle or repeat moves them.
    pub(super) fn hint_queue(&mut self) {
        let mut ahead = self.player.ahead(AHEAD);
        // Repeat All round a short queue: the current track's stream is the player's.
        let current = self
            .player
            .queue()
            .current()
            .map(|t| self.player.play_id(t));
        ahead.retain(|id| Some(id) != current.as_ref());
        let (next, after) = match ahead.split_first() {
            Some((next, after)) => (Some(next), after),
            None => (None, &[][..]),
        };
        self.resolve_ahead(next, after);
        self.look_up_ahead(AHEAD);
    }

    /// Paused or stopped: the queue's resolves ahead stop.
    pub(super) fn stop_ahead(&mut self) {
        self.resolve_ahead(None, &[]);
    }

    fn resolve_ahead(&mut self, next: Option<&TrackId>, after: &[TrackId]) {
        for id in self.streams.ahead(next, after, SystemTime::now()) {
            log!("resolve {} ahead: stopped, no longer up next", id.0);
        }
        self.start_hints();
    }

    /// `src` was just loaded: the stream it replaced gets its head fetched, for Previous.
    pub(super) fn loaded(&mut self, src: StreamSource) {
        if let Some(prev) = self.streams.loaded.replace(src)
            && self
                .streams
                .loaded
                .as_ref()
                .is_some_and(|s| s.url != prev.url)
        {
            // After the load, so the engine no longer has a voice for it.
            self.audio.prefetch(&prev);
        }
    }

    /// A preloaded or prefetched stream's URL was refused: it's resolved again ahead, so
    /// Play needn't wait for the refusal and then the resolve (the restored track,
    /// 2026-11-05).
    pub(super) fn preload_refused(&mut self, url: &str) {
        let Some(id) = self.streams.forget_url(url) else {
            return;
        };
        if !self.streams.is_from_last_session(&id) {
            self.resolver.refused(&id, url);
        }
        if !self.streams.again_after_refusal(&id) {
            log!(
                "preload {} refused again (HTTP 403): resolved at Play",
                id.0
            );
            return;
        }
        log!(
            "preload {} refused (HTTP 403), resolving it again ahead",
            id.0
        );
        self.streams.hint(&id, SystemTime::now());
        self.start_hints();
    }

    pub(super) fn start_hints(&mut self) {
        // The performance harness's fixtures play nothing, so nothing is asked of YouTube.
        #[cfg(feature = "perf-harness")]
        if crate::perf::fixtures() {
            return;
        }
        for (id, urgent) in self.streams.start_hints() {
            log!("resolve {} ahead", id.0);
            let task = self.spawn_resolve(id.clone(), Some(urgent));
            self.streams.started(&id, task);
        }
    }

    /// `Effect::Resolve`: a stream that's here answers at once.
    pub(super) fn resolve(&mut self, id: TrackId) -> Option<Input> {
        #[cfg(feature = "perf-harness")]
        if crate::perf::fixtures() {
            log!(
                "resolve {}: not with the performance harness's fixtures",
                id.0
            );
            return None;
        }
        let want = self.streams.want(&id, SystemTime::now());
        if !matches!(want, Want::Cached(_)) && !self.resolver.helpers_installed() {
            // A first run: the resolve waits for the helpers' download (the player shows
            // Loading meanwhile). Said once per song asked for, never on a timer.
            log!("resolve {}: waiting for yt-dlp and deno to download", id.0);
            (self.ui.events)(super::UiEvent::Toast(getting_ready()));
        }
        match want {
            Want::Cached(s) => {
                log!("resolve {}: resolved ahead", id.0);
                Some(Input::Resolved {
                    id,
                    result: Ok(super::stream_info(&s)),
                })
            }
            Want::Running => {
                log!("resolve {}: already running ({})", id.0, self.time_left());
                None
            }
            Want::Start => {
                log!("resolve {} ({})", id.0, self.time_left());
                self.spawn_resolve(id, None);
                None
            }
        }
    }

    /// A resolve, ahead of a click when `urgent` is given (told when the player asks for
    /// `id` meanwhile).
    fn spawn_resolve(&self, id: TrackId, urgent: Option<Arc<Notify>>) -> AbortHandle {
        let (resolver, tx) = (self.resolver.clone(), self.tx.clone());
        tokio::spawn(async move {
            use featherplay_streams::StreamResolver;
            let started = Instant::now();
            let result = match &urgent {
                Some(urgent) => resolver.resolve_ahead(&id, urgent).await,
                None => resolver.resolve(&id).await,
            };
            let took = started.elapsed();
            if let Some(why) = resolver.take_fallback_reason(&id) {
                log!("resolve {} through yt-dlp, not natively: {why}", id.0);
            }
            // "resolved … in" counts until the controller reads the answer; a slow resolve
            // says here how much of that was the resolver itself.
            if took >= SLOW_RESOLVE {
                log!(
                    "resolve {}: the resolver took {:.1}s",
                    id.0,
                    took.as_secs_f64()
                );
            }
            let _ = tx.send(Msg::Resolved(id, result));
        })
        .abort_handle()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(expires_in: Duration) -> ResolvedStream {
        ResolvedStream {
            url: "https://example.invalid/a".into(),
            mime: "audio/mp4".into(),
            itag: 140,
            bitrate_kbps: 128,
            content_length: Some(1),
            expires_at: SystemTime::now() + expires_in,
            duration: None,
            available_at: None,
            video_title: None,
            first_tag: None,
            loudness_db: None,
        }
    }

    /// The longest a test waits for a task or a notification before it fails.
    const TEST_WAIT: Duration = Duration::from_secs(2);

    fn id(s: &str) -> TrackId {
        TrackId(s.into())
    }

    fn ids(started: Vec<(TrackId, Arc<Notify>)>) -> Vec<TrackId> {
        started.into_iter().map(|(id, _)| id).collect()
    }

    fn waiting(s: &Streams) -> Vec<TrackId> {
        s.waiting.iter().map(|(id, _)| id.clone()).collect()
    }

    const HOURS: Duration = Duration::from_secs(5 * 3600);

    #[test]
    fn a_resolved_stream_answers_the_player() {
        let mut s = Streams::default();
        let now = SystemTime::now();
        assert_eq!(s.want(&id("a"), now), Want::Start);
        assert_eq!(s.want(&id("a"), now), Want::Running, "no second resolve");
        s.done(&id("a"), Some(&stream(HOURS)), &[]);
        assert!(matches!(s.want(&id("a"), now), Want::Cached(_)));
    }

    #[test]
    fn a_stream_near_expiry_is_resolved_again() {
        let mut s = Streams::default();
        let now = SystemTime::now();
        s.want(&id("a"), now);
        s.done(&id("a"), Some(&stream(Duration::from_secs(60))), &[]);
        assert_eq!(s.want(&id("a"), now), Want::Start);
    }

    #[test]
    fn hints_are_bounded_and_deduplicated() {
        let mut s = Streams::default();
        let now = SystemTime::now();
        for x in ["a", "b", "c", "d", "e", "f"] {
            s.hint(&id(x), now);
        }
        s.hint(&id("f"), now);
        // The newest three start; the queue kept the four newest.
        assert_eq!(ids(s.start_hints()), [id("f"), id("e"), id("d")]);
        assert!(s.start_hints().is_empty(), "every slot taken");
        assert_eq!(waiting(&s), [id("c")]);
        // Running or cached: no new hint.
        s.hint(&id("f"), now);
        assert_eq!(s.waiting.len(), 1);
        let (_, hint) = s.done(&id("f"), Some(&stream(HOURS)), &[]);
        assert!(hint);
        s.hint(&id("f"), now);
        assert_eq!(ids(s.start_hints()), [id("c")]);
    }

    #[test]
    fn the_player_takes_over_a_running_hint() {
        let mut s = Streams::default();
        let now = SystemTime::now();
        s.hint(&id("a"), now);
        s.hint(&id("b"), now);
        s.start_hints();
        assert_eq!(s.want(&id("a"), now), Want::Running);
        let (_, hint) = s.done(&id("a"), Some(&stream(HOURS)), &[]);
        assert!(!hint, "asked for by the player while it ran");
        // A waiting hint the player asks for starts as the player's own.
        s.hint(&id("c"), now);
        s.hint(&id("d"), now);
        assert_eq!(s.want(&id("c"), now), Want::Start);
        assert_eq!(waiting(&s), [id("d")]);
    }

    #[test]
    fn a_refused_stream_is_forgotten() {
        let mut s = Streams::default();
        let now = SystemTime::now();
        s.want(&id("a"), now);
        s.done(&id("a"), Some(&stream(HOURS)), &[]);
        assert!(s.forget(&id("a")).is_some());
        assert_eq!(s.want(&id("a"), now), Want::Start);
    }

    /// A refused preload names its URL only: the stream with that URL goes, and its id is
    /// what gets resolved again.
    #[test]
    fn a_stream_is_forgotten_by_its_url() {
        let mut s = Streams::default();
        let now = SystemTime::now();
        let mut a = stream(HOURS);
        a.url = "https://example.invalid/a".into();
        s.want(&id("a"), now);
        s.done(&id("a"), Some(&a), &[]);
        assert_eq!(s.forget_url("https://example.invalid/b"), None);
        assert_eq!(s.forget_url(&a.url), Some(id("a")));
        assert_eq!(s.want(&id("a"), now), Want::Start);
    }

    /// A preload refused again right after its re-resolve isn't resolved a third time
    /// while paused: Play resolves it then. No request loop.
    #[test]
    fn a_refused_preload_is_resolved_again_once() {
        let mut s = Streams::default();
        assert!(s.again_after_refusal(&id("a")));
        assert!(!s.again_after_refusal(&id("a")), "not twice in a row");
        assert!(s.again_after_refusal(&id("b")));
    }

    /// The queue's tracks ahead resolve nearest first, beside a hovered row.
    #[test]
    fn the_tracks_ahead_resolve_nearest_first() {
        let mut s = Streams::default();
        let now = SystemTime::now();
        s.ahead(None, &[id("q1"), id("q2")], now);
        assert_eq!(ids(s.start_hints()), [id("q1"), id("q2")]);
        // Here already, or running: nothing more.
        s.ahead(None, &[id("q1"), id("q2")], now);
        assert!(s.start_hints().is_empty());
        assert!(s.waiting.is_empty());
    }

    /// A queue edit, shuffle or repeat moves other tracks ahead: the ones no longer ahead
    /// stop (running) or go (waiting); hovered rows stay.
    #[test]
    fn a_queue_change_cancels_the_resolves_ahead_it_made_stale() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let task = rt.spawn(std::future::pending::<()>());
        let mut s = Streams::default();
        let now = SystemTime::now();
        s.ahead(None, &[id("q1"), id("q2")], now);
        s.start_hints();
        s.started(&id("q1"), task.abort_handle());
        s.hint(&id("row"), now);
        assert_eq!(s.ahead(None, &[id("q2"), id("q3")], now), [id("q1")]);
        // A task never aborted would never end: the timeout fails the test instead.
        let ended = rt.block_on(async { tokio::time::timeout(TEST_WAIT, task).await });
        let ended = ended.expect("q1's task wasn't stopped");
        assert!(ended.unwrap_err().is_cancelled(), "q1 stopped");
        assert_eq!(ids(s.start_hints()), [id("q3"), id("row")]);
        assert!(s.running.iter().all(|r| r.id != id("q1")));
    }

    /// Paused or stopped: the queue's resolves ahead stop and none starts.
    #[test]
    fn nothing_resolves_ahead_while_paused() {
        let mut s = Streams::default();
        let now = SystemTime::now();
        s.ahead(None, &[id("q1"), id("q2"), id("q3")], now);
        s.start_hints();
        assert_eq!(s.ahead(None, &[], now), [id("q1"), id("q2"), id("q3")]);
        assert!(s.running.is_empty() && s.waiting.is_empty());
        assert!(s.start_hints().is_empty());
    }

    /// A resolve ahead that failed isn't asked again on every state change: no loop.
    #[test]
    fn a_failed_resolve_ahead_is_not_asked_again() {
        let mut s = Streams::default();
        let now = SystemTime::now();
        s.ahead(None, &[id("q1")], now);
        s.start_hints();
        s.done(&id("q1"), None, &[]);
        s.ahead(None, &[id("q1")], now);
        assert!(s.start_hints().is_empty());
        // The player still resolves it when it gets there.
        assert_eq!(s.want(&id("q1"), now), Want::Start);
    }

    /// The player asking for a song whose resolve ahead runs tells that resolve, so it
    /// stops waiting for its turn at yt-dlp.
    #[test]
    fn the_player_taking_over_a_resolve_ahead_hurries_it() {
        let mut s = Streams::default();
        let now = SystemTime::now();
        s.ahead(None, &[id("q1")], now);
        let started = s.start_hints();
        let urgent = started[0].1.clone();
        assert_eq!(s.want(&id("q1"), now), Want::Running);
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let told = rt.block_on(async { tokio::time::timeout(TEST_WAIT, urgent.notified()).await });
        assert!(told.is_ok());
        // Taken over: a queue change no longer stops it.
        assert!(s.ahead(None, &[], now).is_empty());
    }

    /// A song starts: the track two ahead is now the next one, the player's own preload.
    /// Its resolve ahead keeps running, for the player to take over.
    #[test]
    fn the_track_moving_up_to_next_keeps_its_resolve_ahead() {
        let mut s = Streams::default();
        let now = SystemTime::now();
        s.ahead(Some(&id("n")), &[id("q1"), id("q2")], now);
        s.start_hints();
        assert!(
            s.ahead(Some(&id("q1")), &[id("q2"), id("q3")], now)
                .is_empty()
        );
        assert_eq!(s.want(&id("q1"), now), Want::Running);
    }

    /// A stream refused, resolved again and refused again isn't resolved ahead on every
    /// later state change: the player resolves it when it gets there.
    #[test]
    fn a_stream_refused_twice_is_not_resolved_ahead_again() {
        let mut s = Streams::default();
        let now = SystemTime::now();
        assert!(s.again_after_refusal(&id("q1")));
        assert!(!s.again_after_refusal(&id("q1")));
        s.ahead(None, &[id("q1")], now);
        assert!(s.start_hints().is_empty());
    }

    #[test]
    fn the_players_streams_outlive_many_hints() {
        let mut s = Streams::default();
        let now = SystemTime::now();
        s.want(&id("current"), now);
        s.done(&id("current"), Some(&stream(HOURS)), &[]);
        for i in 0..KEEP * 2 {
            let h = id(&format!("h{i}"));
            s.hint(&h, now);
            s.start_hints();
            s.done(&h, Some(&stream(HOURS)), &[id("current")]);
        }
        assert_eq!(s.cache.len(), KEEP);
        assert!(s.get(&id("current")).is_some());
        assert!(s.get(&id("h0")).is_none(), "the oldest hint went");
    }

    fn from(ip: &str, expires_in: Duration) -> ResolvedStream {
        ResolvedStream {
            url: format!("https://example.invalid/videoplayback?expire=1&ip={ip}&c=X"),
            video_title: Some("Title".into()),
            duration: Some(Duration::from_secs(200)),
            ..stream(expires_in)
        }
    }

    /// The streams of the last session answer once a resolve of this one shows the same
    /// address in its URL (YouTube's URLs are for one address); until then, none does.
    #[test]
    fn saved_streams_answer_after_a_restart_from_the_same_address() {
        let now = SystemTime::now();
        let mut before = Streams::default();
        before.done(&id("a"), Some(&from("1.2.3.4", HOURS)), &[]);
        before.done(&id("old"), Some(&from("1.2.3.4", FRESH_FOR / 2)), &[]);
        let mut s = Streams::default();
        s.restore(&before.saved(now), now);
        assert!(
            s.fresh(&id("a"), now).is_none(),
            "the address isn't known yet"
        );
        s.done(&id("b"), Some(&from("1.2.3.4", HOURS)), &[]);
        let parts = |x: &ResolvedStream| (x.url.clone(), x.video_title.clone(), x.duration);
        assert_eq!(
            s.fresh(&id("a"), now).map(parts),
            before.get(&id("a")).map(parts)
        );
        assert!(
            s.get(&id("old")).is_none(),
            "one about to expire isn't kept"
        );
    }

    #[test]
    fn a_saved_stream_keeps_its_loudness() {
        let now = SystemTime::now();
        let mut before = Streams::default();
        let loud = ResolvedStream {
            loudness_db: Some(7.17),
            ..from("1.2.3.4", HOURS)
        };
        before.done(&id("loud"), Some(&loud), &[]);
        before.done(&id("ytdlp"), Some(&from("1.2.3.4", HOURS)), &[]);
        let mut s = Streams::default();
        s.restore(&before.saved(now), now);
        let db = |t: &str| s.get(&id(t)).map(|x| x.loudness_db);
        assert_eq!((db("loud"), db("ytdlp")), (Some(Some(7.17)), Some(None)));
    }

    #[test]
    fn saved_streams_from_another_address_are_not_used() {
        let now = SystemTime::now();
        let mut before = Streams::default();
        before.done(&id("a"), Some(&from("1.2.3.4", HOURS)), &[]);
        let mut s = Streams::default();
        s.restore(&before.saved(now), now);
        s.done(&id("b"), Some(&from("5.6.7.8", HOURS)), &[]);
        assert!(s.fresh(&id("a"), now).is_none());
        assert_eq!(s.want(&id("a"), now), Want::Start);
    }
}
