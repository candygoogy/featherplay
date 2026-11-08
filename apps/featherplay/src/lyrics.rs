//! The Lyrics tab (ui/pages/lyrics.slint), beside Up next and Related in Now playing and
//! in the Up next panel, and Now playing's full lyrics view.
//!
//! Requests: only while lyrics are on screen (the full view, or the tab in either panel:
//! `LyricsData.live-changed`, from `LyricsWatch`), for the current track, and again when the
//! next track starts while they stay, once that track has stayed a second (`SKIP_WAIT`:
//! skipping through songs asks nothing); never in the background; never with Settings'
//! "Show lyrics" off. LRCLIB's synced lyrics first, when Settings allows it (it gets the
//! title, artist, album and length, and no cookies), then the song's own timed lyrics from
//! YouTube Music, then LRCLIB's plain lyrics, then the song's own plain ones (`settle`).
//! Results are kept per track for the session (a bounded map); failures aren't, so the tab
//! asks again the next time it opens on that track. Nor is the song's plain
//! text when LRCLIB failed rather than missed: a passing error mustn't hide synced lyrics.
//! Shown, that fallback says synced lyrics are unavailable for now, and while the view stays
//! open on it LRCLIB is asked once more a minute later (`RESYNC_AFTER`); synced lyrics
//! found then take its place.
//!
//! Timing: nothing ticks. While sound plays and lyrics are on screen, one single-shot timer
//! waits for the next line's start, read from the bar's clock (`bar::position`); a pause, a
//! seek or any new player state aims it again. Paused, with no lyrics on screen, minimized
//! or hidden, no timer is armed; nor with plain lyrics or none.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::OnceLock;
use std::time::Duration;

use featherplay_core::model::{Track, TrackId};
use featherplay_core::tr;
use featherplay_lyrics::lrclib::{self, Lookup, LrcLib, Query};
use featherplay_lyrics::{Line, Lyrics, lrc};
use featherplay_ytm::SongLyrics;
use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};

use crate::app::Ctx;
use crate::bar;
use crate::catalog::Source;
use crate::fixtures::Fixtures;
use crate::follow::colour::Lch;
use crate::pages::settings;
use crate::ui::{LyricsData, LyricsState, Player, Theme};

/// The credit under lyrics from LRCLIB.
fn lrclib_credit() -> &'static str {
    tr!("Lyrics from LRCLIB")
}
/// Tracks whose lyrics are kept; past this the map starts over.
const MAX_CACHED: usize = 64;
/// A line is shown this long after it starts, so a timer that fires a little early never
/// shows the line before it again. A click on a line seeks this far in, for the same reason.
const LATE: Duration = Duration::from_millis(30);
/// LRCLIB failed and the song's plain lyrics show instead: it's asked once more this long
/// after, while the view stays open on them. It answered 503 on and off on 2026-10-31.
const RESYNC_AFTER: Duration = Duration::from_secs(60);
/// A track change while a lyrics view shows waits this long before looking the new track up,
/// so skipping through songs costs no requests.
const SKIP_WAIT: Duration = Duration::from_secs(1);

/// What a lookup found for one track.
#[derive(Debug)]
pub struct Found {
    lyrics: Option<Lyrics>,
    /// Where they come from, shown under them ("" when unknown).
    credit: String,
    /// Kept for the session. Not when LRCLIB failed and these are the fallback: the tab
    /// asks again the next time it opens on the track.
    keep: bool,
}

impl Found {
    /// The song's plain lyrics, shown because LRCLIB failed rather than missed.
    fn synced_unavailable(&self) -> bool {
        !self.keep && matches!(self.lyrics, Some(Lyrics::Plain(_)))
    }
}

/// What LRCLIB is asked once more, a minute after it failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Again {
    /// Synced lyrics, to take the place of the song's plain ones: anything else leaves those.
    Synced,
    /// Any lyrics, for a song with none of its own; LRCLIB having none is an answer too.
    Any,
}

/// Asks LRCLIB once more. `None` when it fails again, or has nothing better than what shows.
async fn ask_lrclib_again(lrclib: &LrcLib, track: &Track, again: Again) -> Option<Found> {
    let title = &track.title;
    let lookup = match lrclib.find(&query(track)).await {
        Ok(lookup) => lookup,
        Err(e) => {
            log!("lyrics {title:?}: {e}, asked again");
            return None;
        }
    };
    log!("lyrics {title:?}: LRCLIB {lookup}, asked again");
    match (lookup, again) {
        (
            Lookup::Found {
                lyrics: lyrics @ Lyrics::Synced(_),
                ..
            },
            _,
        )
        | (Lookup::Found { lyrics, .. }, Again::Any) => Some(Found {
            lyrics: Some(lyrics),
            credit: lrclib_credit().into(),
            keep: true,
        }),
        (_, Again::Synced) => None,
        (_, Again::Any) => Some(Found {
            lyrics: None,
            credit: String::new(),
            keep: true,
        }),
    }
}

/// The re-ask on the runtime: the fixtures' synced record in `--fixtures`, never the network.
async fn ask_again_from(source: Source, track: Track, again: Again) -> Option<Found> {
    if let Source::Fixtures(f) = source {
        return fixture_for(&f, &track.id, true).ok();
    }
    ask_lrclib_again(&client()?, &track, again).await
}

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

/// Marks the line playing now and arms the timer for the next one, while sound plays and
/// the tab is on screen.
fn sync(ctx: &Rc<Ctx>) {
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

/// Looks `track` up `SKIP_WAIT` from now if it's still the one shown: another track change
/// meanwhile replaces the wait, so a run of skips looks up only the track that stays.
fn fetch_soon(ctx: &Rc<Ctx>, track: Track) {
    WAITING.with(|w| *w.borrow_mut() = Some(crate::busy::Busy::start()));
    let weak = Rc::downgrade(ctx);
    WAIT.with(|t| {
        t.start(TimerMode::SingleShot, SKIP_WAIT, move || {
            WAITING.with(|w| w.borrow_mut().take());
            let Some(ctx) = weak.upgrade() else { return };
            let due = STATE.with(|s| {
                let s = s.borrow();
                s.live && s.shown.as_ref() == Some(&track.id) && !s.cache.contains_key(&track.id)
            });
            if due {
                fetch(&ctx, track.clone());
            }
        });
    });
}

/// Stops a wait for a lookup; whether one was waiting.
fn stop_wait() -> bool {
    WAIT.with(Timer::stop);
    WAITING.with(|w| w.borrow_mut().take()).is_some()
}

/// LRCLIB failed for `id`, on screen (the song's plain lyrics show, or none at all): it's
/// asked once more `RESYNC_AFTER` from now, as long as the view stays open on the track
/// (closing it or another track stops the timer). Once: what the re-ask finds doesn't arm it
/// again.
fn resync_later(ctx: &Rc<Ctx>, id: &TrackId, again: Again) {
    if !STATE.with(|s| s.borrow().live) {
        return;
    }
    let weak = Rc::downgrade(ctx);
    let id = id.clone();
    RESYNC.with(|t| {
        t.start(TimerMode::SingleShot, RESYNC_AFTER, move || {
            if let Some(ctx) = weak.upgrade() {
                resync(&ctx, id.clone(), again);
            }
        });
    });
}

/// Asks LRCLIB again for `id`, still on screen, and shows what it found if that's better.
fn resync(ctx: &Rc<Ctx>, id: TrackId, again: Again) {
    let on_screen = settings::lyrics_shown()
        && STATE.with(|s| {
            let s = s.borrow();
            s.live && s.shown.as_ref() == Some(&id)
        });
    let Some(track) = ctx
        .player()
        .and_then(|s| s.current)
        .filter(|t| on_screen && t.id == id)
    else {
        return;
    };
    LOOKUPS.with(|n| n.set(n.get() + 1));
    let generation = STATE.with(|s| s.borrow().generation);
    ctx.load(
        move |source| ask_again_from(source, track, again),
        move |ctx, ui, found| {
            let Some(found) = found else { return };
            let found = Rc::new(found);
            let shown = STATE.with(|s| {
                let mut s = s.borrow_mut();
                if s.generation != generation {
                    return false;
                }
                s.retry.succeeded(&id);
                s.cache.insert(id.clone(), Entry::Ready(found.clone()));
                s.shown.as_ref() == Some(&id)
            });
            if shown {
                fill(&ui.global::<LyricsData>(), &found);
                sync(ctx);
            }
        },
    );
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

/// Looks up `track`'s lyrics on the runtime; the answer lands in the cache, and on the tab
/// if it still shows that track.
fn fetch(ctx: &Rc<Ctx>, track: Track) {
    if !settings::lyrics_shown() {
        return;
    }
    let use_lrclib = settings::lyrics_lrclib();
    LOOKUPS.with(|n| n.set(n.get() + 1));
    let id = track.id.clone();
    let generation = STATE.with(|s| {
        let mut s = s.borrow_mut();
        if s.cache.len() >= MAX_CACHED {
            let shown = s.shown.clone();
            s.cache.retain(|k, _| Some(k) == shown.as_ref());
        }
        s.cache.insert(id.clone(), Entry::Loading);
        s.generation
    });
    ctx.load(
        move |source| look_up(source, use_lrclib, track),
        move |ctx, ui, result| {
            let current = STATE.with(|s| s.borrow().generation == generation);
            if !current {
                return;
            }
            let found = result.map(Rc::new);
            if let Err(e) = &found {
                log!("lyrics for {}: {e}", id.0);
            }
            let shown = STATE.with(|s| {
                let mut s = s.borrow_mut();
                match &found {
                    Ok(f) if f.keep => {
                        s.retry.succeeded(&id);
                        s.cache.insert(id.clone(), Entry::Ready(f.clone()))
                    }
                    // Asked again the next time the view opens on this track, or on Try again.
                    _ => {
                        s.retry.failed(id.clone());
                        s.cache.remove(&id)
                    }
                };
                s.shown.as_ref() == Some(&id)
            });
            if shown {
                let data = ui.global::<LyricsData>();
                match found {
                    Ok(f) => {
                        fill(&data, &f);
                        if f.synced_unavailable() {
                            resync_later(ctx, &id, Again::Synced);
                        }
                    }
                    Err(failure) => show_failure(ctx, &data, &id, &failure),
                }
                sync(ctx);
            }
        },
    );
}

/// Why a lookup found nothing.
#[derive(Debug)]
enum Failure {
    /// LRCLIB failed and the song has no lyrics of its own: there may be some once LRCLIB
    /// answers again.
    Unavailable(String),
    /// The song's own lyrics couldn't be asked for.
    Failed(String),
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Failure::Unavailable(e) | Failure::Failed(e) => f.write_str(e),
        }
    }
}

/// LRCLIB when allowed, then the song's own lyrics (`settle` picks). An error only when
/// nothing was found and a lookup failed. Logs what each found, by the track's title.
async fn look_up(source: Source, use_lrclib: bool, track: Track) -> Result<Found, Failure> {
    if let Source::Fixtures(f) = source {
        return fixture_for(&f, &track.id, use_lrclib).map_err(Failure::Failed);
    }
    let title = &track.title;
    let mut lrclib = FromLrclib::Missed;
    if use_lrclib {
        match client() {
            Some(client) => match client.find(&query(&track)).await {
                Ok(lookup) => {
                    log!("lyrics {title:?}: LRCLIB {lookup}");
                    match lookup {
                        Lookup::Found {
                            lyrics: lyrics @ Lyrics::Synced(_),
                            ..
                        } => {
                            return Ok(Found {
                                lyrics: Some(lyrics),
                                credit: lrclib_credit().into(),
                                keep: true,
                            });
                        }
                        Lookup::Found {
                            lyrics: Lyrics::Plain(text),
                            ..
                        } => lrclib = FromLrclib::Plain(text),
                        _ => {}
                    }
                }
                Err(e) => {
                    log!("lyrics {title:?}: {e}");
                    lrclib = FromLrclib::Failed(e.to_string());
                }
            },
            None => lrclib = FromLrclib::Failed("no LRCLIB client".into()),
        }
    }
    let own = source.lyrics(&track.id).await.map_err(|e| e.to_string());
    match &own {
        Ok(Some(SongLyrics::Timed(t))) => {
            log!(
                "lyrics {title:?}: YouTube Music, {} timed lines",
                t.lines.len()
            );
        }
        Ok(Some(SongLyrics::Plain(_))) => log!("lyrics {title:?}: YouTube Music, plain"),
        Ok(None) => log!("lyrics {title:?}: YouTube Music, none"),
        Err(_) => {}
    }
    settle(lrclib, own)
}

/// What LRCLIB said, short of synced lyrics.
#[derive(Debug)]
enum FromLrclib {
    /// Nothing, or not asked (Settings).
    Missed,
    /// Plain lyrics only.
    Plain(String),
    /// It couldn't tell: an error or an outage.
    Failed(String),
}

/// What the tab gets once LRCLIB had no synced lyrics: the song's own timed lyrics, then
/// LRCLIB's plain ones, then the song's own plain ones, or none. The song's plain lyrics
/// after an LRCLIB failure aren't kept, so the tab asks LRCLIB again the next time it opens
/// on the track; timed ones are.
fn settle(lrclib: FromLrclib, own: Result<Option<SongLyrics>, String>) -> Result<Found, Failure> {
    let plain = match own {
        Ok(Some(SongLyrics::Timed(timed))) => {
            let lines = timed
                .lines
                .into_iter()
                .map(|l| Line {
                    start: l.start,
                    text: l.text,
                })
                .collect();
            return Ok(Found {
                lyrics: Some(Lyrics::Synced(lines)),
                credit: timed.source.unwrap_or_default(),
                keep: true,
            });
        }
        Ok(Some(SongLyrics::Plain(plain))) => Ok(Some(plain)),
        Ok(None) => Ok(None),
        Err(e) => Err(e),
    };
    let lrclib_failed = match lrclib {
        FromLrclib::Plain(text) => {
            return Ok(Found {
                lyrics: Some(Lyrics::Plain(text)),
                credit: lrclib_credit().into(),
                keep: true,
            });
        }
        FromLrclib::Missed => None,
        FromLrclib::Failed(e) => Some(e),
    };
    let keep = lrclib_failed.is_none();
    match plain {
        Ok(Some(plain)) => Ok(Found {
            lyrics: Some(Lyrics::Plain(plain.text)),
            credit: plain.source.unwrap_or_default(),
            keep,
        }),
        Ok(None) => match lrclib_failed {
            Some(e) => Err(Failure::Unavailable(e)),
            None => Ok(Found {
                lyrics: None,
                credit: String::new(),
                keep,
            }),
        },
        Err(e) => Err(Failure::Failed(match lrclib_failed {
            Some(f) => format!("{f}; {e}"),
            None => e,
        })),
    }
}

/// One LRCLIB client for the session.
fn client() -> Option<LrcLib> {
    static CLIENT: OnceLock<Option<LrcLib>> = OnceLock::new();
    CLIENT
        .get_or_init(|| match LrcLib::new() {
            Ok(c) => Some(c),
            Err(e) => {
                log!("lyrics: {e}");
                None
            }
        })
        .clone()
}

/// What LRCLIB is asked: the title, the first artist, the album and the length.
fn query(track: &Track) -> Query {
    Query {
        title: track.title.clone(),
        artist: track
            .artists
            .first()
            .map(|a| a.name.clone())
            .unwrap_or_default(),
        album: track.album.as_ref().map(|a| a.title.clone()),
        duration: track.duration,
    }
}

/// `--fixtures`: the recorded LRCLIB record (placeholder lines), never the network.
/// `FEATHERPLAY_FIXTURE_LYRICS` names another record in LRCLIB's format instead, for
/// measuring with lyrics as long as a real song's (`tests/lyrics_frames.rs`).
fn fixture(synced: bool) -> Result<Found, String> {
    const RECORDED: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../crates/lyrics/tests/fixtures/lrclib_get.json"
    );
    let path = std::env::var_os("FEATHERPLAY_FIXTURE_LYRICS")
        .map_or_else(|| RECORDED.into(), std::path::PathBuf::from);
    let text = std::fs::read_to_string(path).map_err(|e| format!("lyrics fixture: {e}"))?;
    let record = lrclib::parse_record(&text).map_err(|e| e.to_string())?;
    if synced {
        return Ok(Found {
            lyrics: record.lyrics(),
            credit: lrclib_credit().into(),
            keep: true,
        });
    }
    Ok(Found {
        lyrics: record.plain_lyrics.map(Lyrics::Plain),
        credit: tr!("Source: LyricFind").into(),
        keep: true,
    })
}

/// `--fixtures`: in the demo set, the song's own lines (or none), as LRCLIB would have them
/// (`synced`) or as the song's plain lyrics; otherwise the recorded record (`fixture`).
fn fixture_for(f: &Fixtures, id: &TrackId, synced: bool) -> Result<Found, String> {
    if !f.is_demo() {
        return fixture(synced);
    }
    let Some((lines, credit)) = f.demo_lyrics(id).map_err(|e| e.to_string())? else {
        return Ok(Found {
            lyrics: None,
            credit: String::new(),
            keep: true,
        });
    };
    let lyrics = if synced {
        Lyrics::Synced(lines)
    } else {
        let text: Vec<&str> = lines.iter().map(|l| l.text.as_str()).collect();
        Lyrics::Plain(text.join(
            "
",
        ))
    };
    Ok(Found {
        lyrics: Some(lyrics),
        credit,
        keep: true,
    })
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
    use featherplay_ytm::{PlainLyrics, TimedLine, TimedLyrics};

    use super::*;

    fn plain() -> Result<Option<SongLyrics>, String> {
        Ok(Some(SongLyrics::Plain(PlainLyrics {
            text: "Placeholder line".into(),
            source: Some("Source: LyricFind".into()),
        })))
    }

    fn timed() -> Result<Option<SongLyrics>, String> {
        let line = |ms, text: &str| TimedLine {
            start: Duration::from_millis(ms),
            text: text.into(),
        };
        Ok(Some(SongLyrics::Timed(TimedLyrics {
            lines: vec![
                line(250, "Placeholder line 1"),
                line(3980, "Placeholder line 2"),
            ],
            source: Some("Source: Musixmatch".into()),
        })))
    }

    #[test]
    fn the_demo_set_should_show_a_songs_own_timed_lines_with_their_credit() {
        let f = Fixtures::load_set(crate::fixtures::FixtureSet::Demo).expect("demo fixtures");
        let found = fixture_for(&f, &TrackId("demo-invisible-light".into()), true).expect("found");
        let Some(Lyrics::Synced(lines)) = &found.lyrics else {
            panic!("not synced: {found:?}");
        };
        assert!(lines.len() > 5);
        assert!(found.credit.contains("Josh Woodward"));
    }

    fn failed() -> FromLrclib {
        FromLrclib::Failed("status 503".into())
    }

    #[test]
    fn a_plain_fallback_after_an_lrclib_failure_should_be_shown_but_not_kept() {
        let found = settle(failed(), plain()).expect("plain lyrics");
        assert!(matches!(found.lyrics, Some(Lyrics::Plain(_))));
        assert!(!found.keep, "asked again the next time the tab opens");
    }

    #[test]
    fn a_plain_fallback_after_an_lrclib_miss_should_be_kept() {
        let found = settle(FromLrclib::Missed, plain()).expect("plain lyrics");
        assert!(found.keep);
    }

    #[test]
    fn only_a_plain_fallback_after_an_lrclib_failure_says_synced_lyrics_are_unavailable() {
        let failed = settle(failed(), plain()).expect("plain lyrics");
        assert!(failed.synced_unavailable());
        let missed = settle(FromLrclib::Missed, plain()).expect("plain lyrics");
        assert!(!missed.synced_unavailable(), "LRCLIB answered: not synced");
        let synced = fixture(true).expect("fixture");
        assert!(!synced.synced_unavailable());
    }

    #[test]
    fn an_lrclib_failure_should_fall_back_to_youtube_musics_timed_lyrics_without_the_note() {
        let found = settle(failed(), timed()).expect("timed lyrics");
        let Some(Lyrics::Synced(lines)) = &found.lyrics else {
            panic!("not synced: {found:?}");
        };
        assert_eq!(lines[1].start, Duration::from_millis(3980));
        assert_eq!(lines[1].text, "Placeholder line 2");
        assert_eq!(found.credit, "Source: Musixmatch");
        assert!(found.keep, "kept: LRCLIB coming back has nothing better");
        assert!(
            !found.synced_unavailable(),
            "no outage note over synced lines"
        );
    }

    #[test]
    fn youtube_musics_timed_lyrics_should_come_before_lrclibs_plain_ones() {
        let lrclib_plain = || FromLrclib::Plain("LRCLIB's words".into());
        let found = settle(lrclib_plain(), timed()).expect("timed lyrics");
        assert!(matches!(found.lyrics, Some(Lyrics::Synced(_))));
        // No timed lines: LRCLIB's plain lyrics, as before, even if the song's own failed.
        for own in [plain(), Ok(None), Err("offline".into())] {
            let found = settle(lrclib_plain(), own).expect("LRCLIB's plain lyrics");
            assert_eq!(found.lyrics, Some(Lyrics::Plain("LRCLIB's words".into())));
            assert_eq!(found.credit, lrclib_credit());
            assert!(found.keep);
        }
    }

    /// The track of the recorded LRCLIB record.
    fn track() -> Track {
        use featherplay_core::model::{AlbumId, AlbumRef, ArtistRef, LikeStatus, VideoType};
        Track {
            id: TrackId("t".into()),
            title: "Get Lucky".into(),
            artists: vec![ArtistRef {
                id: None,
                name: "Daft Punk".into(),
            }],
            album: Some(AlbumRef {
                id: AlbumId("a".into()),
                title: "Random Access Memories".into(),
            }),
            duration: Some(Duration::from_secs(367)),
            video_type: VideoType::Song,
            thumbnail: None,
            like: LikeStatus::Indifferent,
            counterpart: None,
        }
    }

    /// A stand-in for lrclib.net on a local port: `get` answers `status` with `body`, `search`
    /// with an empty list. Serves `requests` requests, then stops.
    fn stand_in(status: u16, body: &'static str, requests: usize) -> String {
        use std::io::{BufRead, BufReader, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a local port");
        let base = format!("http://{}/api/", listener.local_addr().expect("address"));
        std::thread::spawn(move || {
            for stream in listener.incoming().take(requests) {
                let Ok(mut stream) = stream else { continue };
                let mut reader = BufReader::new(stream.try_clone().expect("clone"));
                let mut line = String::new();
                reader.read_line(&mut line).expect("request line");
                let mut header = String::new();
                while reader.read_line(&mut header).is_ok_and(|n| n > 2) {
                    header.clear();
                }
                let (status, body) = if line.starts_with("GET /api/get") {
                    (status, body)
                } else {
                    (200, "[]")
                };
                let reply = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(reply.as_bytes());
            }
        });
        base
    }

    const RECORD: &str = include_str!("../../../crates/lyrics/tests/fixtures/lrclib_get.json");

    #[tokio::test]
    async fn asking_lrclib_again_should_swap_in_synced_lyrics_once_it_answers() {
        let lrclib = LrcLib::with_base(&stand_in(200, RECORD, 1)).expect("client");
        let found = ask_lrclib_again(&lrclib, &track(), Again::Synced)
            .await
            .expect("synced");
        assert!(matches!(found.lyrics, Some(Lyrics::Synced(_))));
        assert!(found.keep && !found.synced_unavailable());
    }

    #[tokio::test]
    async fn asking_lrclib_again_should_change_nothing_while_it_still_fails() {
        let lrclib = LrcLib::with_base(&stand_in(503, "{}", 2)).expect("client");
        assert!(
            ask_lrclib_again(&lrclib, &track(), Again::Synced)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn asking_lrclib_again_should_keep_the_fallback_when_it_has_only_plain_lyrics() {
        let plain_only = Box::leak(
            RECORD
                .replace("\"syncedLyrics\"", "\"unusedSyncedLyrics\"")
                .into_boxed_str(),
        );
        let lrclib = LrcLib::with_base(&stand_in(200, plain_only, 2)).expect("client");
        assert!(
            ask_lrclib_again(&lrclib, &track(), Again::Synced)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn asking_lrclib_again_for_a_song_without_lyrics_should_take_plain_ones_too() {
        let plain_only = Box::leak(
            RECORD
                .replace("\"syncedLyrics\"", "\"unusedSyncedLyrics\"")
                .into_boxed_str(),
        );
        let lrclib = LrcLib::with_base(&stand_in(200, plain_only, 2)).expect("client");
        let found = ask_lrclib_again(&lrclib, &track(), Again::Any)
            .await
            .expect("an answer");
        assert!(matches!(found.lyrics, Some(Lyrics::Plain(_))));
    }

    #[tokio::test]
    async fn asking_lrclib_again_for_a_song_without_lyrics_should_take_none_as_an_answer() {
        let lrclib = LrcLib::with_base(&stand_in(404, "{}", 2)).expect("client");
        let found = ask_lrclib_again(&lrclib, &track(), Again::Any)
            .await
            .expect("an answer");
        assert!(found.lyrics.is_none() && found.keep);
    }

    #[tokio::test]
    async fn asking_lrclib_again_for_a_song_without_lyrics_should_change_nothing_while_it_fails() {
        let lrclib = LrcLib::with_base(&stand_in(503, "{}", 2)).expect("client");
        assert!(
            ask_lrclib_again(&lrclib, &track(), Again::Any)
                .await
                .is_none()
        );
    }

    #[test]
    fn nothing_found_after_an_lrclib_failure_should_be_an_error() {
        assert!(settle(FromLrclib::Failed("timed out".into()), Ok(None)).is_err());
        assert!(settle(FromLrclib::Missed, Ok(None)).is_ok_and(|f| f.lyrics.is_none() && f.keep));
    }

    #[test]
    fn lrclib_down_with_no_lyrics_of_the_songs_own_should_be_unavailable() {
        let failure = settle(failed(), Ok(None)).expect_err("a failure");
        assert!(matches!(failure, Failure::Unavailable(_)), "{failure:?}");
    }

    #[test]
    fn the_songs_own_lyrics_failing_should_be_a_failed_lookup() {
        for lrclib in [FromLrclib::Missed, failed()] {
            let failure = settle(lrclib, Err("offline".into())).expect_err("a failure");
            assert!(matches!(failure, Failure::Failed(_)), "{failure:?}");
        }
    }

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
