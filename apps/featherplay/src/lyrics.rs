//! The Lyrics tab (ui/pages/lyrics.slint), beside Up next and Related in Now playing and
//! in the Up next panel, and Now playing's full lyrics view.
//!
//! Requests: only while lyrics are on screen (the full view, the tab in either panel, or the
//! desktop lyrics' strip: `LyricsData.live-changed`, from `LyricsWatch`), for the current
//! track, and again when the next track starts while they stay, once that track has stayed a
//! second (`SKIP_WAIT`: skipping through songs asks nothing); never in the background; never
//! with Settings' "Show lyrics" off. With "Lyrics timed by the word" on, Lyrictified's lyrics
//! first (it gets the title and nothing else), then Unison's timed by the word (the video's ID
//! and nothing else); then LRCLIB's synced lyrics, when Settings allows it (it gets the title,
//! artist, album and length, and no cookies), all three asked at once; then the song's own timed lyrics
//! from YouTube Music, then LRCLIB's plain lyrics, then the song's own plain ones (`settle`).
//! Results are kept per track for the session (a bounded map); failures aren't, so the tab asks
//! again the next time it opens on that track. Nor is the song's plain text when LRCLIB failed
//! rather than missed: a passing error mustn't hide synced lyrics. Shown, that fallback says
//! synced lyrics are unavailable for now, and while the view stays open on it LRCLIB is asked
//! once more a minute later (`RESYNC_AFTER`); synced lyrics found then take its place.
//!
//! The lyrics menu (ui/lyrics_menu.slint) sets the text size (Settings' too) and moves the lines
//! half a second sooner or later for the song (`nudge`), kept per song on this PC with what
//! "Choose other lyrics" picked (`choose`), on top of any `[offset:]` the LRC carries.
//! Translation and romanization (`enrich`) are off by default. Their round buttons keep
//! their state in settings; while a main-window lyrics view is on screen, the displayed lines
//! are sent in batches to Google Translate and the answers are cached for the session.
//!
//! Timing: nothing ticks, but one thing. While sound plays and lyrics are on screen, one
//! single-shot timer waits for the next line's start, read from the bar's clock
//! (`bar::position`); a pause, a seek or any new player state aims it again. Paused, with no
//! lyrics on screen, minimized or hidden, no timer is armed; nor with plain lyrics or none.
//! The exception: while a line timed by the word is being sung, the words' clock
//! (`LyricsData.progress`) moves about every frame (`time_words`) and stops once the line has
//! been sung, so a word lights up as it's reached.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Duration;

use featherplay_core::model::TrackId;
use featherplay_core::tr;
use featherplay_lyrics::{Line, Lyrics, Part};
use slint::{ComponentHandle, ModelRc, SharedString, Timer, TimerMode, VecModel};

use crate::app::Ctx;
use crate::bar;
use crate::follow::colour::Lch;
use crate::pages::settings;
use crate::ui::{LyricPart, LyricWord, LyricWordLine, LyricsData, LyricsState, Player, Theme};

mod choose;
mod enrich;
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
    /// The timing of each line's words, one entry per line; empty unless the lyrics have it.
    words: Rc<[Vec<Part>]>,
    /// How much later (positive) the shown track's lines show, in milliseconds.
    nudge_ms: i32,
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
    /// Moves the words' clock about every frame while a line timed by the word is sung
    /// (`time_words`): the one thing here that ticks.
    static TICK: Timer = Timer::default();
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
    data.on_pick_size(move |i| settings::pick_lyrics_size(&c, i));
    let c = ctx.clone();
    data.on_move_timing(move |dir| move_timing(&c, dir));
    let c = ctx.clone();
    data.on_reset_timing(move || set_nudge(&c, 0));
    let c = ctx.clone();
    data.on_unlock_desktop(move || crate::desktop_lyrics::toggle_lock(&c));
    let c = ctx.clone();
    data.on_reset(move || settings::reset_lyrics(&c));
    enrich::init(ctx, &data);
    choose::init(ctx, &ui);
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
    data.set_ink_tint(inks.tint);
    crate::desktop_lyrics::inks_changed(ui);
}

/// A crossfade's turn to the cover of colour `seed` starts (`crate::turn`): the lyrics'
/// panels (the side tab's, Home's band off the field, the desktop lyrics' strip) start
/// towards its ground, easing over the turn (`Theme.t-turn`). Their lines' inks stay the last
/// cover's until the crossing (`set_cover`).
pub fn turn_grounds(ui: &crate::ui::AppWindow, seed: Option<Lch>) {
    let theme = ui.global::<Theme>();
    let fixed = (!theme.get_follow()).then(|| theme.get_preset_dark());
    let next = crate::follow::lyric_inks_for(seed, fixed);
    let data = ui.global::<LyricsData>();
    let ground = |mut ink: crate::ui::LyricInk, to: &crate::ui::LyricInk| {
        ink.ground = to.ground;
        ink
    };
    data.set_ink_dark(ground(data.get_ink_dark(), &next.dark));
    data.set_ink_light(ground(data.get_ink_light(), &next.light));
    data.set_ink_tint(ground(data.get_ink_tint(), &next.tint));
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
    data.set_word_timed(settings::lyrics_word_timed());
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
        TICK.with(Timer::stop);
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
        s.words = Rc::from([]);
    });
    if let Some(ui) = ctx.window() {
        let data = ui.global::<LyricsData>();
        if data.get_state() != LyricsState::Idle {
            data.set_state(LyricsState::Idle);
        }
        enrich::clear(&data);
    }
}

/// The LRCLIB setting changed: what was found under the old one is forgotten.
pub fn setting_changed(ctx: &Rc<Ctx>) {
    RESYNC.with(Timer::stop);
    if let Some(ui) = ctx.window() {
        ui.global::<LyricsData>()
            .set_lrclib(settings::lyrics_lrclib());
        ui.global::<LyricsData>()
            .set_word_timed(settings::lyrics_word_timed());
    }
    STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.cache.clear();
        s.shown = None;
        s.lines = Rc::from([]);
        s.words = Rc::from([]);
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
        let nudge_ms = id.as_ref().map_or(0, choose::nudge);
        STATE.with(|s| {
            let mut s = s.borrow_mut();
            s.shown = id;
            s.lines = Rc::from([]);
            s.words = Rc::from([]);
            s.nudge_ms = nudge_ms;
        });
        show_timing(&data, nudge_ms);
        enrich::clear(&data);
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
                        fill(ctx, &data, &track.id, &found);
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
fn fill(ctx: &Rc<Ctx>, data: &LyricsData, id: &TrackId, found: &Found) {
    let nudge_ms = STATE.with(|s| s.borrow().nudge_ms);
    let synced = matches!(found.lyrics, Some(Lyrics::Synced(_)));
    data.set_credit(credit_line(&found.credit, if synced { nudge_ms } else { 0 }).into());
    data.set_picked(found.picked);
    data.set_synced_unavailable(found.synced_unavailable());
    // Lines timed by the word are drawn word by word; any other lyrics start clean. The words
    // go with the lines they were made with (`Found::timed`): one entry each, or none at all.
    // A line that can't be drawn piece by piece (`drawn_by_the_word`) has none, and shows whole.
    let words: Vec<Vec<Part>> = match &found.lyrics {
        Some(Lyrics::Synced(lines)) if found.words.len() == lines.len() => found
            .words
            .iter()
            .map(|parts| {
                if drawn_by_the_word(parts) {
                    parts.clone()
                } else {
                    Vec::new()
                }
            })
            .collect(),
        _ => Vec::new(),
    };
    data.set_word_lines(word_model(&words));
    STATE.with(|s| s.borrow_mut().words = Rc::from(words));
    match &found.lyrics {
        Some(Lyrics::Synced(lines)) => {
            let source: Vec<String> = lines.iter().map(|line| line.text.clone()).collect();
            let texts: Vec<SharedString> = source.iter().map(|text| text.as_str().into()).collect();
            data.set_current(-1);
            data.set_lines(ModelRc::from(feed(texts)));
            STATE.with(|s| s.borrow_mut().lines = Rc::from(lines.as_slice()));
            data.set_state(LyricsState::Synced);
            enrich::lines_changed(ctx, data, id, source);
        }
        Some(Lyrics::Plain(text)) => {
            let source: Vec<String> = plain_lines(text)
                .into_iter()
                .map(|line| line.to_string())
                .collect();
            data.set_plain(text.as_str().into());
            data.set_plain_lines(ModelRc::from(feed(
                source.iter().map(|line| line.as_str().into()).collect(),
            )));
            data.set_state(LyricsState::Plain);
            enrich::lines_changed(ctx, data, id, source);
        }
        None => {
            enrich::lines_changed(ctx, data, id, Vec::new());
            data.set_state(LyricsState::None);
        }
    }
}

/// Milliseconds in the song, for the views (they count in `int`s).
fn ms(d: Duration) -> i32 {
    i32::try_from(d.as_millis()).unwrap_or(i32::MAX)
}

/// Whether a line may break between two pieces whose characters meet at `before` and `after`
/// with no space: scripts written without spaces (Japanese, Chinese) break between characters,
/// but never before a mark that belongs to the one it follows (punctuation, a long-vowel mark,
/// a small kana).
fn breaks_between(before: char, after: char) -> bool {
    fn cjk(c: char) -> bool {
        matches!(c,
            '\u{3040}'..='\u{30FF}' | '\u{3400}'..='\u{4DBF}' | '\u{4E00}'..='\u{9FFF}'
            | '\u{F900}'..='\u{FAFF}' | '\u{FF66}'..='\u{FF9F}')
    }
    const STICKY: &str =
        "、。，．・：；！？）］｝」』】〉》〕ー〜～…ぁぃぅぇぉっゃゅょゎァィゥェォッャュョヮ々";
    cjk(before) && cjk(after) && !STICKY.contains(after)
}

/// Characters a line can't be drawn piece by piece in: scripts written right to left
/// (Hebrew, Arabic, Syriac, Thaana, N'Ko), whose pieces would be laid out left to right, and
/// scripts whose letters join or reshape across a piece's edge or that break lines without
/// spaces by dictionary (the Brahmic scripts, Thai, Lao, Myanmar, Khmer, Mongolian).
fn needs_whole_line(c: char) -> bool {
    matches!(c,
        '\u{0590}'..='\u{08FF}' | '\u{0900}'..='\u{0DFF}' | '\u{0E00}'..='\u{0EFF}'
        | '\u{1000}'..='\u{109F}' | '\u{1780}'..='\u{18AF}' | '\u{FB1D}'..='\u{FDFF}'
        | '\u{FE70}'..='\u{FEFF}')
}

/// The most characters a word drawn piece by piece may have: its pieces never wrap inside it,
/// so a longer one could run past a narrow view (the side tab at Large).
const MAX_WORD_CHARS: usize = 16;

/// Whether a line's word timing is drawn: not for a script it can't be drawn in
/// (`needs_whole_line`), nor with a word too long to fit (`MAX_WORD_CHARS`). Such a line shows
/// whole, lit as a line timed as a whole is.
fn drawn_by_the_word(parts: &[Part]) -> bool {
    let mut word = 0;
    for p in parts {
        if p.text.chars().any(needs_whole_line) {
            return false;
        }
        word = if p.spaced { 0 } else { word } + p.text.chars().count();
        // Pieces in scripts without spaces break anywhere between them (`breaks_between`).
        if p.text.chars().all(|c| breaks_between(c, c)) {
            word = 0;
        }
        if word > MAX_WORD_CHARS {
            return false;
        }
    }
    true
}

/// The words of each line for the views: a line's pieces grouped into words (a piece with a
/// space before it, or one a line may break before, starts one), the words' pieces edge to edge,
/// each word knowing whether a space follows it. Built at once, not fed like the lines: it's plain data, and only the
/// lines made so far read their entry.
fn word_model(lines: &[Vec<Part>]) -> ModelRc<LyricWordLine> {
    let rows: Vec<LyricWordLine> = lines
        .iter()
        .map(|parts| {
            // (the word's pieces, whether a space follows it)
            let mut words: Vec<(Vec<LyricPart>, bool)> = Vec::new();
            let mut last: Option<char> = None;
            for p in parts {
                let piece = LyricPart {
                    text: p.text.as_str().into(),
                    start: ms(p.start),
                    end: ms(p.end),
                };
                let first = p.text.chars().next();
                let breaks =
                    p.spaced || matches!((last, first), (Some(a), Some(b)) if breaks_between(a, b));
                match words.last_mut() {
                    Some(word) if !breaks => word.0.push(piece),
                    Some(word) => {
                        word.1 = p.spaced;
                        words.push((vec![piece], false));
                    }
                    None => words.push((vec![piece], false)),
                }
                last = p.text.chars().last();
            }
            let words: Vec<LyricWord> = words
                .into_iter()
                .map(|(w, space)| LyricWord {
                    parts: ModelRc::from(Rc::new(VecModel::from(w))),
                    space,
                })
                .collect();
            LyricWordLine {
                words: ModelRc::from(Rc::new(VecModel::from(words))),
            }
        })
        .collect();
    ModelRc::from(Rc::new(VecModel::from(rows)))
}
/// How often the words' clock moves: about a frame at 60 Hz.
const TICK_EVERY: Duration = Duration::from_millis(16);

/// Where the song is in the lyrics' own time, in milliseconds: the lines' timing moved is
/// taken off, so words follow their lines.
fn progress_ms(at: Duration, nudge_ms: i32) -> i32 {
    ms(at).saturating_sub(nudge_ms)
}

/// Sets the words' clock, and keeps it running about every frame only while a line timed by
/// the word is being sung with sound playing and the view on screen: nothing ticks between
/// lines, paused, or for lyrics timed by the line. Called with the lines' timing
/// (`time_lines`), so a pause, a seek or a new line aims it again.
fn time_words(
    ctx: &Rc<Ctx>,
    data: &LyricsData,
    at: Duration,
    current: Option<usize>,
    live: bool,
    audible: bool,
) {
    let (words, nudge_ms) = STATE.with(|s| {
        let s = s.borrow();
        (s.words.clone(), s.nudge_ms)
    });
    if words.is_empty() {
        TICK.with(Timer::stop);
        return;
    }
    let progress = progress_ms(at, nudge_ms);
    if data.get_progress() != progress {
        data.set_progress(progress);
    }
    let sung_until = current
        .and_then(|i| words.get(i))
        .and_then(|parts| parts.last())
        .map(|p| ms(p.end));
    if live && audible && !SNAPSHOT.with(Cell::get) && sung_until.is_some_and(|end| progress < end)
    {
        let weak = Rc::downgrade(ctx);
        TICK.with(|t| {
            t.start(TimerMode::Repeated, TICK_EVERY, move || {
                if let Some(ctx) = weak.upgrade() {
                    tick(&ctx);
                }
            })
        });
    } else {
        TICK.with(Timer::stop);
    }
}

/// One step of the words' clock; it stops itself once the line playing has been sung.
fn tick(ctx: &Rc<Ctx>) {
    let (words, nudge_ms) = STATE.with(|s| {
        let s = s.borrow();
        (s.words.clone(), s.nudge_ms)
    });
    let (Some(at), Some(ui)) = (bar::position(), ctx.window()) else {
        TICK.with(Timer::stop);
        return;
    };
    let data = ui.global::<LyricsData>();
    let progress = progress_ms(at, nudge_ms);
    data.set_progress(progress);
    let sung = usize::try_from(data.get_current())
        .ok()
        .and_then(|i| words.get(i))
        .and_then(|parts| parts.last())
        .is_none_or(|p| progress >= ms(p.end));
    if sung {
        TICK.with(Timer::stop);
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
    FEEDING.with(|f| *f.borrow_mut() = Some(crate::busy::Busy::on_timers()));
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
    let (lines, live, nudge_ms) = STATE.with(|s| {
        let s = s.borrow();
        (s.lines.clone(), s.live, s.nudge_ms)
    });
    let Some(ui) = ctx.window() else { return };
    let (Some(at), false) = (bar::position(), lines.is_empty()) else {
        TIMER.with(Timer::stop);
        TICK.with(Timer::stop);
        return;
    };
    let data = ui.global::<LyricsData>();
    let (line, next) = nudged_line_at(&lines, at, nudge_ms);
    let current = line.map_or(-1, |i| i32::try_from(i).unwrap_or(-1));
    if data.get_current() != current {
        data.set_current(current);
    }
    let (audible, speed) = ctx.player().map_or((false, 1.0), |s| (s.audible, s.speed));
    time_words(ctx, &data, at, line, live, audible);
    match next {
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
    let (lines, nudge_ms) = STATE.with(|s| {
        let s = s.borrow();
        (s.lines.clone(), s.nudge_ms)
    });
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
    let to = (nudged(line.start, nudge_ms) + LATE).min(duration);
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

/// The most the lines move either way: past this, the lyrics are for another recording.
const MAX_NUDGE_MS: i32 = 10_000;
/// One press of − or +.
const NUDGE_STEP_MS: i32 = 500;

/// When a line starting at `start` in the song shows, moved `nudge_ms` later (sooner when
/// negative); never before the song starts.
fn nudged(start: Duration, nudge_ms: i32) -> Duration {
    let ms = i64::try_from(start.as_millis()).unwrap_or(i64::MAX) + i64::from(nudge_ms);
    Duration::from_millis(u64::try_from(ms).unwrap_or(0))
}

/// The line showing at `at` with the lines moved `nudge_ms`, and when the next one shows.
fn nudged_line_at(
    lines: &[Line],
    at: Duration,
    nudge_ms: i32,
) -> (Option<usize>, Option<Duration>) {
    let shown = lines.partition_point(|l| nudged(l.start, nudge_ms) <= at);
    let next = lines.get(shown).map(|l| nudged(l.start, nudge_ms));
    (shown.checked_sub(1), next)
}

/// The timing as the menu says it: "On time", "0.5 s later", "1 s sooner".
fn timing_label(nudge_ms: i32) -> String {
    let secs = seconds(nudge_ms);
    match nudge_ms.signum() {
        0 => tr!("On time").to_string(),
        1 => tr!("{} s later", secs),
        _ => tr!("{} s sooner", secs),
    }
}

/// Half seconds as "0.5", whole ones as "1".
fn seconds(nudge_ms: i32) -> String {
    let ms = nudge_ms.unsigned_abs();
    if ms.is_multiple_of(1000) {
        (ms / 1000).to_string()
    } else {
        format!("{}.{}", ms / 1000, ms % 1000 / 100)
    }
}

/// The credit under the lines, with the timing note once they were moved: "Lyrics from
/// LRCLIB · 0.5 s later for this song".
fn credit_line(credit: &str, nudge_ms: i32) -> String {
    let note = match nudge_ms.signum() {
        0 => return credit.to_string(),
        1 => tr!("{} s later for this song", seconds(nudge_ms)),
        _ => tr!("{} s sooner for this song", seconds(nudge_ms)),
    };
    if credit.is_empty() {
        note
    } else {
        format!("{credit} · {note}")
    }
}

/// The menu's timing line for `nudge_ms`.
fn show_timing(data: &LyricsData, nudge_ms: i32) {
    data.set_timing(timing_label(nudge_ms).into());
    data.set_timing_moved(nudge_ms != 0);
}

/// − (-1) or + (1) in the menu: the lines half a second sooner or later for this song.
fn move_timing(ctx: &Rc<Ctx>, dir: i32) {
    let now = STATE.with(|s| s.borrow().nudge_ms);
    let to = (now + dir.signum() * NUDGE_STEP_MS).clamp(-MAX_NUDGE_MS, MAX_NUDGE_MS);
    set_nudge(ctx, to);
}

/// Moves the shown song's lines to `nudge_ms`, kept for the song; the current line and the
/// credit follow at once.
fn set_nudge(ctx: &Rc<Ctx>, nudge_ms: i32) {
    let Some(ui) = ctx.window() else { return };
    let (shown, found) = STATE.with(|s| {
        let mut s = s.borrow_mut();
        s.nudge_ms = nudge_ms;
        let found = s.shown.as_ref().and_then(|id| match s.cache.get(id) {
            Some(Entry::Ready(f)) => Some(f.clone()),
            _ => None,
        });
        (s.shown.clone(), found)
    });
    let Some(id) = shown else { return };
    choose::set_nudge(&id, nudge_ms);
    let data = ui.global::<LyricsData>();
    show_timing(&data, nudge_ms);
    if let Some(found) = found {
        let synced = matches!(found.lyrics, Some(Lyrics::Synced(_)));
        data.set_credit(credit_line(&found.credit, if synced { nudge_ms } else { 0 }).into());
    }
    sync(ctx);
}

/// Whether any song has a timing or a pick kept (without reading them all).
pub fn songs_chosen() -> bool {
    choose::any()
}

/// "Reset lyrics settings": every song's timing and pick are forgotten, and what shows now
/// is looked up again as it would be without them.
pub fn forget_choices(ctx: &Rc<Ctx>) {
    choose::forget();
    setting_changed(ctx);
}

/// `--snapshot`: the full view's lyrics menu open, at the defaults, or (`moved`) set to
/// Large with the lines half a second later, as board A draws it. Nothing is saved.
pub fn menu_for_snapshot(ctx: &Rc<Ctx>, moved: bool) {
    SNAPSHOT.with(|s| s.set(true));
    let Some(ui) = ctx.window() else { return };
    if moved {
        ui.global::<LyricsData>().set_size(2);
        set_nudge(ctx, NUDGE_STEP_MS);
    }
    // The ⋯ is made as a frame is: one first, so it's there to open its menu.
    if let Err(e) = ui.window().take_snapshot() {
        log!("lyrics menu: snapshot: {e}");
    }
    let menu = ui.global::<crate::ui::LyricsMenu>();
    menu.set_open_asked(menu.get_open_asked() + 1);
}

/// `--snapshot`: "Choose other lyrics" open on the playing song, its matches listed.
pub fn choose_for_snapshot(ctx: &Rc<Ctx>) {
    SNAPSHOT.with(|s| s.set(true));
    choose::open_for_snapshot(ctx);
}

/// The credit a Unison lookup gives, for the snapshot of its lines.
fn unison_credit_for_snapshot() -> String {
    tr!("Lyrics from Unison, timed by {}", "someone")
}

/// Made-up word timing for `lines`: each line's words (the long ones in two syllables) share
/// the time up to the next line, at most five seconds.
fn snapshot_words(lines: &[Line]) -> Vec<Vec<Part>> {
    lines
        .iter()
        .enumerate()
        .map(|(i, line)| {
            let end = lines
                .get(i + 1)
                .map_or(line.start + Duration::from_secs(4), |next| next.start)
                .min(line.start + Duration::from_secs(5));
            let mut pieces: Vec<(String, bool)> = Vec::new();
            for word in line.text.split_whitespace() {
                let chars: Vec<char> = word.chars().collect();
                if chars.first().is_some_and(|c| *c > '\u{2E80}') {
                    // No spaces inside: a piece for each character.
                    for (i, c) in chars.iter().enumerate() {
                        pieces.push((c.to_string(), i == 0));
                    }
                } else if chars.len() > 6 {
                    let (a, b) = chars.split_at(chars.len() / 2);
                    pieces.push((a.iter().collect(), true));
                    pieces.push((b.iter().collect(), false));
                } else {
                    pieces.push((word.to_string(), true));
                }
            }
            let share = (end - line.start) / u32::try_from(pieces.len().max(1)).unwrap_or(1);
            pieces
                .into_iter()
                .zip(0u32..)
                .map(|((text, spaced), n)| Part {
                    start: line.start + share * n,
                    end: line.start + share * (n + 1),
                    text,
                    spaced: spaced && n > 0,
                })
                .collect()
        })
        .collect()
}

/// What a snapshot shows on the tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapshotLyrics {
    Synced,
    /// The same lines timed by the word (made up: even shares of each line's time).
    Words,
    /// Japanese sample lines timed by the character, as songs from Chinese platforms are.
    WordsJapanese,
    /// Synced Japanese sample lines with romanization and English translation under them.
    Enriched,
    /// The same lines just after Translate was turned on: the English on its way.
    Translating,
    Plain,
    None,
    /// The user chose no lyrics for the song ("None of these").
    ChoseNone,
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
        SnapshotLyrics::WordsJapanese => {
            let lines: Vec<Line> = enrich::SNAPSHOT_LINES
                .iter()
                .zip(0u32..)
                .map(|((text, _, _), n)| Line {
                    start: (bar::position().unwrap_or_default() + Duration::from_secs(4) * n)
                        .saturating_sub(Duration::from_secs(20)),
                    text: (*text).into(),
                })
                .collect();
            Ok(Found {
                words: snapshot_words(&lines),
                lyrics: Some(Lyrics::Synced(lines)),
                credit: unison_credit_for_snapshot(),
                keep: true,
                ..Found::default()
            })
        }
        SnapshotLyrics::Words => lookup(true).map(|f| Found {
            words: match &f.lyrics {
                Some(Lyrics::Synced(lines)) => snapshot_words(lines),
                _ => Vec::new(),
            },
            credit: unison_credit_for_snapshot(),
            ..f
        }),
        SnapshotLyrics::Enriched | SnapshotLyrics::Translating => Ok(Found {
            lyrics: Some(Lyrics::Synced(
                enrich::SNAPSHOT_LINES
                    .iter()
                    .zip(0u32..)
                    // Four seconds apart, the sixth one playing at the bar's time.
                    .map(|((text, _, _), n)| Line {
                        start: (bar::position().unwrap_or_default() + Duration::from_secs(4) * n)
                            .saturating_sub(Duration::from_secs(20)),
                        text: (*text).into(),
                    })
                    .collect(),
            )),
            credit: tr!("Lyrics from LRCLIB").into(),
            keep: true,
            ..Found::default()
        }),
        SnapshotLyrics::Plain => lookup(false),
        SnapshotLyrics::Outage => lookup(false).map(|f| Found { keep: false, ..f }),
        SnapshotLyrics::None => Ok(Found {
            keep: true,
            ..Found::default()
        }),
        SnapshotLyrics::ChoseNone => Ok(Found {
            keep: true,
            picked: true,
            ..Found::default()
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
                s.words = Rc::from([]);
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
            if matches!(kind, SnapshotLyrics::Enriched | SnapshotLyrics::Translating) {
                enrich::show_for_snapshot(
                    &ui.global::<LyricsData>(),
                    kind == SnapshotLyrics::Translating,
                );
            }
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
    fn nudged_lines_should_show_later_or_sooner_and_never_before_the_song() {
        let ms = Duration::from_millis;
        let line = |start: u64| Line {
            start: ms(start),
            text: "x".into(),
        };
        let lines = [line(1_000), line(4_000), line(9_000)];
        // On time, as `lrc` has it.
        assert_eq!(
            nudged_line_at(&lines, ms(4_200), 0),
            (Some(1), Some(ms(9_000)))
        );
        // Half a second later: the second line isn't up yet at 4.2 s.
        assert_eq!(
            nudged_line_at(&lines, ms(4_200), 500),
            (Some(0), Some(ms(4_500)))
        );
        // A second sooner: the first line shows from the start.
        assert_eq!(
            nudged_line_at(&lines, ms(0), -1_000),
            (Some(0), Some(ms(3_000)))
        );
        assert_eq!(nudged(ms(300), -500), ms(0));
    }

    #[test]
    fn the_timing_should_read_in_half_seconds_and_note_itself_in_the_credit() {
        assert_eq!(timing_label(0), "On time");
        assert_eq!(timing_label(500), "0.5 s later");
        assert_eq!(timing_label(-1_000), "1 s sooner");
        assert_eq!(timing_label(1_500), "1.5 s later");
        assert_eq!(credit_line("Lyrics from LRCLIB", 0), "Lyrics from LRCLIB");
        assert_eq!(
            credit_line("Lyrics from LRCLIB", 500),
            "Lyrics from LRCLIB · 0.5 s later for this song"
        );
        assert_eq!(credit_line("", -500), "0.5 s sooner for this song");
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
