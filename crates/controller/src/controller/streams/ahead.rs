//! The player's own resolves start at once. Resolves ahead ("hints") take at most
//! `HINT_SLOTS` at a time; the newest hints go first, and older row hints are dropped past
//! `HINT_QUEUE`. Their yt-dlp runs take turns (`YtDlpResolver::resolve_ahead`). A row's
//! hint is for a click that may come any moment (`Ahead::Soon`), the queue's for later. Nothing
//! resolves twice: a hint for a stream that's here, running or waiting does nothing, nor
//! one for a song YouTube said lately is unavailable (`Streams::unavailable`). The
//! queue's hints follow the queue: a track no longer ahead has its resolve stopped, and
//! paused or stopped, none runs. No timers: hints start when they come in and when a
//! resolve ends.
//!
//! The song rows on screen (`Action::Prepare`: a list's rows once it opens or settles after
//! a scroll, Home's song shelves, Related, Up next) are resolved too while a song plays, so
//! a click on one needn't wait for its ad gate (~5-6 s): the top first, `PAGE_SLOTS` at a
//! time on top of the hints, at most `PAGE_ROWS` a screen and `PAGE_PER_MINUTE` a minute.
//! These are `Ahead::Maybe`: the native request only, its failure left for the click, and
//! no fetch of the stream's start until the pointer rests on the row. One waiting out an
//! ad gate leaves its slot to the next row. Rows that leave the screen (another page, a
//! scroll, the window minimised or hidden) have their resolves stopped; paused or stopped,
//! none runs. A row hovered or a track the queue wants takes its resolve over, so it keeps
//! running, and is resolved again in full if it stops short (`deferred`).

use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use featherplay_core::model::{Track, TrackId};
use featherplay_streams::{Ahead, ResolvedStream};
use tokio::sync::Notify;
use tokio::task::AbortHandle;

use super::{Controller, FAILED_FOR, FAILED_KEPT, Hint, Running, Streams};

/// Resolves ahead running at once (the player's own come on top): the queue's two and a
/// hovered row.
const HINT_SLOTS: usize = 3;
/// Row hints waiting for a slot.
const HINT_QUEUE: usize = 4;
/// The queue's tracks resolved ahead, the next one (the player's own preload) included.
const AHEAD: usize = 3;
/// Resolves of rows on screen asking at once, on top of `HINT_SLOTS`. One waiting out an ad
/// gate (~5-6 s, no request meanwhile, `at_gate`) leaves its slot to the next.
const PAGE_SLOTS: usize = 3;
/// The rows of a screen resolved, the top first.
const PAGE_ROWS: usize = super::super::PREPARE_ROWS;
/// Resolves of rows on screen started in a minute, at most, so a fast scroller can't send
/// dozens: two screens' worth.
const PAGE_PER_MINUTE: usize = 2 * PAGE_ROWS;
/// The streams whose start a rest on a row fetched that are remembered, so it isn't fetched
/// again: a few screens of rows.
const HEADS_ASKED: usize = 64;
const MINUTE: Duration = Duration::from_secs(60);

/// What a resolve ahead that stopped short of yt-dlp (`ResolveError::Deferred`) leaves
/// (`Streams::deferred`).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Deferred {
    /// The player asked for the song as it stopped: its own resolve starts now.
    Player,
    /// A hover's or the queue's now: it's resolved ahead again as theirs.
    Again,
    /// A row on screen's, given up on for a while, or one no longer tracked.
    Done,
}

impl Streams {
    /// Whether a hint for `id` would do nothing: its stream is here, running or waiting.
    fn covered(&self, id: &TrackId, now: SystemTime) -> bool {
        self.fresh(id, now).is_some()
            || self.running.iter().any(|r| r.id == *id)
            || self.waiting.iter().any(|(w, _)| w == id)
            || self.page_waiting.contains(id)
    }

    /// Queues a resolve ahead for `id` (a row) unless it's covered already. A row on screen
    /// becomes a hovered one: its resolve no longer stops when it leaves the screen.
    pub fn hint(&mut self, id: &TrackId, now: SystemTime) {
        self.page_waiting.retain(|w| w != id);
        self.take_over(id, Hint::Row);
        if self.covered(id, now) || self.known_unavailable(id).is_some() {
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
        self.page_waiting.retain(|w| !wanted(w));
        for id in ids.iter().chain(next) {
            self.take_over(id, Hint::Queue);
        }
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
        self.forget_failures();
        // The nearest last, so it starts first.
        for id in ids.iter().rev() {
            if !self.covered(id, now) && !self.has_failed(id) {
                self.waiting.push_back((id.clone(), Hint::Queue));
            }
        }
        stopped
    }

    /// A hint for `id` that runs for a row on screen is `hint`'s from now on: a row hovered,
    /// or a track the queue wants, so it no longer stops when the row leaves the screen.
    fn take_over(&mut self, id: &TrackId, hint: Hint) {
        if let Some(r) = self
            .running
            .iter_mut()
            .find(|r| r.id == *id && r.hint == Some(Hint::Page))
        {
            r.hint = Some(hint);
        }
    }

    /// The song rows on screen are now `ids`, the likeliest click first (none on another
    /// page, or with the window minimised or hidden). Resolves for rows no longer there
    /// stop, running or waiting; returns the running ones stopped.
    pub fn on_screen(&mut self, mut ids: Vec<TrackId>, now: SystemTime) -> Vec<TrackId> {
        ids.truncate(PAGE_ROWS);
        self.page = ids;
        self.sync_page(now)
    }

    /// Whether the rows on screen are resolved: while a song plays. Paused or stopped, their
    /// resolves stop; returns the running ones stopped.
    pub fn prepare_while(&mut self, playing: bool, now: SystemTime) -> Vec<TrackId> {
        self.playing = playing;
        self.sync_page(now)
    }

    /// The rows on screen waiting for a slot again, from `page`: the top first, none that's
    /// here, running, waiting as a hint, or failed lately.
    fn sync_page(&mut self, now: SystemTime) -> Vec<TrackId> {
        let (playing, page) = (self.playing, &self.page);
        let mut stopped = Vec::new();
        self.running.retain(|r| {
            let stale = r.hint == Some(Hint::Page) && !(playing && page.contains(&r.id));
            if stale {
                if let Some(task) = &r.task {
                    task.abort();
                }
                stopped.push(r.id.clone());
            }
            !stale
        });
        self.page_waiting.clear();
        if playing {
            self.forget_failures();
            for id in self.page.clone() {
                if !self.covered(&id, now) && !self.has_failed(&id) {
                    self.page_waiting.push_back(id);
                }
            }
        }
        stopped
    }

    /// The stream of `id` if it's here and its start wasn't asked for yet, for that start to
    /// be fetched: the pointer rests on its row, so a click may come. A row resolved while on
    /// screen has no start fetched yet, and one kept from earlier (the last session, an hour
    /// ago) may have none in memory. Once per URL, of the last `HEADS_ASKED`, so resting on
    /// rows again and again fetches nothing more.
    pub fn head_wanted(&mut self, id: &TrackId, now: SystemTime) -> Option<ResolvedStream> {
        let stream = self.fresh(id, now)?;
        if self.heads_asked.contains(&stream.url) {
            return None;
        }
        let stream = stream.clone();
        self.heads_asked.push_back(stream.url.clone());
        if self.heads_asked.len() > HEADS_ASKED {
            self.heads_asked.pop_front();
        }
        Some(stream)
    }

    fn forget_failures(&mut self) {
        let since = Instant::now().checked_sub(FAILED_FOR);
        let recent = |(_, at): &(TrackId, Instant)| since.is_none_or(|since| *at > since);
        self.failed.retain(recent);
        self.page_failed.retain(recent);
    }

    fn has_failed(&self, id: &TrackId) -> bool {
        self.failed
            .iter()
            .chain(&self.page_failed)
            .any(|(f, _)| f == id)
            || self.known_unavailable(id).is_some()
    }

    /// `id` isn't resolved ahead again for `FAILED_FOR`, as `hint`'s (a row on screen's is
    /// kept apart, so a page of them can't push the queue's out).
    pub(super) fn give_up(&mut self, id: &TrackId, hint: Hint) {
        let (failed, kept) = match hint {
            Hint::Page => (&mut self.page_failed, 2 * PAGE_ROWS),
            Hint::Row | Hint::Queue => (&mut self.failed, FAILED_KEPT),
        };
        failed.retain(|(f, _)| f != id);
        failed.push_back((id.clone(), Instant::now()));
        if failed.len() > kept {
            failed.pop_front();
        }
    }

    /// `id`'s resolve ahead of a click told its gate wait (`Msg::GateWait`): a row on
    /// screen's no longer takes a slot.
    pub fn at_gate(&mut self, id: &TrackId) {
        if let Some(r) = self.running.iter_mut().find(|r| r.id == *id) {
            r.at_gate = true;
        }
    }

    /// `id`'s resolve ahead stopped where it would have needed yt-dlp (`Ahead::Maybe`).
    pub fn deferred(&mut self, id: &TrackId) -> Deferred {
        let Some(i) = self.running.iter().position(|r| r.id == *id) else {
            return Deferred::Done;
        };
        match self.running.swap_remove(i).hint {
            None => Deferred::Player,
            Some(Hint::Page) => {
                self.give_up(id, Hint::Page);
                Deferred::Done
            }
            Some(hint @ (Hint::Row | Hint::Queue)) => {
                self.waiting.push_back((id.clone(), hint));
                Deferred::Again
            }
        }
    }

    /// The hints that can start now, newest first, then the rows on screen, top first, each
    /// with what tells it the player asks for its track and how soon that may be; they
    /// count as running.
    pub fn start_hints(&mut self) -> Vec<(TrackId, Arc<Notify>, Ahead)> {
        let mut out = Vec::new();
        let running = |s: &Self, page: bool| {
            s.running
                .iter()
                .filter(|r| r.hint.is_some_and(|h| (h == Hint::Page) == page))
                .filter(|r| !(page && r.at_gate))
                .count()
        };
        while running(self, false) < HINT_SLOTS {
            let Some((id, hint)) = self.waiting.pop_back() else {
                break;
            };
            let ahead = match hint {
                Hint::Row => Ahead::Soon,
                Hint::Queue => Ahead::Later,
                Hint::Page => Ahead::Maybe,
            };
            out.push(self.run_hint(id, hint, ahead));
        }
        let now = Instant::now();
        self.page_starts
            .retain(|at| now.duration_since(*at) < MINUTE);
        while running(self, true) < PAGE_SLOTS && self.page_starts.len() < PAGE_PER_MINUTE {
            let Some(id) = self.page_waiting.pop_front() else {
                break;
            };
            self.page_starts.push_back(now);
            out.push(self.run_hint(id, Hint::Page, Ahead::Maybe));
        }
        out
    }

    fn run_hint(&mut self, id: TrackId, hint: Hint, ahead: Ahead) -> (TrackId, Arc<Notify>, Ahead) {
        let urgent = Arc::new(Notify::new());
        self.running.push(Running {
            id: id.clone(),
            since: Instant::now(),
            hint: Some(hint),
            urgent: urgent.clone(),
            task: None,
            at_gate: false,
            since_sign_out: self.signed_out,
        });
        (id, urgent, ahead)
    }

    /// The hint for `id` runs as `task`, which `ahead` may stop.
    pub fn started(&mut self, id: &TrackId, task: AbortHandle) {
        if let Some(r) = self.running.iter_mut().find(|r| r.id == *id) {
            r.task = Some(task);
        }
    }
}

impl Controller {
    /// Resolves `tracks` ahead of a click (see the module docs).
    pub(crate) fn hint(&mut self, tracks: &[Track]) {
        for t in tracks {
            let id = self.play_id_of(t.id.clone(), Some(t.clone()));
            self.hint_play_id(&id);
        }
        self.start_hints();
    }

    /// A hint by video id alone (a row the window has no `Track` for).
    pub(crate) fn hint_id(&mut self, id: TrackId) {
        let id = self.play_id_of(id, None);
        self.hint_play_id(&id);
        self.start_hints();
    }

    /// A row the pointer rests on: resolved ahead, or, with its stream here (resolved while
    /// on screen, or kept from earlier), its stream's start fetched, as for a stream the
    /// hint resolves.
    fn hint_play_id(&mut self, id: &TrackId) {
        let now = SystemTime::now();
        if let Some(stream) = self.streams.head_wanted(id, now) {
            self.audio.prefetch(&super::super::source(&stream));
        }
        self.streams.hint(id, now);
    }

    /// The id the player would play for row `id`: its song version (`track` when the
    /// window has it), else the queue's track with that id.
    fn play_id_of(&mut self, id: TrackId, track: Option<Track>) -> TrackId {
        if let Some(mut t) = track {
            self.song_versions.fill(&mut t);
            return self.player.play_id(&t);
        }
        let q = self.player.queue();
        q.current()
            .into_iter()
            .chain(q.upcoming())
            .find(|t| t.id == id)
            .map(|t| self.player.play_id(t))
            .unwrap_or(id)
    }

    /// A resolve ahead stopped short of yt-dlp (`ResolveError::Deferred`).
    pub(crate) fn deferred(&mut self, id: TrackId) {
        match self.streams.deferred(&id) {
            Deferred::Player => {
                log!("resolve {}: asked for as its resolve ahead stopped", id.0);
                if let Some(input) = self.resolve(id) {
                    self.input(input);
                }
            }
            Deferred::Again => {
                log!(
                    "resolve {} ahead: hovered or up next, so again in full",
                    id.0
                );
                self.start_hints();
            }
            Deferred::Done => {
                log!("prepare {}: not natively, left for a click", id.0);
                self.start_hints();
            }
        }
    }

    /// `Action::Prepare`: the song rows on screen, the likeliest click first, resolved
    /// ahead while a song plays (see the module docs).
    pub(crate) fn prepare(&mut self, rows: Vec<(TrackId, Option<Track>)>) {
        let ids = rows
            .into_iter()
            .map(|(id, track)| self.play_id_of(id, track))
            .collect();
        for id in self.streams.on_screen(ids, SystemTime::now()) {
            log!("prepare {}: stopped, no longer on screen", id.0);
        }
        self.start_hints();
    }

    /// While the current track plays: the tracks after the next one, up to `AHEAD` with it
    /// (the next one is the player's own preload). Called again after each state change,
    /// so a queue edit, shuffle or repeat moves them.
    pub(crate) fn hint_queue(&mut self) {
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
        self.resolve_ahead(next, after, true);
        self.look_up_ahead(AHEAD);
    }

    /// Paused or stopped: the queue's resolves ahead stop, and the rows on screen's.
    pub(crate) fn stop_ahead(&mut self) {
        self.resolve_ahead(None, &[], false);
    }

    fn resolve_ahead(&mut self, next: Option<&TrackId>, after: &[TrackId], playing: bool) {
        let now = SystemTime::now();
        for id in self.streams.ahead(next, after, now) {
            log!("resolve {} ahead: stopped, no longer up next", id.0);
        }
        for id in self.streams.prepare_while(playing, now) {
            log!("prepare {}: stopped, nothing plays", id.0);
        }
        self.start_hints();
    }

    pub(crate) fn start_hints(&mut self) {
        // The performance harness's fixtures play nothing, so nothing is asked of YouTube.
        if self.fixture_stream.is_some() {
            return;
        }
        for (id, urgent, ahead) in self.streams.start_hints() {
            if ahead == Ahead::Maybe {
                log!("prepare {}: on screen", id.0);
            } else {
                log!("resolve {} ahead", id.0);
            }
            let task = self.spawn_resolve(id.clone(), Some((urgent, ahead)));
            self.streams.started(&id, task);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::Want;
    use super::super::tests::{HOURS, id, stream};
    use super::*;

    /// The longest a test waits for a task or a notification before it fails.
    const TEST_WAIT: Duration = Duration::from_secs(2);

    fn ids(started: Vec<(TrackId, Arc<Notify>, Ahead)>) -> Vec<TrackId> {
        started.into_iter().map(|(id, _, _)| id).collect()
    }

    fn waiting(s: &Streams) -> Vec<TrackId> {
        s.waiting.iter().map(|(id, _)| id.clone()).collect()
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
        assert_eq!(hint, Some(Hint::Row));
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
        assert_eq!(hint, None, "asked for by the player while it ran");
        // A waiting hint the player asks for starts as the player's own.
        s.hint(&id("c"), now);
        s.hint(&id("d"), now);
        assert_eq!(s.want(&id("c"), now), Want::Start);
        assert_eq!(waiting(&s), [id("d")]);
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

    /// A row's click may come any moment, the queue's tracks are minutes away: the
    /// resolver is told which (a row's gated answer gets its fresh one at once).
    #[test]
    fn a_rows_hint_resolves_for_a_click_soon_and_the_queues_for_later() {
        let mut s = Streams::default();
        let now = SystemTime::now();
        s.ahead(None, &[id("q1")], now);
        s.hint(&id("row"), now);
        let started: Vec<_> = s
            .start_hints()
            .into_iter()
            .map(|(id, _, ahead)| (id, ahead))
            .collect();
        assert_eq!(
            started,
            [(id("row"), Ahead::Soon), (id("q1"), Ahead::Later)]
        );
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

    /// YouTube said a song is unavailable: no hover, queue or row on screen resolves it
    /// again for `UNAVAILABLE_FOR`; after that they do.
    #[test]
    fn an_unavailable_song_is_not_resolved_ahead_for_a_while() {
        let mut s = Streams::default();
        let now = SystemTime::now();
        s.hint(&id("a"), now);
        s.start_hints();
        s.done(&id("a"), None, &[]);
        s.unavailable(&id("a"), "This video is not available");
        s.hint(&id("a"), now);
        s.ahead(None, &[id("a")], now);
        s.prepare_while(true, now);
        s.on_screen(vec![id("a")], now);
        assert!(s.start_hints().is_empty());

        for (_, at, _) in &mut s.unavailable {
            *at = Instant::now()
                .checked_sub(super::super::UNAVAILABLE_FOR)
                .unwrap();
        }
        s.hint(&id("a"), now);
        assert_eq!(ids(s.start_hints()), [id("a")]);
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

    fn page(started: &[(TrackId, Arc<Notify>, Ahead)]) -> Vec<TrackId> {
        started
            .iter()
            .filter(|(_, _, ahead)| *ahead == Ahead::Maybe)
            .map(|(id, _, _)| id.clone())
            .collect()
    }

    /// The rows on screen resolve top first, a few at a time, beside the queue's and a
    /// hovered row's resolves, and never a second time for a song one of those has.
    #[test]
    fn rows_on_screen_resolve_top_first_without_repeating_the_queue_or_a_hover() {
        let mut s = Streams::default();
        let now = SystemTime::now();
        s.prepare_while(true, now);
        s.ahead(None, &[id("q1")], now);
        s.hint(&id("hovered"), now);
        s.want(&id("playing"), now);
        let rows = ["playing", "q1", "hovered", "a", "b", "c", "d"].map(id);
        s.on_screen(rows.to_vec(), now);
        let started = s.start_hints();
        assert_eq!(started.len(), 2 + PAGE_SLOTS, "{started:?}");
        assert_eq!(page(&started), [id("a"), id("b"), id("c")]);
        s.done(&id("a"), Some(&stream(HOURS)), &[]);
        assert_eq!(page(&s.start_hints()), [id("d")]);
        // Resolved: on screen again, it isn't asked again.
        s.on_screen(rows.to_vec(), now);
        assert!(s.start_hints().is_empty());
    }

    /// Another page, or the rows scrolled away: their resolves stop, running or waiting.
    #[test]
    fn rows_that_leave_the_screen_stop_resolving() {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .unwrap();
        let task = rt.spawn(std::future::pending::<()>());
        let mut s = Streams::default();
        let now = SystemTime::now();
        s.prepare_while(true, now);
        s.on_screen(["a", "b", "c", "d"].map(id).to_vec(), now);
        s.start_hints();
        s.started(&id("a"), task.abort_handle());
        let stopped = s.on_screen(vec![id("x")], now);
        assert_eq!(stopped, [id("a"), id("b"), id("c")]);
        let ended = rt.block_on(async { tokio::time::timeout(TEST_WAIT, task).await });
        assert!(
            ended
                .expect("a's task wasn't stopped")
                .unwrap_err()
                .is_cancelled()
        );
        assert_eq!(page(&s.start_hints()), [id("x")], "d went with its page");
    }

    /// Paused or stopped, nothing on screen is resolved; playing again, it is.
    #[test]
    fn rows_on_screen_wait_while_paused() {
        let mut s = Streams::default();
        let now = SystemTime::now();
        s.on_screen(["a", "b"].map(id).to_vec(), now);
        assert!(s.start_hints().is_empty(), "nothing plays yet");
        s.prepare_while(true, now);
        assert_eq!(page(&s.start_hints()), [id("a"), id("b")]);
        assert_eq!(s.prepare_while(false, now), [id("a"), id("b")]);
        assert!(s.start_hints().is_empty());
        s.prepare_while(true, now);
        assert_eq!(page(&s.start_hints()), [id("a"), id("b")]);
    }

    /// A fast scroller sends a new screen of rows every moment: the lookups stop at
    /// `PAGE_PER_MINUTE` in the last minute.
    #[test]
    fn rows_on_screen_are_looked_up_at_most_so_often_a_minute() {
        let mut s = Streams::default();
        let now = SystemTime::now();
        s.prepare_while(true, now);
        let mut looked_up = 0;
        for screen in 0..10 {
            let rows: Vec<TrackId> = (0..12).map(|i| id(&format!("{screen}-{i}"))).collect();
            s.on_screen(rows, now);
            for id in page(&s.start_hints()) {
                looked_up += 1;
                s.done(&id, Some(&stream(HOURS)), &[]);
            }
            looked_up += s.start_hints().len();
        }
        assert_eq!(looked_up, PAGE_PER_MINUTE);
        // A minute on, they go on.
        for at in &mut s.page_starts {
            *at = Instant::now().checked_sub(MINUTE).unwrap();
        }
        assert!(!s.start_hints().is_empty());
    }

    /// A row on screen the pointer rests on: its resolve is a hover's from then on, and
    /// one already resolved gets its start fetched for the click.
    #[test]
    fn a_hovered_row_on_screen_is_a_hovers_resolve() {
        let mut s = Streams::default();
        let now = SystemTime::now();
        s.prepare_while(true, now);
        s.on_screen(["a", "b"].map(id).to_vec(), now);
        s.start_hints();
        s.done(&id("a"), Some(&stream(HOURS)), &[]);
        assert!(s.head_wanted(&id("a"), now).is_some());
        assert!(s.head_wanted(&id("a"), now).is_none(), "once");
        s.hint(&id("b"), now);
        assert!(
            s.on_screen(Vec::new(), now).is_empty(),
            "b stays, as a hover's"
        );
        // Stopped short of yt-dlp, it's resolved again as a hover's.
        assert_eq!(s.deferred(&id("b")), Deferred::Again);
        assert_eq!(ids(s.start_hints()), [id("b")]);
    }

    /// A row on screen stopped short of yt-dlp is left for the click; clicked as it
    /// stopped, the player resolves it; one no longer tracked (stopped, its answer late)
    /// starts nothing.
    #[test]
    fn a_row_resolve_that_stops_short_is_left_for_the_click() {
        let mut s = Streams::default();
        let now = SystemTime::now();
        s.prepare_while(true, now);
        s.on_screen(["a", "b"].map(id).to_vec(), now);
        s.start_hints();
        assert_eq!(s.deferred(&id("a")), Deferred::Done);
        s.on_screen(["a", "b"].map(id).to_vec(), now);
        assert!(
            s.start_hints().is_empty(),
            "a isn't asked again for a while"
        );
        assert_eq!(s.want(&id("b"), now), Want::Running);
        assert_eq!(s.deferred(&id("b")), Deferred::Player);
        assert_eq!(s.deferred(&id("b")), Deferred::Done, "no longer tracked");
    }

    /// A row waiting out its ad gate makes no request: the next row takes its slot.
    #[test]
    fn a_row_waiting_out_its_gate_leaves_its_slot() {
        let mut s = Streams::default();
        let now = SystemTime::now();
        s.prepare_while(true, now);
        s.on_screen(["a", "b", "c", "d"].map(id).to_vec(), now);
        assert_eq!(page(&s.start_hints()), [id("a"), id("b"), id("c")]);
        s.at_gate(&id("b"));
        assert_eq!(page(&s.start_hints()), [id("d")]);
    }
}
