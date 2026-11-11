//! The tray's mini player (`ui/mini_player.slint`): a left click on the tray icon opens it
//! over the notification area, another click, Escape or a click elsewhere closes it.
//! Pinned (its pin button), a click elsewhere, the tray icon's menu and showing the main
//! window leave it open: it stays on top (the window always is), its × closes it and a press
//! off its controls drags it. The pin lasts for that opening: every mini player opens
//! unpinned, at the tray. Unpinned with a click that brought it from behind another window,
//! it closes, as it would have had it been unpinned all along.
//!
//! Unpinned, it also watches which window is in front, four times a second: opened from the
//! hidden icons' flyout (^), it never gets the activation, so it can't lose it either. And
//! it hears every press on screen: one on the taskbar closes it, even with the taskbar
//! already in front, where the window in front doesn't change.
//!
//! The window exists only while it shows: `open` creates it (a few milliseconds) and `close`
//! drops it, so a closed mini player costs nothing, no window and no timer. While open it
//! mirrors the main window's `Player` and `Theme` globals (each window has its own) on every
//! state change and on the bar's once-a-second tick, and its buttons go through the main
//! window's `Player` callbacks, the same path as the bar's (`popup`). It never touches the
//! main window's visibility, page or size; only "Open featherplay" shows it.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::{Duration, Instant};

use featherplay_platform::window::{self, Handle};
use slint::ComponentHandle;
use slint::winit_030::{EventResult, WinitWindowAccessor, winit};

use crate::app::{self, Ctx};
use crate::ui::{AppWindow, MiniData, MiniPlayer, Player, SettingsData, Theme};
use crate::ui_bridge::format_duration;
use crate::{popup, shell};

/// The window's size in logical pixels.
pub const SIZE: (f32, f32) = (340.0, 170.0);
/// The cover's size in logical pixels.
const COVER: u32 = 64;
/// A click on the tray icon takes the activation from an open mini player, which closes;
/// the click itself arrives just after and must not open it again.
const REOPEN_GUARD: Duration = Duration::from_millis(400);
/// How often the open, unpinned mini player looks at the window in front.
pub const POLL: Duration = Duration::from_millis(250);

pub use featherplay_platform::window::Foreground;

/// The window classes whose coming to the front doesn't close the mini player: the shell's
/// own, which take the foreground on the way to it, and the tray icon's menu. Seen on
/// Windows 11 (build 26300): the taskbar (`Shell_TrayWnd`, `Shell_SecondaryTrayWnd` on
/// other screens), the hidden icons' flyout (`TopLevelWindowForOverflowXamlIsland`; Windows
/// 10's is `NotifyIconOverflowWindow`), and Explorer's `ForegroundStaging`, which holds the
/// foreground for a moment while it hands it on. tray-icon's own window (`tray_icon_app`)
/// takes the foreground while its menu (`#32768`) shows.
///
/// The taskbar stays neutral here, as the shell hands the foreground through it on the way
/// to the mini player; a press on it closes the mini player instead (`closes_on_press`).
const NEUTRAL: [&str; 7] = [
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "TopLevelWindowForOverflowXamlIsland",
    "NotifyIconOverflowWindow",
    "ForegroundStaging",
    "tray_icon_app",
    "#32768",
];

/// The taskbar's window classes (`Shell_SecondaryTrayWnd` on other screens): a press on it,
/// anywhere (Start, an app's button, the tray, empty space), closes the mini player.
const TASKBAR: [&str; 2] = ["Shell_TrayWnd", "Shell_SecondaryTrayWnd"];

struct Mini {
    ui: MiniPlayer,
    cover_url: RefCell<String>,
    /// The last press on it is what brought it to the front (`pressed`).
    pressed_from_behind: Cell<bool>,
    /// Its window's handle, once it shows on screen (none on the offscreen platform).
    handle: Cell<Option<Handle>>,
    /// Looks at the window in front every `POLL`, while it's unpinned (`watch`).
    poll: slint::Timer,
    /// The window in front when the poll last looked.
    front: Cell<Option<Handle>>,
    /// Hears every press on screen while it shows unpinned (`listen`).
    presses: RefCell<Option<window::PressWatch>>,
}

thread_local! {
    static MINI: RefCell<Option<Rc<Mini>>> = const { RefCell::new(None) };
    /// When the mini player last closed on its own (lost activation, Escape, the poll).
    static DISMISSED: Cell<Option<Instant>> = const { Cell::new(None) };
    /// What the poll reads the window in front with: Windows', or a test's.
    static READ_FOREGROUND: Cell<fn() -> Option<Foreground>> =
        const { Cell::new(window::foreground) };
}

/// Has the foreground poll read the window in front with `read` from now on: the offscreen
/// platform has no windows of its own in front, and tests put theirs there.
pub fn read_foreground_with(read: fn() -> Option<Foreground>) {
    READ_FOREGROUND.set(read);
}

/// Whether the foreground poll runs: only while the mini player shows, unpinned.
pub fn polls() -> bool {
    MINI.with_borrow(|m| m.as_ref().is_some_and(|m| m.poll.running()))
}

/// Whether the mini player shows.
pub fn is_open() -> bool {
    MINI.with_borrow(Option::is_some)
}

/// Whether the mini player shows, pinned.
fn is_pinned() -> bool {
    MINI.with_borrow(|m| {
        m.as_ref()
            .is_some_and(|m| m.ui.global::<MiniData>().get_pinned())
    })
}

/// A left click on the tray icon at `(x, y)` (physical screen pixels): opens the mini
/// player there, or closes it.
pub fn toggle(x: i32, y: i32) {
    if is_open() {
        close();
        return;
    }
    if DISMISSED.take().is_some_and(|t| t.elapsed() < REOPEN_GUARD) {
        return;
    }
    let Some(ctx) = Ctx::get() else { return };
    if let Err(e) = open(&ctx, x, y) {
        log!("mini player: {e}");
        close();
    }
}

/// Builds the mini player with the main window's state, without showing it: `open` places
/// and shows it, the snapshot and the render tests take it as it is.
pub fn create(ctx: &Rc<Ctx>) -> Result<MiniPlayer, String> {
    let main = ctx.window().ok_or("no main window")?;
    let ui = popup::creating(|| {
        MiniPlayer::new().inspect(|ui| {
            ui.window()
                .set_size(slint::LogicalSize::new(SIZE.0, SIZE.1));
        })
    })
    .map_err(|e| format!("window: {e}"))?;
    wire(&ui, &main);
    let mini = Rc::new(Mini {
        ui: ui.clone_strong(),
        cover_url: RefCell::new(String::new()),
        pressed_from_behind: Cell::new(false),
        handle: Cell::new(None),
        poll: slint::Timer::default(),
        front: Cell::new(None),
        presses: RefCell::new(None),
    });
    // Registered first: a cover already decoded arrives inside `sync`.
    MINI.with_borrow_mut(|m| *m = Some(mini.clone()));
    sync(ctx, &mini, &main);
    watch(&mini);
    Ok(ui)
}

/// Opens the mini player for a click at `(x, y)` (physical screen pixels): above the
/// taskbar's notification area, or beside a taskbar on the left or right.
pub fn open(ctx: &Rc<Ctx>, x: i32, y: i32) -> Result<(), String> {
    let started = Instant::now();
    let ui = create(ctx)?;
    let border = popup::border(&ui.global::<Theme>());
    let (left, top) = popup::show_at(ui.window(), (x, y), SIZE)?;
    // Winit makes the window on a later turn of the loop: what needs its handle waits for it.
    shell::when_handle(
        &ui,
        move |_, handle| {
            popup::dress(handle, border);
            window_ready(handle);
            if let Err(e) = window::when_pressed(handle, pressed) {
                log!("mini player: presses: {e}");
            }
        },
        |_| log!("mini player: its window never came"),
    );
    log!(
        "mini player opened at {left},{top} ({:.1} ms)",
        started.elapsed().as_secs_f64() * 1000.0
    );
    Ok(())
}

/// The open mini player's window exists now, with handle `handle`: the poll can tell it from
/// other windows, and unpinned, it hears presses on screen.
fn window_ready(handle: Handle) {
    MINI.with_borrow(|m| {
        if let Some(mini) = m {
            mini.handle.set(Some(handle));
            if !mini.ui.global::<MiniData>().get_pinned() {
                listen(mini);
            }
        }
    });
}

/// Gives the open mini player window handle `handle` without listening for presses: the
/// offscreen platform has no handles, and the poll judges nothing without one.
#[doc(hidden)]
pub fn own_window_for_tests(handle: Handle) {
    MINI.with_borrow(|m| {
        if let Some(mini) = m {
            mini.handle.set(Some(handle));
        }
    });
}

/// Closes and drops the mini player, if it shows.
pub fn close() {
    let Some(mini) = MINI.with_borrow_mut(Option::take) else {
        return;
    };
    let _ = mini.ui.hide();
}

/// Closes the mini player unless it's pinned: the tray icon's menu opens, or the main window
/// shows.
pub fn close_unpinned() {
    if !is_pinned() {
        close();
    }
}

/// The player changed: the open mini player shows it.
pub fn update(ctx: &Rc<Ctx>) {
    let Some(mini) = MINI.with_borrow(Clone::clone) else {
        return;
    };
    if let Some(main) = ctx.window() {
        sync(ctx, &mini, &main);
    }
}

/// The bar's once-a-second tick while sound plays: the progress line moves on.
pub fn tick(at: Duration) {
    MINI.with_borrow(|m| {
        if let Some(mini) = m {
            let p = mini.ui.global::<Player>();
            p.set_position_s(at.as_secs_f32());
            p.set_elapsed(format_duration(at).into());
        }
    });
}

fn wire(ui: &MiniPlayer, main: &AppWindow) {
    let player = ui.global::<Player>();
    popup::wire_player(&player, main, || {
        if let Some(ctx) = Ctx::get() {
            update(&ctx);
        }
    });
    // A seek on its line goes the bar's way, and shows here at once.
    let m = main.as_weak();
    player.on_seek(move |fraction| {
        let Some(main) = m.upgrade() else { return };
        main.global::<Player>().invoke_seek(fraction);
        if let Some(ctx) = Ctx::get() {
            update(&ctx);
        }
    });
    // Not for me as the tray icon menu's: toast and Undo when the window is on screen, else
    // just rated and skipped.
    player.on_not_for_me(|| {
        if let Some(ctx) = Ctx::get() {
            crate::not_for_me::from_tray(&ctx);
        }
    });

    let data = ui.global::<MiniData>();
    // Both close the window from inside its own callbacks: the next turn of the loop does.
    data.on_open_app(|| {
        let _ = slint::invoke_from_event_loop(|| {
            if let Some(ctx) = Ctx::get() {
                app::show_window(&ctx);
            }
        });
    });
    data.on_dismiss(|| dismiss("Escape or its ×"));
    data.on_activation_lost(activation_lost);
    data.on_toggle_pin(toggle_pin);
    let weak = ui.as_weak();
    data.on_drag(move || {
        let Some(ui) = weak.upgrade() else { return };
        // Windows moves it from here until the button is released.
        if let Some(Err(e)) = ui.window().with_winit_window(|w| w.drag_window()) {
            log!("mini player: drag: {e}");
        }
    });
    // Winit's word on the activation, should the focus have left the key scope.
    ui.window().on_winit_window_event(|_, ev| {
        if matches!(ev, winit::event::WindowEvent::Focused(false)) {
            activation_lost();
        }
        EventResult::Propagate
    });
}

/// Another window took the activation: a flyout closes, a pinned mini player stays.
fn activation_lost() {
    if !is_pinned() {
        dismiss("another window took the activation");
    }
}

/// The pin button: pins or unpins the open mini player, for this opening. Unpinned from
/// behind another window (the click on the pin brought it forward), it closes: the user had
/// moved on, and unpinned all along, it would have closed then.
fn toggle_pin() {
    let Some(mini) = MINI.with_borrow(Clone::clone) else {
        return;
    };
    let data = mini.ui.global::<MiniData>();
    let pinned = !data.get_pinned();
    data.set_pinned(pinned);
    if pinned {
        mini.poll.stop();
        mini.presses.take();
    } else if mini.pressed_from_behind.take() {
        dismiss("unpinned from behind another window");
    } else {
        watch(&mini);
    }
}

/// Starts the foreground poll from the window in front now, and listens for presses:
/// unpinned, the mini player closes once another app comes to the front (`moved_away`) or
/// the taskbar is pressed (`closes_on_press`).
fn watch(mini: &Mini) {
    mini.front.set(READ_FOREGROUND.get()().map(|f| f.window));
    mini.poll
        .start(slint::TimerMode::Repeated, POLL, look_at_the_front);
    listen(mini);
}

/// Hears every press on screen from now on, once the mini player shows on screen (the
/// offscreen platform has no screen to press). Pinning or closing it stops that.
fn listen(mini: &Mini) {
    if mini.handle.get().is_none() || mini.presses.borrow().is_some() {
        return;
    }
    match window::watch_presses(pressed_on) {
        Ok(watch) => {
            log!("mini player: listening for presses");
            *mini.presses.borrow_mut() = Some(watch);
        }
        Err(e) => log!("mini player: presses on screen: {e}"),
    }
}

/// A press somewhere on screen, on a top-level window of class `class`. Windows calls this
/// from the UI thread's message loop and it can't unwind, so it only reads `MINI`: nothing
/// may hold `MINI` mutably while it pumps messages.
fn pressed_on(class: &str) {
    log!("mini player: a press on {class:?}");
    if closes_on_press(class) {
        dismiss("a press on the taskbar");
    }
}

/// Whether a press on a top-level window of class `class` closes the unpinned mini player:
/// one on the taskbar does. With the mini player in front, that press takes the activation
/// from it anyway; opened from the flyout, the taskbar is already in front and stays there,
/// so the poll can't see it. A press on the tray icon closes it before the click arrives,
/// which then doesn't open it again (`REOPEN_GUARD`). Presses elsewhere are the poll's and
/// the activation's to judge: the mini player itself, the flyout, the tray icon's menu.
fn closes_on_press(class: &str) -> bool {
    TASKBAR.contains(&class)
}

/// One round of the foreground poll.
fn look_at_the_front() {
    let Some(mini) = MINI.with_borrow(Clone::clone) else {
        return;
    };
    let front = READ_FOREGROUND.get()();
    let mut last = mini.front.get();
    let away = moved_away(front.as_ref(), mini.handle.get(), &mut last);
    mini.front.set(last);
    if away {
        mini.poll.stop();
        dismiss("another window came to the front");
    }
}

/// Whether the window in front, `front`, means the user has moved on from the mini player
/// (window `own`), given the one in front when the poll last looked (`last`, updated).
///
/// Only a change counts: whatever was in front when the poll started (the taskbar, the
/// hidden icons' flyout the click came from, an app that kept the foreground) leaves it
/// open. A change to the mini player itself, to a `NEUTRAL` shell window (the flyout
/// handing the foreground to the taskbar as it closes, a second click on the tray icon,
/// which then toggles it) or to no window at all (Windows between two) leaves it open too.
/// Any other window, an app or the desktop (`Progman`, `WorkerW`), closes it.
fn moved_away(front: Option<&Foreground>, own: Option<Handle>, last: &mut Option<Handle>) -> bool {
    // Until its own window is known (winit makes it after `show` returns), the mini player
    // coming to the front can't be told from another app: nothing is judged, or taken as seen.
    let (Some(front), Some(own)) = (front, own) else {
        return false;
    };
    if *last == Some(front.window) {
        return false;
    }
    *last = Some(front.window);
    front.window != own && !NEUTRAL.contains(&front.class.as_str())
}

/// A mouse button, finger or pen went down on the open mini player; `from_behind` when that
/// press is what brought it to the front (`window::when_pressed`). The window is in front by
/// the time a click's callback runs either way, so this is how the pin tells the two apart.
pub fn pressed(from_behind: bool) {
    MINI.with_borrow(|m| {
        if let Some(mini) = m {
            mini.pressed_from_behind.set(from_behind);
        }
    });
}

/// Closes the open mini player for `why`, which the log keeps.
fn dismiss(why: &str) {
    if !is_open() {
        return;
    }
    log!("mini player closes: {why}");
    DISMISSED.set(Some(Instant::now()));
    let _ = slint::invoke_from_event_loop(close);
}

/// Copies the main window's theme and player into the mini player's own globals, and loads
/// its cover when the track's changed.
fn sync(ctx: &Rc<Ctx>, mini: &Mini, main: &AppWindow) {
    popup::sync_theme(
        &main.global::<Theme>(),
        &mini.ui.global::<Theme>(),
        main.global::<SettingsData>().get_appearance(),
    );
    let (from, to) = (main.global::<Player>(), mini.ui.global::<Player>());
    popup::sync_player(&from, &to);

    let url = if from.get_has_track() {
        from.get_cover_url().to_string()
    } else {
        String::new()
    };
    if *mini.cover_url.borrow() == url {
        return;
    }
    mini.cover_url.replace(url.clone());
    let data = mini.ui.global::<MiniData>();
    data.set_cover(Default::default());
    let weak = mini.ui.as_weak();
    let wanted = url.clone();
    ctx.covers.load(&wanted, COVER, move |img| {
        let Some(ui) = weak.upgrade() else { return };
        let still = MINI.with_borrow(|m| m.as_ref().is_some_and(|m| *m.cover_url.borrow() == url));
        if still {
            ui.global::<MiniData>().set_cover(img);
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    const OWN: u32 = 0x1;
    const APP: u32 = 0x40;

    /// Window `n` (`OWN`, `APP`, another), as the poll tells them apart.
    fn w(n: u32) -> Handle {
        crate::shell::stand_in_handle(n)
    }

    fn front(n: u32, class: &str) -> Foreground {
        Foreground {
            window: w(n),
            class: class.into(),
        }
    }

    #[test]
    fn the_window_in_front_at_the_start_leaves_it_open() {
        let mut last = Some(w(APP));
        let app = front(APP, "Chrome_WidgetWin_1");
        assert!(!moved_away(Some(&app), Some(w(OWN)), &mut last));
    }

    #[test]
    fn another_app_coming_to_the_front_closes_it() {
        let mut last = Some(w(0x10));
        let app = front(APP, "Chrome_WidgetWin_1");
        assert!(moved_away(Some(&app), Some(w(OWN)), &mut last));
    }

    #[test]
    fn the_desktop_coming_to_the_front_closes_it() {
        for class in ["Progman", "WorkerW"] {
            let mut last = Some(w(0x10));
            let desktop = front(0x30, class);
            assert!(
                moved_away(Some(&desktop), Some(w(OWN)), &mut last),
                "{class}"
            );
        }
    }

    #[test]
    fn the_mini_player_itself_coming_to_the_front_leaves_it_open() {
        let mut last = Some(w(0x10));
        let own = front(OWN, "Window Class");
        assert!(!moved_away(Some(&own), Some(w(OWN)), &mut last));
        assert_eq!(last, Some(w(OWN)));
    }

    #[test]
    fn nothing_closes_it_before_its_own_window_is_known() {
        // Winit makes the window after `show` returns: until its handle is known, the mini
        // player coming to the front (a press on its play button) looks like any other window.
        let mut last = Some(w(0x20));
        let itself = front(OWN, "Window Class");
        assert!(!moved_away(Some(&itself), None, &mut last));
        assert_eq!(
            last,
            Some(w(0x20)),
            "a window it couldn't judge was taken as seen"
        );
    }

    #[test]
    fn the_shell_and_the_tray_menu_coming_to_the_front_leave_it_open() {
        for class in NEUTRAL {
            let mut last = Some(w(APP));
            let shell = front(0x20, class);
            assert!(
                !moved_away(Some(&shell), Some(w(OWN)), &mut last),
                "{class}"
            );
        }
    }

    #[test]
    fn no_window_in_front_leaves_it_open_and_the_last_one_kept() {
        let mut last = Some(w(0x10));
        assert!(!moved_away(None, Some(w(OWN)), &mut last));
        assert_eq!(last, Some(w(0x10)));
    }

    #[test]
    fn the_first_app_coming_back_after_the_taskbar_closes_it() {
        let mut last = Some(w(APP));
        let taskbar = front(0x20, "Shell_TrayWnd");
        let app = front(APP, "Chrome_WidgetWin_1");
        assert!(!moved_away(Some(&taskbar), Some(w(OWN)), &mut last));
        assert!(moved_away(Some(&app), Some(w(OWN)), &mut last));
    }

    #[test]
    fn a_press_on_the_taskbar_closes_it() {
        for class in ["Shell_TrayWnd", "Shell_SecondaryTrayWnd"] {
            assert!(closes_on_press(class), "{class}");
        }
    }

    #[test]
    fn a_press_anywhere_else_leaves_it_to_the_poll() {
        let elsewhere = [
            "TopLevelWindowForOverflowXamlIsland",
            "NotifyIconOverflowWindow",
            "ForegroundStaging",
            "#32768",
            "Window Class",
            "Chrome_WidgetWin_1",
            "Progman",
            "",
        ];
        for class in elsewhere {
            assert!(!closes_on_press(class), "{class}");
        }
    }
}
