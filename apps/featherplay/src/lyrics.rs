//! The Lyrics tab (ui/pages/lyrics.slint), beside Up next and Related in Now playing and
//! in the Up next panel, and Now playing's full lyrics view.
//!
//! Requests: only while lyrics are on screen (the full view, the tab in either panel, or the
//! desktop lyrics' strip: `LyricsData.live-changed`, from `LyricsWatch`), for the current
//! track, and again when the next track starts while they stay, once that track has stayed a
//! second (`SKIP_WAIT`: skipping through songs asks nothing); never in the background; never
//! with Settings' "Show lyrics" off. LRCLIB's synced lyrics first, when Settings allows it (it
//! gets the title, artist, album and length, and no cookies), then the song's own timed lyrics
//! from YouTube Music, then LRCLIB's plain lyrics, then the song's own plain ones (`settle`).
//! Results are kept per track for the session (a bounded map); failures aren't, so the tab asks
//! again the next time it opens on that track. Nor is the song's plain text when LRCLIB failed
//! rather than missed: a passing error mustn't hide synced lyrics. Shown, that fallback says
//! synced lyrics are unavailable for now, and while the view stays open on it LRCLIB is asked
//! once more a minute later (`RESYNC_AFTER`); synced lyrics found then take its place.
//!
//! Timing: nothing ticks. While sound plays and lyrics are on screen, one single-shot timer
//! waits for the next line's start, read from the bar's clock (`bar::position`); a pause, a
//! seek or any new player state aims it again. Paused, with no lyrics on screen, minimized
//! or hidden, no timer is armed; nor with plain lyrics or none.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use featherplay_core::model::TrackId;
use featherplay_lyrics::{Line, Lyrics, lrc};
use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};

use crate::app::Ctx;
use crate::bar;
use crate::follow::colour::Lch;
use crate::pages::settings;
use crate::ui::{LyricsData, LyricsState, Player, Theme};

mod lookup;

pub use lookup::Found;
use lookup::{Again, Failure, fetch, fetch_soon, fixture, fixture_for, resync_later, stop_wait};

/// A line is shown this long after it starts, so a timer that fires a little early never
/// shows the line before it again. A click on a line seeks this far in, for the same reason.
const LATE: Duration = Duration::from_millis(30);

/// Lookups started this session (LRCLIB and the song's own lyrics, or LRCLIB asked again):
/// what the tests count.
pub fn lookups() -> u64 {
    LOOKUPS.with(Cell::get)
}

enum Entry {
    Loading,
    Ready(Rc<Found>),
}

#[derive(Default)]
struct State {
    /// The tab is on screen.
    live: bool,
    cache: HashMap<TrackId, Entry>,
    /// The track the tab shows.
    shown: Option<TrackId>,
    /// Its synced lines; empty for plain lyrics, none, or while loading.
    lines: Rc<[Line]>,
    /// Bumped when the setting changes, so an answer to the old setting is dropped.
    generation: u64,
    retry: Retry,
}

/// When a failed lookup is asked again: once each time the view opens on its track, and on
/// each Try again, never in a loop. "Failed" is an error, or the song's plain lyrics shown
/// because LRCLIB failed; "none found" is an answer and is never asked again.
#[derive(Debug, Default)]
struct Retry {
    /// The track whose last lookup failed.
    failed: Option<TrackId>,
    /// This open has already asked again.
    asked: bool,
}

impl Retry {
    fn failed(&mut self, id: TrackId) {
        self.failed = Some(id);
    }

    fn succeeded(&mut self, id: &TrackId) {
        if self.failed.as_ref() == Some(id) {
            self.failed = None;
        }
    }

    fn closed(&mut self) {
        self.asked = false;
    }

    /// The view opened on `shown`: whether to ask for it again now.
    fn opened(&mut self, shown: Option<&TrackId>) -> bool {
        if self.asked || shown.is_none() || self.failed.as_ref() != shown {
            return false;
        }
        self.asked = true;
        self.failed = None;
        true
    }

    /// Try again on `shown`: whether to ask for it again now.
    fn try_again(&mut self, shown: Option<&TrackId>) -> bool {
        if shown.is_none() || self.failed.as_ref() != shown {
            return false;
        }
        self.failed = None;
        true
    }
}

thread_local! {
    static STATE: RefCell<State> = RefCell::new(State::default());
    static TIMER: Timer = Timer::default();
    /// Asks LRCLIB once more after an outage (`RESYNC_AFTER`).
    static RESYNC: Timer = Timer::default();
    /// Looks up the track that stayed, `SKIP_WAIT` after a track change.
    static WAIT: Timer = Timer::default();
    /// Held while `WAIT` runs: a lookup is due (`--snapshot` and the tests wait for it).
    static WAITING: RefCell<Option<crate::busy::Busy>> = const { RefCell::new(None) };
    static SNAPSHOT: Cell<bool> = const { Cell::new(false) };
    /// The current cover's colour, once known.
    static COVER: Cell<Option<Lch>> = const { Cell::new(None) };
    static LOOKUPS: Cell<u64> = const { Cell::new(0) };
}

/// Once at startup: connect `LyricsData`'s callbacks.
pub fn init(ctx: &Rc<Ctx>) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<LyricsData>();
    let c = ctx.clone();
    data.on_live_changed(move |live| live_changed(&c, live));
    let c = ctx.clone();
    data.on_seek_line(move |i| seek_line(&c, i));
    let c = ctx.clone();
    data.on_retry(move || {
        let ask = STATE.with(|s| {
            let mut s = s.borrow_mut();
            let shown = s.shown.clone();
            s.retry.try_again(shown.as_ref())
        });
        if ask {
            ask_again(&c);
        }
    });
    refresh_inks(&ui);
}

/// The playing song's cover colour (`follow`, once per song; `None` without a cover): the lyrics'
/// panel and lines are made from it.
pub fn set_cover(ui: &crate::ui::AppWindow, seed: Option<Lch>) {
    COVER.with(|c| c.set(seed));
    refresh_inks(ui);
}

/// Sets the lyrics' colours from the cover and, for grey art or no cover, a graphite panel
/// from the cover, or with a fixed accent that accent (`Theme.preset-dark`: red, or the
/// user's preset or custom colour). Called again when the Accent setting changes.
pub fn refresh_inks(ui: &crate::ui::AppWindow) {
    let theme = ui.global::<Theme>();
    let fixed = (!theme.get_follow()).then(|| theme.get_preset_dark());
    let inks = crate::follow::lyric_inks_for(COVER.with(Cell::get), fixed);
    let data = ui.global::<LyricsData>();
    data.set_ink_dark(inks.dark);
    data.set_ink_light(inks.light);
    crate::desktop_lyrics::inks_changed(ui);
}

/// Plain lyrics as lines, in the synced lines' type: blank lines between verses kept as one
/// gap ("").
fn plain_lines(text: &str) -> Vec<SharedString> {
    let mut out: Vec<SharedString> = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() && out.last().is_none_or(|l| l.is_empty()) {
            continue;
        }
        out.push(line.into());
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out
}

/// Settings' "Show lyrics" was loaded or changed. Off, the full view closes and the Lyrics
/// toggles and tab go (`LyricsData.on`), so no view is on screen and nothing is looked up.
/// Also tells the views whether LRCLIB is asked, for the line saying where featherplay looked.
pub fn on_changed(ctx: &Rc<Ctx>, on: bool) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<LyricsData>();
    if !on {
        data.set_full(false);
    }
    data.set_on(on);
    data.set_lrclib(settings::lyrics_lrclib());
}

/// The tab came on screen, or left it. Never on screen with lyrics off in Settings.
pub fn live_changed(ctx: &Rc<Ctx>, live: bool) {
    let live = live && settings::lyrics_shown();
    let mut dropped = false;
    let ask = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.live = live;
        if !live {
            s.retry.closed();
            RESYNC.with(Timer::stop);
            // A lookup still waiting is dropped; reopening looks the track up.
            if stop_wait() {
                s.shown = None;
                dropped = true;
            }
            return false;
        }
        let shown = s.shown.clone();
        s.retry.opened(shown.as_ref())
    });
    if ask {
        ask_again(ctx);
    } else if live {
        show(ctx, false);
    } else {
        TIMER.with(Timer::stop);
        // Not "Finding lyrics…" for a lookup that won't happen.
        if dropped && let Some(ui) = ctx.window() {
            ui.global::<LyricsData>().set_state(LyricsState::Idle);
        }
    }
}

/// Looks the shown track up again (its failed lookup isn't cached): `show` takes it as new.
fn ask_again(ctx: &Rc<Ctx>) {
    STATE.with(|s| s.borrow_mut().shown = None);
    show(ctx, false);
}

/// The controller's state changed (track, play/pause, seek). A new track is looked up only
/// once it has stayed `SKIP_WAIT`. With no lyrics on screen nothing is looked up, but the state
/// goes back to idle for a new track, so the Lyrics toggles never say "No lyrics" for a song
/// that wasn't looked up.
pub fn player_changed(ctx: &Rc<Ctx>) {
    let (live, shown) = STATE.with(|s| {
        let s = s.borrow();
        (s.live, s.shown.clone())
    });
    if live {
        show(ctx, true);
        return;
    }
    let current = ctx.player().and_then(|s| s.current).map(|t| t.id);
    if shown.is_some() && current == shown {
        return;
    }
    // Shown again later, the track is taken as new (from the cache, or looked up).
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.shown = None;
        s.lines = Rc::from([]);
    });
    if let Some(ui) = ctx.window() {
        let data = ui.global::<LyricsData>();
        if data.get_state() != LyricsState::Idle {
            data.set_state(LyricsState::Idle);
        }
    }
}

/// The LRCLIB setting changed: what was found under the old one is forgotten.
pub fn setting_changed(ctx: &Rc<Ctx>) {
    RESYNC.with(Timer::stop);
    if let Some(ui) = ctx.window() {
        ui.global::<LyricsData>()
            .set_lrclib(settings::lyrics_lrclib());
    }
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.cache.clear();
        s.shown = None;
        s.lines = Rc::from([]);
        s.generation += 1;
    });
    player_changed(ctx);
}

/// Shows the current track's lyrics, asking for them if they aren't known (`wait`: after
/// `SKIP_WAIT`, for a track change), and times the current line.
fn show(ctx: &Rc<Ctx>, wait: bool) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<LyricsData>();
    let track = ctx.player().and_then(|s| s.current);
    let id = track.as_ref().map(|t| t.id.clone());
    let changed = STATE.with(|s| s.borrow().shown != id);
    if changed {
        RESYNC.with(Timer::stop);
        stop_wait();
        stop_feed();
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.shown = id;
            s.lines = Rc::from([]);
        });
        match track {
            None => data.set_state(LyricsState::Idle),
            Some(track) => {
                let known = STATE.with(|s| match s.borrow().cache.get(&track.id) {
                    Some(Entry::Ready(found)) => Some(Some(found.clone())),
                    Some(Entry::Loading) => Some(None),
                    None => None,
                });
                match known {
                    Some(Some(found)) => {
                        fill(&data, &found);
                        if found.synced_unavailable() {
                            resync_later(ctx, &track.id, Again::Synced);
                        }
                    }
                    // Already on its way.
                    Some(None) => data.set_state(LyricsState::Loading),
                    None => {
                        data.set_state(LyricsState::Loading);
                        if wait {
                            fetch_soon(ctx, track);
                        } else {
                            fetch(ctx, track);
                        }
                    }
                }
            }
        }
    }
    sync(ctx);
}

/// Puts `found` on the tab.
fn fill(data: &LyricsData, found: &Found) {
    data.set_credit(found.credit.as_str().into());
    data.set_synced_unavailable(found.synced_unavailable());
    match &found.lyrics {
        Some(Lyrics::Synced(lines)) => {
            let texts: Vec<SharedString> = lines.iter().map(|l| l.text.as_str().into()).collect();
            data.set_current(-1);
            data.set_lines(ModelRc::from(feed(texts)));
            STATE.with(|s| s.borrow_mut().lines = Rc::from(lines.as_slice()));
            data.set_state(LyricsState::Synced);
        }
        Some(Lyrics::Plain(text)) => {
            data.set_plain(text.as_str().into());
            data.set_plain_lines(ModelRc::from(feed(plain_lines(text))));
            data.set_state(LyricsState::Plain);
        }
        None => data.set_state(LyricsState::None),
    }
}

/// Lines handed to the views per `FEED_EVERY`.
const FEED_LINES: usize = 4;
/// About one frame at 240 Hz.
const FEED_EVERY: Duration = Duration::from_millis(4);

thread_local! {
    /// Hands the rest of a song's lines over, `FEED_LINES` at a time.
    static FEED: Timer = Timer::default();
    /// Held while lines are still to come (`--snapshot` and the tests wait for them).
    static FEEDING: RefCell<Option<crate::busy::Busy>> = const { RefCell::new(None) };
}

/// A model that receives `lines` a few at a time: each line is a text every lyrics view
/// shapes and lays out in the frame it arrives, and all of a song's at once (80 lines, in the
/// side tab, the full view and a hidden tab) cost ~15–25 ms in one frame at 4K. Spread over a
/// few frames, ~2 ms each. The first lines come at once; a new song's lines replace the rest.
fn feed(lines: Vec<SharedString>) -> Rc<VecModel<SharedString>> {
    stop_feed();
    let at_once = SNAPSHOT.with(Cell::get) || lines.len() <= FEED_LINES;
    let first = if at_once { lines.len() } else { FEED_LINES };
    let mut rest = lines.into_iter();
    let model = Rc::new(VecModel::from(
        rest.by_ref().take(first).collect::<Vec<_>>(),
    ));
    let rest: Vec<SharedString> = rest.collect();
    if rest.is_empty() {
        return model;
    }
    FEEDING.with(|f| *f.borrow_mut() = Some(crate::busy::Busy::start()));
    let weak = Rc::downgrade(&model);
    let pending = RefCell::new(rest.into_iter());
    FEED.with(|t| {
        t.start(TimerMode::Repeated, FEED_EVERY, move || {
            let Some(model) = weak.upgrade() else {
                stop_feed();
                return;
            };
            let mut pending = pending.borrow_mut();
            for line in pending.by_ref().take(FEED_LINES) {
                model.push(line);
            }
            if pending.as_slice().is_empty() {
                stop_feed();
            }
        });
    });
    model
}

/// The lines still to come aren't wanted any more (another song's are, or none).
fn stop_feed() {
    FEED.with(Timer::stop);
    FEEDING.with(|f| f.borrow_mut().take());
}

/// Times the lines (`time_lines`) and shows the change on the desktop lyrics' strip, if it
/// shows.
fn sync(ctx: &Rc<Ctx>) {
    time_lines(ctx);
    crate::desktop_lyrics::refresh(ctx);
}

/// The synced lines shown now (empty for plain lyrics, none, or while looking).
pub fn synced_lines() -> Rc<[Line]> {
    STATE.with(|s| s.borrow().lines.clone())
}

/// Marks the line playing now and arms the timer for the next one, while sound plays and
/// the tab is on screen.
fn time_lines(ctx: &Rc<Ctx>) {
    let (lines, live) = STATE.with(|s| {
        let s = s.borrow();
        (s.lines.clone(), s.live)
    });
    let Some(ui) = ctx.window() else { return };
    let (Some(at), false) = (bar::position(), lines.is_empty()) else {
        TIMER.with(Timer::stop);
        return;
    };
    let data = ui.global::<LyricsData>();
    let current = lrc::line_at(&lines, at).map_or(-1, |i| i32::try_from(i).unwrap_or(-1));
    if data.get_current() != current {
        data.set_current(current);
    }
    let (audible, speed) = ctx.player().map_or((false, 1.0), |s| (s.audible, s.speed));
    match lrc::next_start(&lines, at) {
        Some(next) if live && audible && !SNAPSHOT.with(Cell::get) => {
            let weak = Rc::downgrade(ctx);
            TIMER.with(|t| {
                // The song's time to the next line, played at the speed.
                let wait = (next - at).div_f32(speed.max(0.5)) + LATE;
                t.start(TimerMode::SingleShot, wait, move || {
                    if let Some(ctx) = weak.upgrade() {
                        sync(&ctx);
                    }
                })
            });
        }
        _ => TIMER.with(Timer::stop),
    }
}

/// A click on line `i`: plays from its start.
fn seek_line(ctx: &Rc<Ctx>, i: i32) {
    let Some(ui) = ctx.window() else { return };
    let lines = STATE.with(|s| s.borrow().lines.clone());
    let Some(line) = usize::try_from(i).ok().and_then(|i| lines.get(i)) else {
        return;
    };
    let Some(duration) = ctx
        .player()
        .and_then(|s| s.current)
        .and_then(|t| t.duration)
        .filter(|d| !d.is_zero())
    else {
        return;
    };
    let to = (line.start + LATE).min(duration);
    // The bar's seek: it moves the clock at once and tells the controller.
    let fraction = to.as_secs_f64() / duration.as_secs_f64();
    ui.global::<Player>().invoke_seek(fraction as f32);
    sync(ctx);
}

/// The lookup for `id`, on screen, found nothing: the view says why, and if LRCLIB was down
/// it's asked again a minute later.
fn show_failure(ctx: &Rc<Ctx>, data: &LyricsData, id: &TrackId, failure: &Failure) {
    let unavailable = matches!(failure, Failure::Unavailable(_));
    data.set_unavailable(unavailable);
    data.set_state(LyricsState::Error);
    if unavailable {
        resync_later(ctx, id, Again::Any);
    }
}

/// What a snapshot shows on the tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotLyrics {
    Synced,
    Plain,
    None,
    Loading,
    /// LRCLIB failed: the song's plain lyrics instead, LRCLIB asked again a minute later.
    Outage,
    /// LRCLIB failed and the song has no lyrics of its own.
    Unavailable,
    /// The lookup failed otherwise.
    Failed,
}

/// `--snapshot`: the tab open on the current track with `kind`, no timer armed.
pub fn show_for_snapshot(ctx: &Rc<Ctx>, kind: SnapshotLyrics) {
    SNAPSHOT.with(|s| s.set(true));
    let Some(ui) = ctx.window() else { return };
    let Some(id) = ctx.player().and_then(|s| s.current).map(|t| t.id) else {
        return;
    };
    let lookup = |synced: bool| match ctx.catalog.fixtures() {
        Some(f) => fixture_for(&f, &id, synced),
        None => fixture(synced),
    };
    let found = match kind {
        SnapshotLyrics::Synced => lookup(true),
        SnapshotLyrics::Plain => lookup(false),
        SnapshotLyrics::Outage => lookup(false).map(|f| Found { keep: false, ..f }),
        SnapshotLyrics::None => Ok(Found {
            lyrics: None,
            credit: String::new(),
            keep: true,
        }),
        SnapshotLyrics::Loading => {
            STATE.with(|s| s.borrow_mut().cache.insert(id, Entry::Loading));
            ui.global::<LyricsData>().set_state(LyricsState::Loading);
            return;
        }
        SnapshotLyrics::Unavailable | SnapshotLyrics::Failed => {
            let failure = if kind == SnapshotLyrics::Unavailable {
                Failure::Unavailable("LRCLIB: status 503 (snapshot)".into())
            } else {
                Failure::Failed("offline (snapshot)".into())
            };
            RESYNC.with(Timer::stop);
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                s.cache.remove(&id);
                s.shown = Some(id.clone());
                s.lines = Rc::from([]);
                s.generation += 1;
                s.retry.failed(id.clone());
            });
            show_failure(ctx, &ui.global::<LyricsData>(), &id, &failure);
            return;
        }
    };
    match found {
        Ok(found) => {
            STATE.with(|s| {
                let mut s = s.borrow_mut();
                s.cache.insert(id, Entry::Ready(Rc::new(found)));
                s.shown = None;
                // A fixture lookup the tab may have started meanwhile doesn't replace it.
                s.generation += 1;
            });
            live_changed(ctx, true);
        }
        Err(e) => log!("{e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(s: &str) -> TrackId {
        TrackId(s.into())
    }

    #[test]
    fn a_failed_lookup_should_be_asked_again_once_per_open() {
        let mut retry = Retry::default();
        retry.failed(id("a"));
        // Still open: nothing asks again by itself.
        assert!(!retry.opened(Some(&id("b"))), "another track");
        retry.closed();
        assert!(retry.opened(Some(&id("a"))), "reopened on the failed track");
        // It failed again during this open: not again until the view closes and reopens.
        retry.failed(id("a"));
        assert!(!retry.opened(Some(&id("a"))));
        retry.closed();
        assert!(retry.opened(Some(&id("a"))));
    }

    #[test]
    fn a_found_or_none_found_lookup_should_never_be_asked_again() {
        let mut retry = Retry::default();
        retry.failed(id("a"));
        retry.succeeded(&id("a"));
        retry.closed();
        assert!(!retry.opened(Some(&id("a"))));
        assert!(!retry.try_again(Some(&id("a"))));
    }

    #[test]
    fn try_again_should_ask_for_the_failed_track_each_press() {
        let mut retry = Retry::default();
        assert!(!retry.try_again(Some(&id("a"))), "nothing failed");
        retry.failed(id("a"));
        assert!(!retry.try_again(Some(&id("b"))), "another track shows");
        assert!(retry.try_again(Some(&id("a"))));
        // Asked: until that fails too, another press asks nothing.
        assert!(!retry.try_again(Some(&id("a"))));
        retry.failed(id("a"));
        assert!(retry.try_again(Some(&id("a"))));
    }
}
