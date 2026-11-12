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
//! `ahead.rs` has the resolves ahead of a click (the hints and the rows on screen),
//! `saved.rs` the streams kept for the next session.

mod ahead;
mod saved;

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use featherplay_audio::StreamSource;
use featherplay_core::failure::getting_ready;
use featherplay_core::model::TrackId;
use featherplay_core::player::{Input, Player, Status};
use featherplay_streams::{Ahead, ResolvedStream};
use tokio::sync::Notify;
use tokio::task::AbortHandle;

pub(super) use saved::SAVED_STREAMS;
use saved::SavedFile;

use super::{Controller, Msg};

/// Resolved streams kept, oldest dropped first (but never the ones `done` is told to keep),
/// and saved for the next session: ~1.5 KB each.
const KEEP: usize = 200;
/// The newest streams `likely_server` counts.
const SERVER_SAMPLE: usize = 32;
/// Tracks the queue doesn't resolve ahead again for `FAILED_FOR` (their resolve failed, or
/// their stream was refused twice), and refused preloads remembered. The rows on screen
/// keep their own, `2 * PAGE_ROWS`.
const FAILED_KEPT: usize = 8;
const FAILED_FOR: Duration = Duration::from_secs(10 * 60);
/// A stream that expires sooner than this is resolved again (the player re-resolves at
/// 30 s before expiry, so a stream handed to it must outlive that comfortably).
const FRESH_FOR: Duration = Duration::from_secs(10 * 60);
/// A resolve that takes longer logs its own time (a normal one takes ~0.4 s).
const SLOW_RESOLVE: Duration = Duration::from_secs(1);

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
    /// A row on screen (`Streams::on_screen`).
    Page,
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
    /// A row on screen's resolve waiting out an ad gate (`at_gate`).
    at_gate: bool,
    /// `Streams::signed_out` when it started: one from before a sign-out isn't kept.
    since_sign_out: u64,
}

#[derive(Default)]
pub(super) struct Streams {
    /// Oldest first, one per id.
    cache: VecDeque<(TrackId, ResolvedStream)>,
    running: Vec<Running>,
    /// Newest last.
    waiting: VecDeque<(TrackId, Hint)>,
    /// Tracks given up on ahead, and when (`give_up`), newest last: the queue's, the rows
    /// on screen's.
    failed: VecDeque<(TrackId, Instant)>,
    page_failed: VecDeque<(TrackId, Instant)>,
    /// The stream loaded last.
    loaded: Option<StreamSource>,
    /// The tracks whose refused preload was resolved again lately (`again_after_refusal`).
    refused_preloads: VecDeque<TrackId>,
    /// Streams of the last session (`restore`) not resolved again since.
    restored: Vec<TrackId>,
    /// The `ip=` of the last stream resolved this session: once known, a restored one
    /// answers only for the same address.
    address: Option<String>,
    /// A stream came or went since `take_changed`.
    changed: bool,
    /// The writes of `SAVED_STREAMS`, and the number of the last one asked for.
    file: SavedFile,
    writes: u64,
    /// The rows on screen, the likeliest click first (`on_screen`).
    page: Vec<TrackId>,
    /// Those still to resolve, next first.
    page_waiting: VecDeque<TrackId>,
    /// When the resolves of rows on screen started, in the last `MINUTE`.
    page_starts: VecDeque<Instant>,
    /// Rows are resolved: a song plays.
    playing: bool,
    /// The URLs whose start a rest on their row asked for, newest last (`head_wanted`).
    heads_asked: VecDeque<String>,
    /// Sign-outs so far (`forget_all`): a resolve started before the last one plays, but
    /// isn't saved (`unsaved`), so the removed file doesn't come back with it.
    signed_out: u64,
    /// Kept streams of resolves started before the last sign-out.
    unsaved: Vec<TrackId>,
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
    /// One from the last session unless a resolve showed another address (a URL is for
    /// one): the network seldom changes between two sessions, and a URL refused anyway is
    /// resolved again (`is_from_last_session`).
    fn fresh(&self, id: &TrackId, now: SystemTime) -> Option<&ResolvedStream> {
        self.get(id).filter(|s| {
            s.expires_at > now + FRESH_FOR
                && (!self.restored.contains(id)
                    || self
                        .address
                        .as_deref()
                        .is_none_or(|ip| address(&s.url) == Some(ip)))
        })
    }

    /// The player asks for `id`.
    pub fn want(&mut self, id: &TrackId, now: SystemTime) -> Want {
        self.waiting.retain(|(w, _)| w != id);
        self.page_waiting.retain(|w| w != id);
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
            at_gate: false,
            since_sign_out: self.signed_out,
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

    /// A resolve ended: returns how long it took and the hint it was, if the player still
    /// hasn't asked for it. A stream is kept; when that makes too many, the oldest not in
    /// `keep` (the player's current and next tracks) goes.
    pub fn done(
        &mut self,
        id: &TrackId,
        stream: Option<&ResolvedStream>,
        keep: &[TrackId],
    ) -> (Duration, Option<Hint>) {
        let (took, hint, kept) = match self.running.iter().position(|r| r.id == *id) {
            Some(i) => {
                let r = self.running.swap_remove(i);
                (
                    r.since.elapsed(),
                    r.hint,
                    r.since_sign_out == self.signed_out,
                )
            }
            None => (Duration::ZERO, None, true),
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
                // Started before the sign-out: it plays, but isn't saved.
                self.unsaved.retain(|u| u != id);
                if kept {
                    self.changed = true;
                } else {
                    self.unsaved.push(id.clone());
                }
            }
            None => {
                if let Some(hint @ (Hint::Queue | Hint::Page)) = hint {
                    self.give_up(id, hint);
                }
            }
        }
        (took, hint)
    }

    /// Drops the stream of `id` (its URL was refused); returns it.
    pub fn forget(&mut self, id: &TrackId) -> Option<ResolvedStream> {
        let i = self.cache.iter().position(|(c, _)| c == id)?;
        self.changed = true;
        self.cache.remove(i).map(|(_, s)| s)
    }

    /// Whether a refused preload of `id` is resolved again ahead: not when it was one of
    /// the last resolved again, so a URL refused every time can't loop while paused. Nor is
    /// it resolved ahead for the queue again for a while then: the player resolves it.
    pub fn again_after_refusal(&mut self, id: &TrackId) -> bool {
        if self.refused_preloads.contains(id) {
            self.give_up(id, Hint::Queue);
            return false;
        }
        self.refused_preloads.push_back(id.clone());
        if self.refused_preloads.len() > FAILED_KEPT {
            self.refused_preloads.pop_front();
        }
        true
    }

    /// Drops the stream whose URL is `url` (refused while preloaded); returns its id.
    pub fn forget_url(&mut self, url: &str) -> Option<TrackId> {
        let i = self.cache.iter().position(|(_, s)| s.url == url)?;
        self.changed = true;
        self.cache.remove(i).map(|(id, _)| id)
    }

    /// The server (`scheme://host`) the next stream most likely comes from: the one most
    /// of the newest `SERVER_SAMPLE` streams for this address are on, the newer of two with
    /// as many. A stream's server repeats (4 servers over 32 streams kept, 2026-11-09), and
    /// this guesses right about half the time, the last stream's server a third.
    pub fn likely_server(&self) -> Option<&str> {
        let servers: Vec<&str> = self
            .cache
            .iter()
            .filter(|(_, s)| self.address.is_none() || address(&s.url) == self.address.as_deref())
            .filter_map(|(_, s)| server(&s.url))
            .collect();
        let servers = &servers[servers.len().saturating_sub(SERVER_SAMPLE)..];
        // Oldest first: of equal counts `max_by_key` keeps the last, the newest.
        servers
            .iter()
            .copied()
            .max_by_key(|s| servers.iter().filter(|o| *o == s).count())
    }
}

/// The `scheme://host` of an `http(s)` URL.
fn server(url: &str) -> Option<&str> {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    let end = url.len() - rest.len() + rest.find(['/', '?']).unwrap_or(rest.len());
    Some(&url[..end])
}

/// Whether the player waits for the stream of `id` to play it: a click on the current
/// track, not the next one's preload nor a load while paused.
fn waits_to_play(player: &Player, id: &TrackId) -> bool {
    player.wants_play()
        && *player.status() == Status::Loading
        && player
            .queue()
            .current()
            .is_some_and(|t| player.play_id(t) == *id)
}

impl Controller {
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

    /// `id`'s resolve started to wait for the helpers' download: a first run, a song the
    /// native request couldn't give, and yt-dlp not there yet. Said when the player waits
    /// for that song (it shows Loading meanwhile); a resolve ahead of a click waits quietly,
    /// until the player asks for it (`resolve`). Once per wait, never on a timer.
    pub(super) fn helpers_wait(&mut self, id: &TrackId) {
        if waits_to_play(&self.player, id) {
            (self.ui.events)(super::UiEvent::Toast(getting_ready()));
        }
    }

    /// `Effect::Resolve`: a stream that's here answers at once.
    pub(super) fn resolve(&mut self, id: TrackId) -> Option<Input> {
        if let Some(fixture_stream) = self.fixture_stream {
            match fixture_stream() {
                Some(stream) => {
                    log!("resolve {}: the performance harness's local file", id.0);
                    let _ = self.tx.send(Msg::Resolved(id, Ok(stream)));
                }
                None => log!(
                    "resolve {}: not with the performance harness's fixtures",
                    id.0
                ),
            }
            return None;
        }
        let want = self.streams.want(&id, SystemTime::now());
        if !matches!(want, Want::Cached(_))
            && waits_to_play(&self.player, &id)
            && let Some(server) = self.streams.likely_server()
        {
            // A click that waits for its resolve: the stream's request finds a connection
            // open, ~100 ms sooner, when its server is the one guessed.
            self.audio.connect(server);
        }
        match want {
            Want::Cached(s) => {
                log!("resolve {}: resolved ahead, via {}", id.0, s.via.label());
                Some(Input::Resolved {
                    id,
                    result: Ok(super::stream_info(&s)),
                })
            }
            Want::Running => {
                log!("resolve {}: already running ({})", id.0, self.time_left());
                if self.resolver.waits_for_helpers(&id) {
                    // Resolved ahead, it has been waiting for the helpers quietly.
                    (self.ui.events)(super::UiEvent::Toast(getting_ready()));
                }
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
    /// `id` meanwhile), with how soon that may be.
    fn spawn_resolve(&self, id: TrackId, urgent: Option<(Arc<Notify>, Ahead)>) -> AbortHandle {
        let (resolver, tx) = (self.resolver.clone(), self.tx.clone());
        tokio::spawn(async move {
            use featherplay_streams::StreamResolver;
            let started = Instant::now();
            let result = match &urgent {
                Some((urgent, ahead)) => resolver.resolve_ahead(&id, urgent, *ahead).await,
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
    use featherplay_core::model::Track;
    use featherplay_core::player::{Effect, StreamInfo};
    use featherplay_streams::Via;

    use super::*;

    pub(super) fn stream(expires_in: Duration) -> ResolvedStream {
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
            via: Via::YtDlpDefault,
        }
    }

    pub(super) fn id(s: &str) -> TrackId {
        TrackId(s.into())
    }

    pub(super) const HOURS: Duration = Duration::from_secs(5 * 3600);

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

    /// Streams resolved in this order, each on the server named, all for the address `ip`.
    fn resolved_on(s: &mut Streams, servers: &[&str], ip: &str) {
        for server in servers {
            let stream = ResolvedStream {
                url: format!("https://{server}.example.invalid/videoplayback?expire=1&ip={ip}"),
                ..stream(HOURS)
            };
            let n = s.cache.len();
            s.done(&id(&format!("{ip}-{n}")), Some(&stream), &[]);
        }
    }

    /// The next stream comes from the server most of the kept ones are on more often than
    /// from the last one's.
    #[test]
    fn the_likely_server_is_the_one_most_kept_streams_are_on() {
        let mut s = Streams::default();
        assert_eq!(s.likely_server(), None, "no stream kept, no guess");
        resolved_on(&mut s, &["rr3", "rr1", "rr3", "rr5"], "1.2.3.4");
        assert_eq!(s.likely_server(), Some("https://rr3.example.invalid"));
    }

    #[test]
    fn the_newer_of_two_servers_with_as_many_streams_is_the_likely_one() {
        let mut s = Streams::default();
        resolved_on(&mut s, &["rr3", "rr1", "rr1", "rr3", "rr5"], "1.2.3.4");
        assert_eq!(s.likely_server(), Some("https://rr3.example.invalid"));
    }

    /// The last session's streams stand in until a resolve shows this session's address;
    /// from another address (another network, other servers) they no longer count.
    #[test]
    fn streams_for_another_address_do_not_make_a_server_likely() {
        let now = SystemTime::now();
        let mut before = Streams::default();
        resolved_on(&mut before, &["home", "home", "home"], "1.2.3.4");
        let mut s = Streams::default();
        s.restore(&before.saved(now), now);
        assert_eq!(s.likely_server(), Some("https://home.example.invalid"));
        resolved_on(&mut s, &["away"], "5.6.7.8");
        assert_eq!(s.likely_server(), Some("https://away.example.invalid"));
    }

    fn track(s: &str) -> Track {
        Track {
            id: id(s),
            title: s.into(),
            artists: Vec::new(),
            album: None,
            duration: None,
            video_type: featherplay_core::model::VideoType::Song,
            thumbnail: None,
            like: featherplay_core::model::LikeStatus::Indifferent,
            counterpart: None,
        }
    }

    /// A player that was just told to play `a`, then `b`.
    fn clicked() -> Player {
        let prefs = super::super::prefs(&featherplay_store::Settings::default());
        let mut player = Player::new(prefs);
        let effects = player.handle(Input::PlayTracks {
            tracks: vec![track("a"), track("b")],
            start: 0,
            radio: false,
        });
        assert!(effects.contains(&Effect::Resolve(id("a"))));
        player
    }

    /// A connection opens ahead for the song the player waits to play, and not for the
    /// next one, which it resolves while the current one plays.
    #[test]
    fn a_click_waits_for_its_stream_and_the_next_songs_preload_does_not() {
        let mut player = clicked();
        assert!(waits_to_play(&player, &id("a")), "the song clicked");
        let info = StreamInfo {
            expires_at: SystemTime::now() + HOURS,
            duration: None,
        };
        player.handle(Input::Resolved {
            id: id("a"),
            result: Ok(info),
        });
        let effects = player.handle(Input::AudioStarted);
        assert!(effects.contains(&Effect::Resolve(id("b"))));
        assert!(!waits_to_play(&player, &id("b")), "the next song");
    }

    #[test]
    fn a_song_paused_while_it_resolves_is_not_waited_for() {
        let mut player = clicked();
        player.handle(Input::Toggle);
        assert!(!waits_to_play(&player, &id("a")));
    }
}
