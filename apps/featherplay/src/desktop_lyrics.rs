//! Desktop lyrics (`ui/desktop_lyrics.slint`): a small borderless strip, always on top, with
//! the line being sung and the next one, in the lyrics' panel colour, over whatever app is in
//! use. On Windows and on X11; Wayland lets no app keep a window on top, place it or let
//! clicks through it, so there it's not offered (`available`), and Settings says so.
//!
//! One state behind every switch (Settings' row, the tray menu, the lyrics menu, Ctrl+Shift+L
//! and the strip's ×): the setting, kept with how the strip was left (locked, its size, its
//! place). It shows while the setting is on and "Show lyrics" is too.
//!
//! The window exists only while it shows: `open` creates it (without taking the activation
//! from the app in front) and `close` drops it, so with desktop lyrics off it costs nothing,
//! no window and no timer. While it shows it counts as lyrics on screen
//! (`LyricsData.desktop`), so the lookup and the one timer per line (`lyrics::sync`) run for
//! it even with the main window in the tray; `lyrics` hands it each change (`refresh`), and it
//! repaints once per line. Nothing ticks between lines; a drag's end is saved by one short
//! single-shot timer.
//!
//! Its place: first just above the taskbar, centred on the main window's screen; once dragged,
//! where it was left, as the middle of its bottom edge, so a longer line grows it upward and a
//! size change keeps it centred there. That place belongs to its screen: once the screen is
//! gone or changed, it goes back to the default. Unlocked the first time (its second line
//! saying once that it can be dragged and locked); locked, the window lets every click, the
//! wheel and hover through to the app beneath (winit's cursor hit test: Win32's extended
//! styles, X11's input shape) and shows no controls. The tray menu and the lyrics menu unlock
//! it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use featherplay_lyrics::Line;
use featherplay_platform::tray::DesktopLyricsMenu;
use featherplay_platform::window::{self, Handle, ScreenRect};
use featherplay_platform::{Feature, supports};
use featherplay_store::{DesktopLyrics as Setting, StripPlace, StripSize};
use slint::winit_030::{EventResult, WinitWindowAccessor, winit};
use slint::{ComponentHandle, PhysicalPosition, PhysicalSize};

use crate::app::{self, Ctx};
use crate::pages::settings;
use crate::ui::{
    AppWindow, DesktopLyrics, LyricsData, LyricsState, Player, SettingsData, StripData,
};
use crate::{chrome, popup, shell};

/// Between the strip's bottom edge and the taskbar, at first, in logical pixels.
const GAP: f32 = 40.0;
/// A drag is over once the window has stood still this long: its place is saved then.
const DRAG_SETTLES: Duration = Duration::from_millis(400);

struct Strip {
    ui: DesktopLyrics,
    /// Its window's handle, once it shows on screen (none on the offscreen platform).
    handle: Cell<Option<Handle>>,
    /// The middle of its bottom edge, physical screen pixels, once placed.
    anchor: Cell<Option<(i32, i32)>>,
    /// A drag moved it: `Moved` events are the user's until the drag settles.
    dragging: Cell<bool>,
    settle: slint::Timer,
    /// The first opening's hint shows in the second line.
    hint: Cell<bool>,
}

thread_local! {
    static STRIP: RefCell<Option<Rc<Strip>>> = const { RefCell::new(None) };
}

/// Whether this desktop can show the strip: everywhere but Wayland.
pub fn available() -> bool {
    supports(Feature::DesktopLyrics)
}

/// Whether the switches for it show: on a desktop that can, with "Show lyrics" on.
pub fn offered() -> bool {
    available() && settings::lyrics_shown()
}

/// Whether the strip shows.
pub fn is_open() -> bool {
    STRIP.with_borrow(Option::is_some)
}

/// The strip's window, while it shows (the tests drive it).
#[doc(hidden)]
pub fn strip() -> Option<DesktopLyrics> {
    STRIP.with_borrow(|s| s.as_ref().map(|s| s.ui.clone_strong()))
}

/// The tray menu's desktop lyrics entries.
pub fn menu_state() -> DesktopLyricsMenu {
    let s = settings::desktop_lyrics();
    DesktopLyricsMenu {
        offered: offered(),
        on: is_open(),
        locked: s.locked,
    }
}

/// Once at startup, after the settings: shows the strip if desktop lyrics were left on.
pub fn init(ctx: &Rc<Ctx>) {
    apply(ctx);
}

/// Desktop lyrics on or off (Settings' row, the tray menu, the lyrics menu, Ctrl+Shift+L).
pub fn set_on(ctx: &Rc<Ctx>, on: bool) {
    let mut s = settings::desktop_lyrics();
    if s.on != on {
        s.on = on;
        settings::set_desktop_lyrics(ctx, s);
    }
    apply(ctx);
}

/// Turns desktop lyrics the other way (the tray menu's entry, Ctrl+Shift+L).
pub fn toggle(ctx: &Rc<Ctx>) {
    if offered() {
        set_on(ctx, !is_open());
    }
}

/// Locks the shown strip, or unlocks it (its Lock, the tray menu, the lyrics menu).
pub fn toggle_lock(ctx: &Rc<Ctx>) {
    let mut s = settings::desktop_lyrics();
    s.locked = !s.locked;
    settings::set_desktop_lyrics(ctx, s.clone());
    if let Some(strip) = STRIP.with_borrow(Clone::clone) {
        strip.hint.set(false);
        strip.ui.global::<StripData>().set_locked(s.locked);
        let_clicks_through(&strip, s.locked);
        refresh(ctx);
    }
    menus_changed(ctx);
}

/// Shows or drops the strip as the settings now say (also after "Show lyrics" changed).
pub fn apply(ctx: &Rc<Ctx>) {
    let wanted = offered() && settings::desktop_lyrics().on;
    if wanted && !is_open() {
        if let Err(e) = open(ctx) {
            log!("desktop lyrics: {e}");
            close(ctx);
        }
    } else if !wanted && is_open() {
        close(ctx);
    }
    menus_changed(ctx);
}

/// Settings' row and the tray menu show the strip's state.
fn menus_changed(ctx: &Ctx) {
    let Some(main) = ctx.window() else { return };
    let data = main.global::<SettingsData>();
    data.set_desktop_lyrics(settings::desktop_lyrics().on);
    data.set_desktop_lyrics_here(available());
    shell::set_tray_menu(&main.global::<Player>());
}

/// Builds the strip with the lyrics as they are, without showing it: `open` places and shows
/// it, the snapshot and the tests take it as it is. From now on lyrics count as on screen.
pub fn create(ctx: &Rc<Ctx>) -> Result<DesktopLyrics, String> {
    let main = ctx.window().ok_or("no main window")?;
    let ui = popup::creating_quiet(DesktopLyrics::new).map_err(|e| format!("window: {e}"))?;
    let s = settings::desktop_lyrics();
    let data = ui.global::<StripData>();
    data.set_size(size_index(s.size));
    data.set_locked(s.locked);
    let strip = Rc::new(Strip {
        ui: ui.clone_strong(),
        handle: Cell::new(None),
        anchor: Cell::new(None),
        dragging: Cell::new(false),
        settle: slint::Timer::default(),
        hint: Cell::new(!s.hinted),
    });
    if !s.hinted {
        settings::set_desktop_lyrics(
            ctx,
            Setting {
                hinted: true,
                ..s.clone()
            },
        );
    }
    wire(&strip);
    STRIP.with_borrow_mut(|m| *m = Some(strip));
    theme_changed(ctx);
    inks_changed(&main);
    // Lyrics are on screen now: they're looked up and timed for the strip.
    main.global::<LyricsData>().set_desktop(true);
    refresh(ctx);
    ui.window().set_size(slint::LogicalSize::new(
        ui.get_wanted_width(),
        ui.get_wanted_height(),
    ));
    Ok(ui)
}

/// `--snapshot`: the strip at `size`, with the first opening's hint or without, as it is.
pub fn show_for_snapshot(
    ctx: &Rc<Ctx>,
    size: StripSize,
    hint: bool,
) -> Result<DesktopLyrics, String> {
    let ui = create(ctx)?;
    ui.global::<StripData>().set_size(size_index(size));
    with_strip(|strip| strip.hint.set(hint));
    refresh(ctx);
    Ok(ui)
}

/// Shows the strip where it was left on its screen, else above the taskbar of the main
/// window's screen.
fn open(ctx: &Rc<Ctx>) -> Result<(), String> {
    let ui = create(ctx)?;
    let main = ctx.window().ok_or("no main window")?;
    let (anchor, work, scale) = starting_place(&main);
    let size = physical_size(&ui, scale);
    let (left, top) = top_left(anchor, size, work);
    ui.window().set_position(PhysicalPosition::new(left, top));
    popup::creating_quiet(|| ui.show()).map_err(|e| format!("show: {e}"))?;
    with_strip(|strip| strip.anchor.set(Some(anchor)));
    // The screen's scale may differ from the one the window got: placed again by its own.
    resized(&ui);
    let locked = settings::desktop_lyrics().locked;
    shell::when_handle(
        &ui,
        move |ui, handle| {
            if let Err(e) = window::opaque(handle) {
                log!("desktop lyrics: opaque: {e}");
            }
            let ground = ui.global::<StripData>().get_ink().ground;
            let _ = chrome::popup_border(handle, ground);
            with_strip(|strip| {
                strip.handle.set(Some(handle));
                if locked {
                    let_clicks_through(strip, true);
                }
            });
        },
        |_| log!("desktop lyrics: its window never came"),
    );
    log!("desktop lyrics shown at {left},{top}");
    Ok(())
}

/// Drops the strip, if it shows: lyrics on screen are the main window's again.
fn close(ctx: &Ctx) {
    let Some(strip) = STRIP.with_borrow_mut(Option::take) else {
        return;
    };
    strip.settle.stop();
    let _ = strip.ui.hide();
    if let Some(main) = ctx.window() {
        main.global::<LyricsData>().set_desktop(false);
    }
    log!("desktop lyrics closed");
}

fn with_strip(f: impl FnOnce(&Strip)) {
    if let Some(strip) = STRIP.with_borrow(Clone::clone) {
        f(&strip);
    }
}

fn wire(strip: &Rc<Strip>) {
    let data = strip.ui.global::<StripData>();
    data.on_step_size(|step| {
        if let Some(ctx) = Ctx::get() {
            step_size(&ctx, step);
        }
    });
    data.on_lock(|| {
        if let Some(ctx) = Ctx::get() {
            toggle_lock(&ctx);
        }
    });
    data.on_open_app(|| {
        let _ = slint::invoke_from_event_loop(|| {
            if let Some(ctx) = Ctx::get() {
                app::show_window(&ctx);
            }
        });
    });
    // From inside its own callback: the next turn of the loop drops it.
    data.on_close(|| {
        let _ = slint::invoke_from_event_loop(|| {
            if let Some(ctx) = Ctx::get() {
                set_on(&ctx, false);
            }
        });
    });
    let weak = Rc::downgrade(strip);
    data.on_drag(move || {
        let Some(strip) = weak.upgrade() else { return };
        strip.dragging.set(true);
        if strip.hint.take()
            && let Some(ctx) = Ctx::get()
        {
            refresh(&ctx);
        }
        // Windows (or the window manager) moves it from here until the button is released.
        if let Some(Err(e)) = strip.ui.window().with_winit_window(|w| w.drag_window()) {
            log!("desktop lyrics: drag: {e}");
        }
    });
    let weak = Rc::downgrade(strip);
    strip.ui.window().on_winit_window_event(move |_, ev| {
        match ev {
            winit::event::WindowEvent::Moved(at) => {
                if let Some(strip) = weak.upgrade() {
                    moved(&strip, (at.x, at.y));
                }
            }
            // A screen with another scale: sized again once Slint has taken it.
            winit::event::WindowEvent::ScaleFactorChanged { .. } => {
                let _ = slint::invoke_from_event_loop(|| {
                    with_strip(|strip| resized(&strip.ui));
                });
            }
            _ => {}
        }
        EventResult::Propagate
    });
}

/// The window moved to `at` (its top-left corner, physical pixels). Only a drag's moves are
/// the user's: once it settles, the place is kept for its screen.
fn moved(strip: &Rc<Strip>, at: (i32, i32)) {
    if !strip.dragging.get() {
        return;
    }
    let size = strip.ui.window().size();
    strip.anchor.set(Some((
        at.0 + size.width as i32 / 2,
        at.1 + size.height as i32,
    )));
    let weak = Rc::downgrade(strip);
    strip
        .settle
        .start(slint::TimerMode::SingleShot, DRAG_SETTLES, move || {
            let Some(strip) = weak.upgrade() else { return };
            strip.dragging.set(false);
            if let (Some(anchor), Some(ctx)) = (strip.anchor.get(), Ctx::get()) {
                keep_place(&ctx, anchor);
            }
        });
}

/// Keeps `anchor` as the strip's place on the screen it's on.
fn keep_place(ctx: &Rc<Ctx>, (x, bottom): (i32, i32)) {
    let Some((work, _)) = window::work_area_at(x, bottom - 1) else {
        return;
    };
    let mut s = settings::desktop_lyrics();
    s.place = Some(StripPlace {
        x,
        bottom,
        screen: [work.0, work.1, work.2, work.3],
    });
    settings::set_desktop_lyrics(ctx, s);
    log!("desktop lyrics left at {x},{bottom}");
}

/// A− (-1) or A+ (1): the strip's size, kept, about the middle of its bottom edge.
fn step_size(ctx: &Rc<Ctx>, step: i32) {
    let mut s = settings::desktop_lyrics();
    let Some(size) = s.size.step(step) else {
        return;
    };
    s.size = size;
    settings::set_desktop_lyrics(ctx, s);
    with_strip(|strip| {
        strip.ui.global::<StripData>().set_size(size_index(size));
        resized(&strip.ui);
    });
}

fn size_index(size: StripSize) -> i32 {
    StripSize::ALL.iter().position(|s| *s == size).unwrap_or(1) as i32
}

/// Where the strip starts: the middle of its bottom edge, the work area it's in, and that
/// screen's scale.
fn starting_place(main: &AppWindow) -> ((i32, i32), ScreenRect, f32) {
    let saved = settings::desktop_lyrics().place;
    if let Some(place) = saved
        && let Some((work, scale)) = window::work_area_at(place.x, place.bottom - 1)
        && [work.0, work.1, work.2, work.3] == place.screen
    {
        return ((place.x, place.bottom), work, scale);
    }
    // The main window's screen; hidden in the tray or minimised, the main screen's.
    let w = main.window();
    let (x, y) = if w.is_visible() && !w.is_minimized() {
        let (p, s) = (w.position(), w.size());
        (p.x + s.width as i32 / 2, p.y + s.height as i32 / 2)
    } else {
        (0, 0)
    };
    let (work, scale) = window::work_area_at(x, y).unwrap_or(((0, 0, 1920, 1040), 1.0));
    (
        default_anchor(work, (GAP * scale).round() as i32),
        work,
        scale,
    )
}

/// The strip's first place on a screen whose work area is `work`: centred, `gap` above its
/// bottom (the taskbar).
pub fn default_anchor(work: ScreenRect, gap: i32) -> (i32, i32) {
    ((work.0 + work.2) / 2, work.3 - gap)
}

/// The top-left corner of a strip `size` big whose bottom edge's middle is at `anchor`, kept
/// inside the work area `work`.
pub fn top_left(anchor: (i32, i32), size: (i32, i32), work: ScreenRect) -> (i32, i32) {
    let (left, top, right, bottom) = work;
    let x = (anchor.0 - size.0 / 2).min(right - size.0).max(left);
    let y = (anchor.1 - size.1).min(bottom - size.1).max(top);
    (x, y)
}

fn physical_size(ui: &DesktopLyrics, scale: f32) -> (i32, i32) {
    (
        (ui.get_wanted_width() * scale).round() as i32,
        (ui.get_wanted_height() * scale).round() as i32,
    )
}

/// Gives the window the size its lines want, keeping the middle of its bottom edge where it
/// is (a wrapped line grows it upward).
fn resized(ui: &DesktopLyrics) {
    let window = ui.window();
    let scale = window.scale_factor();
    let (w, h) = physical_size(ui, scale);
    let now = window.size();
    if (now.width as i32, now.height as i32) == (w, h) {
        return;
    }
    window.set_size(PhysicalSize::new(w as u32, h as u32));
    let anchor = STRIP.with_borrow(|s| s.as_ref().and_then(|s| s.anchor.get()));
    if let Some(anchor) = anchor
        && let Some((work, _)) = window::work_area_at(anchor.0, anchor.1 - 1)
    {
        let (left, top) = top_left(anchor, (w, h), work);
        window.set_position(PhysicalPosition::new(left, top));
    }
    // X11's input shape is the window's size when it was set: it follows a new size.
    if cfg!(not(windows)) && !settings::desktop_lyrics().locked {
        let _ = window.with_winit_window(|w| w.set_cursor_hittest(true));
    }
}

/// Lets clicks, the wheel and hover go through the strip to the app beneath, or not.
fn let_clicks_through(strip: &Strip, through: bool) {
    let window = strip.ui.window();
    if let Some(Err(e)) = window.with_winit_window(|w| w.set_cursor_hittest(!through)) {
        log!("desktop lyrics: click-through: {e}");
    }
    if through
        && let Some(handle) = strip.handle.get()
        && let Err(e) = window::layered_opaque(handle)
    {
        log!("desktop lyrics: layered: {e}");
    }
}

/// The lyrics' inks changed (a new cover, the accent setting): the strip takes them.
pub fn inks_changed(main: &AppWindow) {
    with_strip(|strip| {
        let (from, to) = (main.global::<LyricsData>(), strip.ui.global::<StripData>());
        to.set_ink_dark(from.get_ink_dark());
        to.set_ink_light(from.get_ink_light());
        if let Some(handle) = strip.handle.get() {
            let _ = chrome::popup_border(handle, to.get_ink().ground);
        }
    });
}

/// The theme flipped: the strip takes the panel's inks for it.
pub fn theme_changed(ctx: &Ctx) {
    let Some(main) = ctx.window() else { return };
    with_strip(|strip| {
        popup::sync_theme(
            &main.global::<crate::ui::Theme>(),
            &strip.ui.global::<crate::ui::Theme>(),
            main.global::<SettingsData>().get_appearance(),
        );
        if let Some(handle) = strip.handle.get() {
            let ground = strip.ui.global::<StripData>().get_ink().ground;
            let _ = chrome::popup_border(handle, ground);
        }
    });
}

/// The lyrics or the player changed (`lyrics::sync`, each line and each lookup): the strip
/// shows the line now, and the next.
pub fn refresh(ctx: &Ctx) {
    let Some(strip) = STRIP.with_borrow(Clone::clone) else {
        return;
    };
    let Some(main) = ctx.window() else { return };
    let (lyrics, player) = (main.global::<LyricsData>(), main.global::<Player>());
    let lines = crate::lyrics::synced_lines();
    let message = lyrics.get_message();
    let mut shown = shown(&Now {
        has_track: player.get_has_track(),
        title: &player.get_title(),
        artist: &player.get_artist(),
        state: lyrics.get_state(),
        lines: &lines,
        current: lyrics.get_current(),
        failed: &message,
    });
    if strip.hint.get() {
        shown.under =
            featherplay_core::tr!("Drag to move it · lock it to let clicks through").into();
        shown.quiet = true;
    }
    let data = strip.ui.global::<StripData>();
    if data.get_line() != shown.line.as_str() {
        data.set_line(shown.line.into());
    }
    if data.get_under() != shown.under.as_str() {
        data.set_under(shown.under.into());
    }
    data.set_quiet(shown.quiet);
    resized(&strip.ui);
}

/// What the strip is given to show.
pub struct Now<'a> {
    pub has_track: bool,
    pub title: &'a str,
    pub artist: &'a str,
    pub state: LyricsState,
    /// The synced lines (empty for plain lyrics or none).
    pub lines: &'a [Line],
    /// The line playing, -1 before the first.
    pub current: i32,
    /// What the lyrics views say for a failed lookup.
    pub failed: &'a str,
}

/// What the strip shows: its first line, its second, and whether that's a quiet note.
#[derive(Debug, PartialEq, Eq)]
pub struct Shown {
    pub line: String,
    pub under: String,
    pub quiet: bool,
}

/// The strip's two lines for `now`: the line being sung and the next; between lines (or
/// before the first) a ♪ with the next under it; otherwise what the lyrics views say, with
/// the song named under it, or the song's title with "No lyrics for this song".
pub fn shown(now: &Now) -> Shown {
    use featherplay_core::tr;
    let note = |line: &str, under: String| Shown {
        line: line.into(),
        under,
        quiet: true,
    };
    let song = || match (now.title, now.artist) {
        (t, "") => t.to_string(),
        (t, a) => format!("{t} · {a}"),
    };
    if !now.has_track {
        return note(
            tr!("Nothing playing"),
            tr!("Play a song to see its lyrics here").into(),
        );
    }
    match now.state {
        LyricsState::Synced => {
            let at = usize::try_from(now.current).ok();
            let next = now
                .lines
                .iter()
                .skip(at.map_or(0, |i| i + 1))
                .map(|l| l.text.trim())
                .find(|t| !t.is_empty())
                .unwrap_or_default()
                .to_string();
            match at.and_then(|i| now.lines.get(i)).map(|l| l.text.trim()) {
                Some(line) if !line.is_empty() => Shown {
                    line: line.into(),
                    under: next,
                    quiet: false,
                },
                _ => note("♪", next),
            }
        }
        LyricsState::Plain => note(tr!("Lines aren't synced for this song"), song()),
        LyricsState::None => {
            let none = tr!("No lyrics for this song");
            let under = if now.artist.is_empty() {
                none.to_string()
            } else {
                format!("{} · {none}", now.artist)
            };
            note(now.title, under)
        }
        LyricsState::Error => note(now.failed, song()),
        LyricsState::Loading | LyricsState::Idle => note(tr!("Finding lyrics…"), song()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(texts: &[&str]) -> Vec<Line> {
        texts
            .iter()
            .enumerate()
            .map(|(i, t)| Line {
                start: Duration::from_secs(i as u64 * 4),
                text: (*t).into(),
            })
            .collect()
    }

    fn now<'a>(state: LyricsState, lines: &'a [Line], current: i32) -> Now<'a> {
        Now {
            has_track: true,
            title: "Origami",
            artist: "Scott Buckley",
            state,
            lines,
            current,
            failed: "Lyrics are unavailable right now",
        }
    }

    #[test]
    fn a_line_being_sung_shows_with_the_next_one_under_it() {
        let l = lines(&["One", "", "Two", "Three"]);
        assert_eq!(
            shown(&now(LyricsState::Synced, &l, 0)),
            Shown {
                line: "One".into(),
                under: "Two".into(),
                quiet: false
            }
        );
        let last = shown(&now(LyricsState::Synced, &l, 3));
        assert_eq!((last.line.as_str(), last.under.as_str()), ("Three", ""));
    }

    #[test]
    fn between_lines_and_before_the_first_a_note_shows_with_the_next_line() {
        let l = lines(&["One", "", "Two"]);
        for current in [-1, 1] {
            let s = shown(&now(LyricsState::Synced, &l, current));
            assert_eq!(s.line, "♪");
            assert!(s.quiet);
        }
        assert_eq!(shown(&now(LyricsState::Synced, &l, 1)).under, "Two");
        assert_eq!(shown(&now(LyricsState::Synced, &l, -1)).under, "One");
    }

    #[test]
    fn a_song_without_lyrics_is_named_and_says_so() {
        assert_eq!(
            shown(&now(LyricsState::None, &[], -1)),
            Shown {
                line: "Origami".into(),
                under: "Scott Buckley · No lyrics for this song".into(),
                quiet: true
            }
        );
    }

    #[test]
    fn the_other_states_say_what_the_lyrics_views_say_and_name_the_song() {
        let say = |state| {
            let s = shown(&now(state, &[], -1));
            (s.line, s.under)
        };
        let song = "Origami · Scott Buckley".to_string();
        assert_eq!(
            say(LyricsState::Loading),
            ("Finding lyrics…".into(), song.clone())
        );
        assert_eq!(
            say(LyricsState::Plain),
            ("Lines aren't synced for this song".into(), song.clone())
        );
        assert_eq!(
            say(LyricsState::Error),
            ("Lyrics are unavailable right now".into(), song)
        );
        let idle = shown(&Now {
            has_track: false,
            ..now(LyricsState::Idle, &[], -1)
        });
        assert_eq!(idle.line, "Nothing playing");
    }

    /// A 1920 × 1080 screen with a 48 px taskbar at the bottom.
    const WORK: ScreenRect = (0, 0, 1920, 1032);

    #[test]
    fn it_starts_centred_above_the_taskbar() {
        let anchor = default_anchor(WORK, 40);
        assert_eq!(anchor, (960, 992));
        assert_eq!(top_left(anchor, (760, 88), WORK), (580, 904));
    }

    #[test]
    fn a_taller_strip_grows_upward_and_stays_on_screen() {
        assert_eq!(top_left((960, 992), (760, 130), WORK), (580, 862));
        // Dragged against the screen's corner, it stays inside the work area.
        assert_eq!(top_left((10, 1030), (760, 88), WORK), (0, 942));
        let left_screen = (-1920, 0, 0, 1032);
        assert_eq!(top_left((-5, 40), (600, 80), left_screen), (-600, 0));
    }
}
