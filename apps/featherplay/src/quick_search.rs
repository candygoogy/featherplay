//! Quick search from anywhere (`ui/quick_search.slint`): a global hotkey (Ctrl+Alt+K by
//! default; Settings › Window, off by default) opens a small borderless box over whatever
//! app is in front, on that app's monitor, centred, its top a fifth of the way down. Typing
//! gets the top bar's suggestions (the same request and pacing, a second `Suggester`), results
//! first; Enter plays, Shift+Enter plays next (the box stays open, picks keep their order),
//! Ctrl+Enter opens the item's page in featherplay. Empty, it lists the last five picks, kept
//! in `{cache}/quick-search.json`: nothing is asked for when it opens.
//!
//! The box closes on Esc, Enter, the shortcut again (it toggles), and a click elsewhere; the
//! first three give the focus back to the window that had it. With featherplay's own window
//! in front, the shortcut does what Ctrl+K does there instead (`search_in_main`). Where the
//! desktop lets no app hear keys pressed in another (Wayland), `featherplay --quick-search`
//! asks the running featherplay to open it over the single-instance channel.
//!
//! The window exists only while it shows, as the mini player's: `open` makes it (a few
//! milliseconds) and `close` drops it with its timer and its `Suggester`, so a closed box
//! costs nothing but the registered key.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Instant;

use featherplay_controller::suggest::{INTERVAL, Next, PAUSE, Suggester};
use featherplay_core::model::{AlbumId, ArtistId, PlaylistId, Track, TrackId, VideoType};
use featherplay_core::tr;
use featherplay_integrations::{KeyError, QuickKey};
use featherplay_platform::window::{self, Handle};
use featherplay_store::Paths;
use featherplay_ytm::{HomeItem, SearchSuggestions};
use serde::{Deserialize, Serialize};
use slint::winit_030::{EventResult, WinitWindowAccessor, winit};
use slint::{ComponentHandle, Model, ModelRc, PhysicalPosition, VecModel};

use crate::app::{self, Ctx};
use crate::controller::{Action, QueueAction};
use crate::pages::content;
use crate::router::Page;
use crate::ui::{
    AppWindow, Player, QuickData, QuickPick, QuickRow, QuickSearch, QuickState, SettingsData,
    SuggestionKind, Theme, TopBarData,
};
use crate::ui_bridge::{artist_names, shown};
use crate::{link, popup, shell, song_menu};

/// The box's width, in logical pixels.
pub const WIDTH: f32 = 640.0;
/// A result's cover, square, in logical pixels.
const COVER_PX: u32 = 40;
/// At most this many results, then this many completions.
const MAX_RESULTS: usize = 5;
const MAX_COMPLETIONS: usize = 3;
/// The picks kept for the empty box.
const RECENT: usize = 5;

/// A pick kept for the empty box: what it is (with what playing or opening it needs) and its
/// line.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Recent {
    pub item: HomeItem,
    pub subtitle: String,
}

struct QuickBox {
    ui: QuickSearch,
    suggester: RefCell<Suggester>,
    /// The suggester's pause or interval: a single shot, so nothing runs once it's fired.
    timer: slint::Timer,
    /// The answer the rows show (results trimmed to `MAX_RESULTS`, completions to
    /// `MAX_COMPLETIONS`), or the recent picks while the field is empty.
    shown: RefCell<Shown>,
    /// The window in front when it opened, to give the focus back to.
    previous: Option<Handle>,
    /// What Shift+Enter in this box set to play next last: the next one goes after it.
    last_next: RefCell<Option<TrackId>>,
    /// The last request failed: nothing can play without a connection, so the recent picks
    /// hide too, until an answer comes.
    offline: Cell<bool>,
}

#[derive(Default)]
enum Shown {
    #[default]
    Nothing,
    Recent(Vec<Recent>),
    Answer(SearchSuggestions),
    /// An answer without rows for this text: one row searches featherplay for it.
    None(String),
}

/// Registers a key for `set_key`: the system's (`QuickKey`), or a test's stand-in, which
/// registers nothing.
type Register = fn(&str) -> Result<Option<QuickKey>, KeyError>;

thread_local! {
    static BOX: RefCell<Option<Rc<QuickBox>>> = const { RefCell::new(None) };
    /// The registered key, while the setting is on, and whether one is (a stand-in's too).
    static KEY: RefCell<Option<QuickKey>> = const { RefCell::new(None) };
    static REGISTERED: Cell<bool> = const { Cell::new(false) };
    static REGISTER: Cell<Register> = const { Cell::new(register_system_key) };
    /// Why the key last couldn't be registered, for Settings' row.
    static KEY_ERROR: RefCell<Option<KeyError>> = const { RefCell::new(None) };
    /// Where the recent picks are kept: `{cache}/quick-search.json` (a test's own folder).
    static RECENT_FILE: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
    /// A snapshot's recent picks, read instead of the file.
    static RECENT_SHOWN: RefCell<Option<Vec<Recent>>> = const { RefCell::new(None) };
}

fn register_system_key(text: &str) -> Result<Option<QuickKey>, KeyError> {
    QuickKey::register(text, || {
        // Run on the event loop's next turn, outside the hotkey window's procedure.
        let _ = slint::invoke_from_event_loop(pressed);
    })
    .map(Some)
}

/// Has `set_key` register with `register` from now on: tests and snapshots never take a key
/// from the system.
pub fn register_keys_with(register: Register) {
    REGISTER.set(register);
}

/// Registers `key` as the quick search's hotkey in place of any before; `None` lets go of
/// it. Its presses open or close the box (`pressed`).
pub fn set_key(key: Option<&str>) -> Result<(), KeyError> {
    // Let go first: the new key may be the old one.
    KEY.with_borrow_mut(Option::take);
    REGISTERED.set(false);
    KEY_ERROR.set(None);
    let Some(text) = key else {
        return Ok(());
    };
    match REGISTER.get()(text) {
        Ok(k) => {
            log!("quick search: {text} registered");
            KEY.set(k);
            REGISTERED.set(true);
            Ok(())
        }
        Err(e) => {
            log!("quick search: {text}: {e:?}");
            KEY_ERROR.set(Some(e.clone()));
            Err(e)
        }
    }
}

/// The key to register for the saved `stored`: itself when it's a key with a modifier,
/// else (hand-edited, from a newer featherplay, empty) the default.
pub fn usable_key(stored: &str) -> &str {
    if featherplay_integrations::parse_hotkey(stored).is_ok() {
        stored
    } else {
        featherplay_store::Settings::QUICK_SEARCH_KEY
    }
}

/// Whether the quick search's key is registered now.
pub fn key_registered() -> bool {
    REGISTERED.get()
}

/// Why the key couldn't be registered the last time it was tried, if it couldn't.
pub fn key_error() -> Option<KeyError> {
    KEY_ERROR.with_borrow(Clone::clone)
}

/// Whether the box shows.
pub fn is_open() -> bool {
    BOX.with_borrow(Option::is_some)
}

/// The quick search's key was pressed (or `--quick-search` asked): with featherplay's own
/// window in front, its search takes the keys as Ctrl+K gives them; else the box opens, or
/// closes if it shows.
pub fn pressed() {
    if is_open() {
        close(true);
        return;
    }
    let Some(ctx) = Ctx::get() else { return };
    if main_in_front(&ctx) {
        search_in_main(&ctx);
        return;
    }
    if let Err(e) = open(&ctx) {
        log!("quick search: {e}");
        close(false);
    }
}

/// Whether featherplay's main window is on screen and has the keys.
fn main_in_front(ctx: &Ctx) -> bool {
    ctx.window().is_some_and(|ui| {
        let w = ui.window();
        w.is_visible()
            && !w.is_minimized()
            && w.with_winit_window(|w| w.has_focus()).unwrap_or(false)
    })
}

/// What Ctrl+K does in the main window: Now playing goes back first (it hides the bar), and
/// the top bar's field takes the keys with its text selected.
pub fn search_in_main(ctx: &Ctx) {
    use slint::platform::{Key, WindowEvent};
    let Some(ui) = ctx.window() else { return };
    let w = ui.window();
    let key = |text: slint::SharedString, down: bool| {
        w.dispatch_event(if down {
            WindowEvent::KeyPressed { text }
        } else {
            WindowEvent::KeyReleased { text }
        });
    };
    key(Key::Control.into(), true);
    key("k".into(), true);
    key("k".into(), false);
    key(Key::Control.into(), false);
}

/// Makes the box with the main window's theme, its field empty and the recent picks under
/// it, without showing it: `open` places and shows it, the snapshots and tests take it as it
/// is.
pub fn create(ctx: &Rc<Ctx>) -> Result<QuickSearch, String> {
    let main = ctx.window().ok_or("no main window")?;
    let ui = popup::creating(|| {
        QuickSearch::new().inspect(|ui| {
            ui.global::<QuickData>().set_framed(!popup::placed());
        })
    })
    .map_err(|e| format!("window: {e}"))?;
    sync_theme(&ui, &main);
    let quick = Rc::new(QuickBox {
        ui: ui.clone_strong(),
        suggester: RefCell::new(Suggester::default()),
        timer: slint::Timer::default(),
        shown: RefCell::new(Shown::Nothing),
        previous: window::foreground().map(|f| f.window),
        last_next: RefCell::new(None),
        offline: Cell::new(false),
    });
    BOX.with_borrow_mut(|b| *b = Some(quick.clone()));
    wire(ctx, &ui);
    sync_playing(&ui, &main);
    show_recent(ctx, &quick);
    ui.window()
        .set_size(slint::LogicalSize::new(WIDTH, ui.get_box_height()));
    ui.invoke_focus_input();
    Ok(ui)
}

/// Opens the box on the monitor of the window in front (else the pointer's), centred, its
/// top a fifth of the way down; on Wayland, where the desktop puts it.
fn open(ctx: &Rc<Ctx>) -> Result<(), String> {
    let started = Instant::now();
    let ui = create(ctx)?;
    let border = popup::border(&ui.global::<Theme>());
    let placed = popup::placed()
        .then(window::active_point)
        .flatten()
        .and_then(|(x, y)| window::work_area_at(x, y));
    match placed {
        Some((work, scale)) => {
            let height = ui.get_box_height();
            let (mut left, top) = place(work, scale, WIDTH);
            ui.window().set_position(PhysicalPosition::new(left, top));
            popup::creating(|| ui.window().show()).map_err(|e| format!("show: {e}"))?;
            // Made at another monitor's scale: centred by its own width.
            let real = ui.window().size().width as i32;
            let wanted = (WIDTH * scale).round() as i32;
            if real > 0 && real != wanted {
                left += (wanted - real) / 2;
                ui.window().set_position(PhysicalPosition::new(left, top));
            }
            log!(
                "quick search opened at {left},{top} ({WIDTH}x{height}, {:.1} ms)",
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
        None => {
            popup::show_unplaced(ui.window())?;
            log!(
                "quick search opened where the desktop put it ({:.1} ms)",
                started.elapsed().as_secs_f64() * 1000.0
            );
        }
    }
    shell::when_handle(
        &ui,
        move |ui, handle| {
            popup::dress(handle, border);
            if cfg!(not(windows)) {
                ui.window().with_winit_window(|w| w.focus_window());
            }
        },
        |_| log!("quick search: its window never came"),
    );
    Ok(())
}

/// Where the box's top-left corner goes on a monitor whose work area is `work` (physical
/// pixels) at `scale`: centred, its top a fifth of the way down.
pub fn place(work: window::ScreenRect, scale: f32, width: f32) -> (i32, i32) {
    let (left, top, right, bottom) = work;
    let w = (width * scale).round() as i32;
    (left + (right - left - w) / 2, top + (bottom - top) / 5)
}

/// Closes and drops the box, if it shows; `give_back` hands the focus back to the window
/// that had it when the box opened (Esc, Enter, the shortcut again). Never from inside the
/// box's own callbacks: they go through `close_later`.
pub fn close(give_back: bool) {
    let Some(quick) = BOX.with_borrow_mut(Option::take) else {
        return;
    };
    quick.timer.stop();
    // First, while featherplay is still in front and may hand the foreground on.
    if give_back && let Some(previous) = quick.previous {
        window::bring_forward(previous);
    }
    let _ = quick.ui.hide();
}

/// `close`, on the event loop's next turn: the box's own callbacks can't drop it.
fn close_later(give_back: bool) {
    let _ = slint::invoke_from_event_loop(move || close(give_back));
}

/// The theme flipped or the cover's colours moved: the open box follows.
pub fn theme_changed(ctx: &Ctx) {
    let Some(quick) = BOX.with_borrow(Clone::clone) else {
        return;
    };
    if let Some(main) = ctx.window() {
        sync_theme(&quick.ui, &main);
    }
}

/// The song changed or stopped: the open box's footer follows.
pub fn update(ctx: &Ctx) {
    let Some(quick) = BOX.with_borrow(Clone::clone) else {
        return;
    };
    if let Some(main) = ctx.window() {
        sync_playing(&quick.ui, &main);
    }
}

/// The footer's song playing now: the main window's, none without a track.
fn sync_playing(ui: &QuickSearch, main: &AppWindow) {
    let player = main.global::<Player>();
    let title = if player.get_has_track() {
        player.get_title()
    } else {
        Default::default()
    };
    ui.global::<QuickData>().set_playing(title);
}

fn sync_theme(ui: &QuickSearch, main: &AppWindow) {
    popup::sync_theme(
        &main.global::<Theme>(),
        &ui.global::<Theme>(),
        main.global::<SettingsData>().get_appearance(),
    );
}

fn wire(ctx: &Rc<Ctx>, ui: &QuickSearch) {
    let data = ui.global::<QuickData>();
    // The window follows the box's height, its top edge where it is.
    let weak = ui.as_weak();
    data.on_height_changed(move |height| {
        if let Some(ui) = weak.upgrade() {
            ui.window().set_size(slint::LogicalSize::new(WIDTH, height));
        }
    });
    let c = ctx.clone();
    data.on_edited(move |text| edited(&c, &text));
    let c = ctx.clone();
    data.on_pick(move |i, how| pick(&c, i.max(0) as usize, how));
    data.on_dismiss(|| close_later(true));
    // Another window took the activation (a click elsewhere, Alt+Tab): it closes, the focus
    // left where it went. Only a focus it had can be lost (X11 reports a new window
    // unfocused first).
    let had_focus = Cell::new(false);
    ui.window().on_winit_window_event(move |_, ev| {
        match ev {
            winit::event::WindowEvent::Focused(true) => had_focus.set(true),
            winit::event::WindowEvent::Focused(false) if had_focus.get() && is_open() => {
                log!("quick search closes: another window took the activation");
                close_later(false);
            }
            _ => {}
        }
        EventResult::Propagate
    });
}

fn edited(ctx: &Rc<Ctx>, text: &str) {
    let Some(quick) = BOX.with_borrow(Clone::clone) else {
        return;
    };
    let next = quick.suggester.borrow_mut().edited(text);
    act(ctx, &quick, next);
}

fn act(ctx: &Rc<Ctx>, quick: &Rc<QuickBox>, next: Next) {
    match next {
        Next::Hide => {
            quick.timer.stop();
            show_recent(ctx, quick);
        }
        Next::Show(s) => show_answer(ctx, quick, s),
        Next::Fetch { ask, query } => {
            waiting(quick);
            start_timer(ctx, quick, INTERVAL);
            fetch(ctx, ask, query);
        }
        Next::Pause => {
            waiting(quick);
            start_timer(ctx, quick, PAUSE);
        }
        Next::Wait => {}
    }
}

/// Typed, and the answer for the text isn't in yet: the rows (or the no-results line) for an
/// older text stay; the recent picks (an empty field's) go.
fn waiting(quick: &QuickBox) {
    if matches!(*quick.shown.borrow(), Shown::Answer(_) | Shown::None(_)) {
        return;
    }
    quick.shown.replace(Shown::Nothing);
    let data = quick.ui.global::<QuickData>();
    data.set_rows(ModelRc::default());
    data.set_completions_from(-1);
    data.set_highlighted(-1);
    data.set_state(QuickState::Waiting);
}

fn start_timer(ctx: &Rc<Ctx>, quick: &QuickBox, after: std::time::Duration) {
    let c = ctx.clone();
    quick
        .timer
        .start(slint::TimerMode::SingleShot, after, move || {
            let Some(quick) = BOX.with_borrow(Clone::clone) else {
                return;
            };
            let next = quick.suggester.borrow_mut().due();
            act(&c, &quick, next);
        });
}

/// Edit `ask`'s request for `query`; its answer shows if it's still the freshest and the box
/// is still open.
fn fetch(ctx: &Rc<Ctx>, ask: u64, query: String) {
    let asked = query.clone();
    ctx.load(
        move |source| async move { source.search_suggestions(&asked).await },
        move |ctx, _, result| {
            let Some(quick) = BOX.with_borrow(Clone::clone) else {
                return;
            };
            let field = quick.ui.global::<QuickData>().get_query();
            let current = field.trim().starts_with(query.as_str());
            match result {
                Ok(answer) => {
                    quick.offline.set(false);
                    let fresh = quick.suggester.borrow_mut().answered(ask, &query, answer);
                    if let Some(answer) = fresh.filter(|_| current) {
                        show_answer(ctx, &quick, answer);
                    }
                }
                Err(e) => {
                    log!("quick search: {e}");
                    quick.offline.set(true);
                    if current {
                        offline(&quick);
                    }
                }
            }
        },
    );
}

/// The empty field: the recent picks, or the hint line before the first.
fn show_recent(ctx: &Ctx, quick: &QuickBox) {
    if quick.offline.get() {
        offline(quick);
        return;
    }
    let picks = load_recent();
    let data = quick.ui.global::<QuickData>();
    data.set_completions_from(-1);
    data.set_highlighted(-1);
    if picks.is_empty() {
        data.set_rows(ModelRc::default());
        data.set_state(QuickState::Fresh);
        quick.shown.replace(Shown::Nothing);
        return;
    }
    let (rows, covers): (Vec<QuickRow>, Vec<Option<String>>) = picks
        .iter()
        .map(|r| (result_row(&r.item, &r.subtitle), cover_of(&r.item)))
        .unzip();
    set_rows(ctx, quick, rows, covers);
    data.set_state(QuickState::Recent);
    quick.shown.replace(Shown::Recent(picks));
}

fn show_answer(ctx: &Ctx, quick: &QuickBox, mut s: SearchSuggestions) {
    s.results.truncate(MAX_RESULTS);
    s.queries.truncate(MAX_COMPLETIONS);
    let data = quick.ui.global::<QuickData>();
    let query = data.get_query().trim().to_string();
    if s.results.is_empty() && s.queries.is_empty() {
        // Enter still does something: it searches featherplay for the text.
        data.set_rows(ModelRc::new(VecModel::from(vec![search_for_row(&query)])));
        data.set_none_title(tr!("No results for “{}”", query).into());
        data.set_completions_from(0);
        data.set_highlighted(0);
        data.set_state(QuickState::None);
        quick.shown.replace(Shown::None(query));
        return;
    }
    let (rows, covers) = rows(&s);
    let had_rows = data.get_state() == QuickState::Results;
    set_rows(ctx, quick, rows, covers);
    data.set_completions_from(if s.queries.is_empty() {
        -1
    } else {
        s.results.len() as i32
    });
    // The top result is highlighted, so text and Enter plays it; new text puts the
    // highlight back on it.
    data.set_highlighted(0);
    data.set_state(QuickState::Results);
    if !had_rows {
        data.set_arrived(data.get_arrived() + 1);
    }
    quick.shown.replace(Shown::Answer(s));
}

fn offline(quick: &QuickBox) {
    let data = quick.ui.global::<QuickData>();
    data.set_rows(ModelRc::default());
    data.set_completions_from(-1);
    data.set_highlighted(-1);
    data.set_state(QuickState::Offline);
    quick.shown.replace(Shown::Nothing);
}

fn set_rows(ctx: &Ctx, quick: &QuickBox, rows: Vec<QuickRow>, covers: Vec<Option<String>>) {
    let model = Rc::new(VecModel::from(rows));
    quick
        .ui
        .global::<QuickData>()
        .set_rows(ModelRc::from(model.clone()));
    ctx.covers
        .fill(&model, covers, COVER_PX, |row, img| row.cover = img);
}

/// The rows for `s` as the box shows them: the results, then the completions (the words they
/// add to the typed text bright); and each one's cover URL.
pub fn rows(s: &SearchSuggestions) -> (Vec<QuickRow>, Vec<Option<String>>) {
    let results = s
        .results
        .iter()
        .map(|r| (result_row(&r.item, &r.subtitle), cover_of(&r.item)));
    let completions = s.queries.iter().map(|q| {
        let (typed, rest) = q.text.split_at(q.typed.min(q.text.len()));
        let row = QuickRow {
            kind: SuggestionKind::Query,
            lead: typed.into(),
            hit: rest.into(),
            action: tr!("Search").into(),
            ..QuickRow::default()
        };
        (row, None)
    });
    results.chain(completions).unzip()
}

/// The row under "No results": “Search for “xqzv””, the text bright.
pub fn search_for_row(query: &str) -> QuickRow {
    let full = tr!("Search for “{}”", query);
    let (lead, hit, tail) = match full.find(query) {
        Some(at) => (&full[..at], query, &full[at + query.len()..]),
        None => (full.as_str(), "", ""),
    };
    QuickRow {
        kind: SuggestionKind::Query,
        lead: lead.into(),
        hit: hit.into(),
        tail: tail.into(),
        action: tr!("Search").into(),
        ..QuickRow::default()
    }
}

fn title_of(item: &HomeItem) -> String {
    match item {
        HomeItem::Track(t) => t.display_title().into(),
        HomeItem::Album(a) => a.title.clone(),
        HomeItem::Artist(a) => a.name.clone(),
        HomeItem::Playlist(p) => p.title.clone(),
    }
}

fn cover_of(item: &HomeItem) -> Option<String> {
    match item {
        HomeItem::Track(t) => t.thumbnail.clone(),
        HomeItem::Album(a) => a.thumbnail.clone(),
        HomeItem::Artist(a) => a.thumbnail.clone(),
        HomeItem::Playlist(p) => p.thumbnail.clone(),
    }
}

/// A result's row: its title; its line, the artist (an album's or a song's) or the playlist's
/// author, from the reply's line when the item doesn't name them, none for an artist; its kind
/// and Enter's name for it; an artist without a photo, their initials.
fn result_row(item: &HomeItem, subtitle: &str) -> QuickRow {
    let (kind, what, action) = match item {
        HomeItem::Track(t) if t.video_type != VideoType::Song => {
            (SuggestionKind::Song, tr!("Video"), tr!("Play video"))
        }
        HomeItem::Track(_) => (SuggestionKind::Song, tr!("Song"), tr!("Play song")),
        HomeItem::Album(_) => (SuggestionKind::Album, tr!("Album"), tr!("Play album")),
        HomeItem::Artist(_) => (SuggestionKind::Artist, tr!("Artist"), tr!("Play artist")),
        HomeItem::Playlist(_) => (
            SuggestionKind::Playlist,
            tr!("Playlist"),
            tr!("Play playlist"),
        ),
    };
    let named = match item {
        HomeItem::Track(t) => artist_names(&t.artists),
        HomeItem::Album(a) => artist_names(&a.artists),
        HomeItem::Playlist(p) => p.author.clone().unwrap_or_default(),
        HomeItem::Artist(_) => String::new(),
    };
    // "Song • Josh Woodward • 111M plays": the part after the kind.
    let line = if named.is_empty() && !matches!(item, HomeItem::Artist(_)) {
        subtitle.split(" • ").nth(1).unwrap_or_default().to_string()
    } else {
        named
    };
    let title = title_of(item);
    let initials = match item {
        HomeItem::Artist(a) if a.thumbnail.is_none() => initials(&title),
        _ => String::new(),
    };
    QuickRow {
        kind,
        title: shown(&title),
        line: shown(&line),
        what: what.into(),
        action: action.into(),
        initials: initials.into(),
        ..QuickRow::default()
    }
}

/// "JW" for "Josh Woodward": the first letters of the first two words, capitals.
pub fn initials(name: &str) -> String {
    shown(name)
        .split_whitespace()
        .filter_map(|w| w.chars().find(|c| c.is_alphanumeric()))
        .take(2)
        .flat_map(char::to_uppercase)
        .collect()
}

/// What row `i` of what the box shows is.
enum Picked {
    Item(Recent),
    Search(String),
}

fn picked(quick: &QuickBox, i: usize) -> Option<Picked> {
    match &*quick.shown.borrow() {
        Shown::Nothing => None,
        Shown::None(query) => (i == 0).then(|| Picked::Search(query.clone())),
        Shown::Recent(picks) => picks.get(i).cloned().map(Picked::Item),
        Shown::Answer(s) => match s.results.get(i) {
            Some(r) => Some(Picked::Item(Recent {
                item: r.item.clone(),
                subtitle: r.subtitle.clone(),
            })),
            None => s
                .queries
                .get(i - s.results.len())
                .map(|q| Picked::Search(q.text.clone())),
        },
    }
}

fn pick(ctx: &Rc<Ctx>, i: usize, how: QuickPick) {
    let Some(quick) = BOX.with_borrow(Clone::clone) else {
        return;
    };
    match picked(&quick, i) {
        None => {}
        // A completion runs the search in featherplay.
        Some(Picked::Search(query)) => {
            close_later(false);
            app::show_window(ctx);
            if let Some(ui) = ctx.window() {
                ui.global::<TopBarData>().set_query(query.as_str().into());
            }
            ctx.navigate(link::search_page(&query));
        }
        Some(Picked::Item(item)) => {
            remember(&item);
            match how {
                QuickPick::Play => {
                    close_later(true);
                    play(ctx, &item.item);
                }
                QuickPick::Next => play_next(ctx, &quick, i, &item.item),
                QuickPick::Open => {
                    close_later(false);
                    app::show_window(ctx);
                    if let Some(page) = page_of(&item.item) {
                        ctx.navigate(page);
                    }
                }
            }
        }
    }
}

/// Plays a pick: a song with its radio after it, as the top bar's do; an album or playlist
/// from its first song; an artist as their page's Play does.
fn play(ctx: &Rc<Ctx>, item: &HomeItem) {
    match item {
        HomeItem::Track(t) => {
            song_menu::remember(std::slice::from_ref(t));
            ctx.send(Action::Play {
                tracks: vec![t.clone()],
                start: 0,
                radio: true,
            });
        }
        HomeItem::Album(a) => content::play_album(ctx, a.id.clone()),
        HomeItem::Playlist(p) => content::play_playlist(ctx, p.id.clone()),
        HomeItem::Artist(a) => content::play_artist(ctx, a.id.clone()),
    }
}

/// The page Ctrl+Enter opens: the item's, and a song's album.
fn page_of(item: &HomeItem) -> Option<Page> {
    Some(match item {
        HomeItem::Track(t) => Page::Album {
            id: t.album.as_ref()?.id.clone(),
        },
        HomeItem::Album(a) => Page::Album { id: a.id.clone() },
        HomeItem::Playlist(p) => Page::Playlist { id: p.id.clone() },
        HomeItem::Artist(a) => Page::Artist { id: a.id.clone() },
    })
}

/// Shift+Enter: the song (an album's or a playlist's songs, an artist's top songs) to play
/// next, after what this box set to play next before; row `i` says so, and the box stays.
fn play_next(ctx: &Rc<Ctx>, quick: &QuickBox, i: usize, item: &HomeItem) {
    let tag = if quick.last_next.borrow().is_some() {
        tr!("Plays after it")
    } else {
        tr!("Plays next")
    };
    let data = quick.ui.global::<QuickData>();
    let rows = data.get_rows();
    if let Some(mut row) = rows.row_data(i) {
        row.tag = tag.into();
        rows.set_row_data(i, row);
    }
    match item {
        HomeItem::Track(t) => put_next(ctx, vec![t.clone()]),
        HomeItem::Album(a) => tracks_of(ctx, Place::Album(a.id.clone())),
        HomeItem::Playlist(p) => tracks_of(ctx, Place::Playlist(p.id.clone())),
        HomeItem::Artist(a) => tracks_of(ctx, Place::Artist(a.id.clone())),
    }
}

enum Place {
    Album(AlbumId),
    Playlist(PlaylistId),
    Artist(ArtistId),
}

fn tracks_of(ctx: &Rc<Ctx>, place: Place) {
    ctx.load(
        move |source| async move {
            match place {
                Place::Album(id) => source.album(&id).await.map(|a| a.tracks),
                Place::Playlist(id) => source.playlist(&id).await.map(|p| p.tracks),
                Place::Artist(id) => source.artist(&id).await.map(|a| a.top_songs),
            }
        },
        |ctx, _, result| match result {
            Ok(tracks) => put_next(ctx, tracks),
            Err(e) => log!("quick search: play next: {e}"),
        },
    );
}

/// Sends `tracks` to play next, after this box's last ones; remembered as its last.
fn put_next(ctx: &Ctx, tracks: Vec<Track>) {
    let Some(last) = tracks.last().map(|t| t.id.clone()) else {
        return;
    };
    song_menu::remember(&tracks);
    let after = BOX.with_borrow(|b| {
        b.as_ref()
            .and_then(|q| q.last_next.replace(Some(last.clone())))
    });
    ctx.send(Action::Queue(QueueAction::PlayAfter { tracks, after }));
}

/// The file the recent picks are kept in.
fn recent_file() -> PathBuf {
    RECENT_FILE.with_borrow(|f| {
        f.clone()
            .unwrap_or_else(|| Paths::resolve().cache.join("quick-search.json"))
    })
}

/// Keeps the recent picks in `path` from now on (a test's or a snapshot's own).
pub fn keep_recent_in(path: PathBuf) {
    RECENT_FILE.set(Some(path));
}

/// `--snapshot quick-search…`: `picks` are the recent picks, whatever the file has.
pub fn recent_for_snapshot(picks: Vec<Recent>) {
    RECENT_SHOWN.set(Some(picks));
}

/// The recent picks, newest first; none when the file is missing or unreadable.
pub fn load_recent() -> Vec<Recent> {
    if let Some(picks) = RECENT_SHOWN.with_borrow(Clone::clone) {
        return picks;
    }
    std::fs::read(recent_file())
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Puts `pick` first among the recent picks, once, and keeps the newest `RECENT`.
fn remember(pick: &Recent) {
    let mut picks = load_recent();
    picks.retain(|r| item_key(&r.item) != item_key(&pick.item));
    picks.insert(0, pick.clone());
    picks.truncate(RECENT);
    // A snapshot's picks stay in memory.
    if RECENT_SHOWN.with_borrow(Option::is_some) {
        RECENT_SHOWN.set(Some(picks));
        return;
    }
    match serde_json::to_vec(&picks) {
        Ok(bytes) => {
            if let Err(e) = featherplay_store::write_atomic(&recent_file(), &bytes) {
                log!("quick search: keeping the recent picks: {e}");
            }
        }
        Err(e) => log!("quick search: recent picks: {e}"),
    }
}

fn item_key(item: &HomeItem) -> (u8, &str) {
    match item {
        HomeItem::Track(t) => (0, t.id.0.as_str()),
        HomeItem::Album(a) => (1, a.id.0.as_str()),
        HomeItem::Artist(a) => (2, a.id.0.as_str()),
        HomeItem::Playlist(p) => (3, p.id.0.as_str()),
    }
}

/// `--snapshot quick-search…` and the tests: the box made (not shown) with `text` typed in
/// it, the answer asked for at once.
pub fn type_for_snapshot(ctx: &Rc<Ctx>, ui: &QuickSearch, text: &str) {
    ui.global::<QuickData>().set_query(text.into());
    edited(ctx, text);
    // The pause or the interval runs on the real clock: ask now.
    if let Some(quick) = BOX.with_borrow(Clone::clone) {
        quick.timer.stop();
        let next = quick.suggester.borrow_mut().due();
        act(ctx, &quick, next);
    }
}

/// `--snapshot quick-search-waiting`: `text` typed, its first answer on its way.
pub fn waiting_for_snapshot(ui: &QuickSearch, text: &str) {
    ui.global::<QuickData>().set_query(text.into());
    if let Some(quick) = BOX.with_borrow(Clone::clone) {
        quick.timer.stop();
        waiting(&quick);
    }
}

/// `--snapshot quick-search-offline`: the box as it is when the answer couldn't come.
pub fn offline_for_snapshot() {
    if let Some(quick) = BOX.with_borrow(Clone::clone) {
        quick.offline.set(true);
        offline(&quick);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_result_reads_title_artist_kind_and_a_completion_brightens_what_it_adds() {
        use featherplay_core::model::{ArtistRef, LikeStatus};
        use featherplay_ytm::{QuerySuggestion, SuggestedResult};
        let song = Track {
            id: TrackId("demo-invisible-light".into()),
            title: "Invisible Light".into(),
            artists: vec![ArtistRef {
                name: "Josh Woodward".into(),
                id: None,
            }],
            album: None,
            duration: None,
            video_type: VideoType::Song,
            thumbnail: None,
            like: LikeStatus::Indifferent,
            counterpart: None,
        };
        let s = SearchSuggestions {
            queries: vec![QuerySuggestion {
                text: "josh woodward".into(),
                typed: 4,
                history: false,
            }],
            results: vec![
                SuggestedResult {
                    item: HomeItem::Artist(featherplay_ytm::ArtistSummary {
                        id: ArtistId("a".into()),
                        name: "Josh Woodward".into(),
                        thumbnail: None,
                    }),
                    subtitle: "Artist • 1.2K monthly audience".into(),
                },
                SuggestedResult {
                    item: HomeItem::Track(song),
                    subtitle: "Song • Josh Woodward • 111 plays".into(),
                },
            ],
        };
        let (rows, _) = rows(&s);
        let parts = |r: &QuickRow| {
            [&r.title, &r.line, &r.what, &r.initials, &r.action].map(|p| p.to_string())
        };
        // An artist without a photo: no line, their initials.
        assert_eq!(
            parts(&rows[0]),
            ["Josh Woodward", "", "Artist", "JW", "Play artist"]
        );
        assert_eq!(
            parts(&rows[1]),
            ["Invisible Light", "Josh Woodward", "Song", "", "Play song"]
        );
        // Results first, the completion after them: the typed part, then the words it adds.
        assert_eq!(rows[2].kind, SuggestionKind::Query);
        assert_eq!(
            (
                rows[2].lead.as_str(),
                rows[2].hit.as_str(),
                rows[2].action.as_str()
            ),
            ("josh", " woodward", "Search")
        );
    }

    #[test]
    fn no_results_offers_to_search_for_the_text() {
        let row = search_for_row("xqzv");
        assert_eq!(row.kind, SuggestionKind::Query);
        assert_eq!(
            [&row.lead, &row.hit, &row.tail].map(|p| p.to_string()),
            ["Search for “", "xqzv", "”"]
        );
        assert_eq!(initials("scott  buckley trio"), "SB");
        assert_eq!(initials("Madonna"), "M");
    }

    #[test]
    fn the_box_sits_centred_a_fifth_of_the_way_down() {
        // 1920 x 1032 at 100 %, and a second monitor on the left at 150 %.
        assert_eq!(place((0, 0, 1920, 1032), 1.0, 640.0), (640, 206));
        assert_eq!(place((-2560, 0, 0, 1392), 1.5, 640.0), (-1760, 278));
    }
}
