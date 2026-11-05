//! Resolved streams, kept until they near their expiry, and resolves started ahead of a
//! click: a row hovered for 120 ms or pressed, the first row of a list page that opens,
//! Home's featured song, the restored session's track, the track after next. A track whose
//! stream is here starts without a resolve, and with its first 256 KB fetched into memory
//! (`AudioEngine::prefetch`) without a request.
//!
//! The track played before the current one gets its first 256 KB fetched again when it's
//! left (its download went with it), so Previous starts from memory too.
//!
//! The player's own resolves start at once. Resolves ahead ("hints") take at most
//! `HINT_SLOTS` at a time; the newest hints go first and older ones are dropped past
//! `HINT_QUEUE`. Nothing resolves twice: a hint for a stream that's here, running or
//! waiting does nothing. No timers: hints start when they come in and when a resolve ends.

use std::collections::VecDeque;
use std::time::{Duration, Instant, SystemTime};

use featherplay_audio::StreamSource;
use featherplay_core::failure::getting_ready;
use featherplay_core::model::{Track, TrackId};
use featherplay_core::player::Input;
use featherplay_streams::ResolvedStream;

use super::{Controller, Msg};

/// Resolved streams kept, oldest dropped first (but never the ones `done` is told to keep).
const KEEP: usize = 32;
/// Resolves ahead running at once (the player's own come on top).
const HINT_SLOTS: usize = 2;
/// Hints waiting for a slot.
const HINT_QUEUE: usize = 4;
/// A stream that expires sooner than this is resolved again (the player re-resolves at
/// 30 s before expiry, so a stream handed to it must outlive that comfortably).
const FRESH_FOR: Duration = Duration::from_secs(10 * 60);
/// A resolve that takes longer logs its own time (a normal one takes ~0.4 s).
const SLOW_RESOLVE: Duration = Duration::from_secs(1);

struct Running {
    id: TrackId,
    since: Instant,
    /// Started ahead of a click; the player hasn't asked for it.
    hint: bool,
}

#[derive(Default)]
pub(super) struct Streams {
    /// Oldest first, one per id.
    cache: VecDeque<(TrackId, ResolvedStream)>,
    running: Vec<Running>,
    /// Newest last.
    waiting: VecDeque<TrackId>,
    /// The stream loaded last.
    loaded: Option<StreamSource>,
    /// The last track whose refused preload was resolved again (`again_after_refusal`).
    refused_preload: Option<TrackId>,
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
    fn fresh(&self, id: &TrackId, now: SystemTime) -> Option<&ResolvedStream> {
        self.get(id).filter(|s| s.expires_at > now + FRESH_FOR)
    }

    /// The player asks for `id`.
    pub fn want(&mut self, id: &TrackId, now: SystemTime) -> Want {
        self.waiting.retain(|w| w != id);
        if let Some(s) = self.fresh(id, now) {
            return Want::Cached(Box::new(s.clone()));
        }
        if let Some(r) = self.running.iter_mut().find(|r| r.id == *id) {
            r.hint = false;
            return Want::Running;
        }
        self.running.push(Running {
            id: id.clone(),
            since: Instant::now(),
            hint: false,
        });
        Want::Start
    }

    /// Queues a resolve ahead for `id` unless it's here, running or queued already.
    pub fn hint(&mut self, id: &TrackId, now: SystemTime) {
        if self.fresh(id, now).is_some()
            || self.running.iter().any(|r| r.id == *id)
            || self.waiting.contains(id)
        {
            return;
        }
        self.waiting.push_back(id.clone());
        if self.waiting.len() > HINT_QUEUE {
            self.waiting.pop_front();
        }
    }

    /// The hints that can start now, newest first; they count as running.
    pub fn start_hints(&mut self) -> Vec<TrackId> {
        let mut out = Vec::new();
        while self.running.iter().filter(|r| r.hint).count() < HINT_SLOTS {
            let Some(id) = self.waiting.pop_back() else {
                break;
            };
            self.running.push(Running {
                id: id.clone(),
                since: Instant::now(),
                hint: true,
            });
            out.push(id);
        }
        out
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
            None => (Duration::ZERO, false),
        };
        if let Some(s) = stream {
            self.cache.retain(|(c, _)| c != id);
            self.cache.push_back((id.clone(), s.clone()));
            if self.cache.len() > KEEP
                && let Some(i) = self.cache.iter().position(|(c, _)| !keep.contains(c))
            {
                self.cache.remove(i);
            }
        }
        (took, hint)
    }

    /// Drops the stream of `id` (its URL was refused); returns it.
    pub fn forget(&mut self, id: &TrackId) -> Option<ResolvedStream> {
        let i = self.cache.iter().position(|(c, _)| c == id)?;
        self.cache.remove(i).map(|(_, s)| s)
    }

    /// Whether a refused preload of `id` is resolved again ahead: not when it was the last
    /// one resolved again, so a URL refused every time can't loop while paused.
    pub fn again_after_refusal(&mut self, id: &TrackId) -> bool {
        if self.refused_preload.as_ref() == Some(id) {
            return false;
        }
        self.refused_preload = Some(id.clone());
        true
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
            let id = self.player.play_id(t);
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

    /// The track after the next one, once the current one plays (the next one is the
    /// player's own preload).
    pub(super) fn hint_queue(&mut self) {
        let Some(t) = self.player.queue().upcoming().get(1).cloned() else {
            return;
        };
        self.hint(std::slice::from_ref(&t));
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

    /// A preloaded stream's URL was refused: it's resolved again ahead, so Play needn't wait
    /// for the refusal and then the resolve (the restored track, 2026-11-05).
    pub(super) fn preload_refused(&mut self, url: &str) {
        self.resolver.refused(url);
        let Some(id) = self.streams.forget_url(url) else {
            return;
        };
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
        for id in self.streams.start_hints() {
            log!("resolve {} ahead", id.0);
            self.spawn_resolve(id);
        }
    }

    /// `Effect::Resolve`: a stream that's here answers at once.
    pub(super) fn resolve(&mut self, id: TrackId) -> Option<Input> {
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
                self.spawn_resolve(id);
                None
            }
        }
    }

    fn spawn_resolve(&self, id: TrackId) {
        let (resolver, tx) = (self.resolver.clone(), self.tx.clone());
        tokio::spawn(async move {
            use featherplay_streams::StreamResolver;
            let started = Instant::now();
            let result = resolver.resolve(&id).await;
            let took = started.elapsed();
            if let Some(why) = resolver.take_fallback_reason() {
                log!("resolve {} [native] failed, so through yt-dlp: {why}", id.0);
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
        });
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
        }
    }

    fn id(s: &str) -> TrackId {
        TrackId(s.into())
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
        // The newest two start; the queue kept the four newest.
        assert_eq!(s.start_hints(), vec![id("f"), id("e")]);
        assert_eq!(s.start_hints(), Vec::<TrackId>::new(), "both slots taken");
        assert_eq!(s.waiting, VecDeque::from([id("c"), id("d")]));
        // Running or cached: no new hint.
        s.hint(&id("f"), now);
        assert_eq!(s.waiting.len(), 2);
        let (_, hint) = s.done(&id("f"), Some(&stream(HOURS)), &[]);
        assert!(hint);
        s.hint(&id("f"), now);
        assert_eq!(s.start_hints(), vec![id("d")]);
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
        assert_eq!(s.waiting, VecDeque::from([id("d")]));
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
}
