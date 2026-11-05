//! The tray's mini player (`ui/mini_player.slint`): a left click on the tray icon opens it
//! over the notification area, another click, Escape or a click elsewhere closes it.
//! Pinned (its pin button), a click elsewhere, the tray icon's menu and showing the main
//! window leave it open: it stays on top (the window always is), its × closes it and a press
//! off its controls drags it. The pin lasts for that opening: every mini player opens
//! unpinned, at the tray. Unpinned with a click that brought it from behind another window,
//! it closes, as it would have had it been unpinned all along.
//!
//! Unpinned, it also watches which window is in front, four times a second: opened from the
//! hidden icons' flyout (^), it never gets the activation, so it can't lose it either.
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

use slint::ComponentHandle;
use slint::winit_030::{EventResult, WinitWindowAccessor, winit};

use crate::app::{self, Ctx};
use crate::ui::{AppWindow, MiniData, MiniPlayer, Player, SettingsData, Theme};
use crate::ui_bridge::format_duration;
use crate::{native, popup, shell};

/// The window's size in logical pixels.
pub const SIZE: (f32, f32) = (340.0, 170.0);
/// The cover's size in logical pixels.
const COVER: u32 = 64;
/// A click on the tray icon takes the activation from an open mini player, which closes;
/// the click itself arrives just after and must not open it again.
const REOPEN_GUARD: Duration = Duration::from_millis(400);
/// How often the open, unpinned mini player looks at the window in front.
pub const POLL: Duration = Duration::from_millis(250);

pub use crate::native::Foreground;

/// The window classes whose coming to the front doesn't close the mini player: the shell's
/// own, which take the foreground on the way to it, and the tray icon's menu. Seen on
/// Windows 11 (build 26300): the taskbar (`Shell_TrayWnd`, `Shell_SecondaryTrayWnd` on
/// other screens), the hidden icons' flyout (`TopLevelWindowForOverflowXamlIsland`; Windows
/// 10's is `NotifyIconOverflowWindow`), and Explorer's `ForegroundStaging`, which holds the
/// foreground for a moment while it hands it on. tray-icon's own window (`tray_icon_app`)
/// takes the foreground while its menu (`#32768`) shows.
const NEUTRAL: [&str; 7] = [
    "Shell_TrayWnd",
    "Shell_SecondaryTrayWnd",
    "TopLevelWindowForOverflowXamlIsland",
    "NotifyIconOverflowWindow",
    "ForegroundStaging",
    "tray_icon_app",
    "#32768",
];

struct Mini {
    ui: MiniPlayer,
    cover_url: RefCell<String>,
    /// The last press on it is what brought it to the front (`pressed`).
    pressed_from_behind: Cell<bool>,
    /// Its window's handle, once it shows on screen (none on the offscreen platform).
    hwnd: Cell<Option<isize>>,
    /// Looks at the window in front every `POLL`, while it's unpinned (`watch`).
    poll: slint::Timer,
    /// The window in front when the poll last looked.
    front: Cell<Option<isize>>,
}

thread_local! {
    static MINI: RefCell<Option<Rc<Mini>>> = const { RefCell::new(None) };
    /// When the mini player last closed on its own (lost activation, Escape, the poll).
    static DISMISSED: Cell<Option<Instant>> = const { Cell::new(None) };
    /// What the poll reads the window in front with: Windows', or a test's.
    static READ_FOREGROUND: Cell<fn() -> Option<Foreground>> =
        const { Cell::new(native::foreground) };
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
        hwnd: Cell::new(None),
        poll: slint::Timer::default(),
        front: Cell::new(None),
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
    let (left, top) = popup::show_at(ui.window(), (x, y), SIZE, border)?;
    if let Some(hwnd) = shell::hwnd(ui.window()) {
        MINI.with_borrow(|m| m.as_ref().map(|m| m.hwnd.set(Some(hwnd))));
        if let Err(e) = native::when_pressed(hwnd, pressed) {
            log!("mini player: presses: {e}");
        }
    }
    log!(
        "mini player opened at {left},{top} ({:.1} ms)",
        started.elapsed().as_secs_f64() * 1000.0
    );
    Ok(())
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
    data.on_dismiss(dismiss);
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
        dismiss();
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
    } else if mini.pressed_from_behind.take() {
        dismiss();
    } else {
        watch(&mini);
    }
}

/// Starts the foreground poll from the window in front now: unpinned, the mini player
/// closes once another app comes to the front (`moved_away`).
fn watch(mini: &Mini) {
    mini.front.set(READ_FOREGROUND.get()().map(|f| f.hwnd));
    mini.poll
        .start(slint::TimerMode::Repeated, POLL, look_at_the_front);
}

/// One round of the foreground poll.
fn look_at_the_front() {
    let Some(mini) = MINI.with_borrow(Clone::clone) else {
        return;
    };
    let front = READ_FOREGROUND.get()();
    let mut last = mini.front.get();
    let away = moved_away(front.as_ref(), mini.hwnd.get(), &mut last);
    mini.front.set(last);
    if away {
        mini.poll.stop();
        dismiss();
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
fn moved_away(front: Option<&Foreground>, own: Option<isize>, last: &mut Option<isize>) -> bool {
    let Some(front) = front else {
        return false;
    };
    if *last == Some(front.hwnd) {
        return false;
    }
    *last = Some(front.hwnd);
    Some(front.hwnd) != own && !NEUTRAL.contains(&front.class.as_str())
}

/// A mouse button, finger or pen went down on the open mini player; `from_behind` when that
/// press is what brought it to the front (`native::when_pressed`). The window is in front by
/// the time a click's callback runs either way, so this is how the pin tells the two apart.
pub fn pressed(from_behind: bool) {
    MINI.with_borrow(|m| {
        if let Some(mini) = m {
            mini.pressed_from_behind.set(from_behind);
        }
    });
}

fn dismiss() {
    if !is_open() {
        return;
    }
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

    const OWN: isize = 0x1;
    const APP: isize = 0x40;

    fn front(hwnd: isize, class: &str) -> Foreground {
        Foreground {
            hwnd,
            class: class.into(),
        }
    }

    #[test]
    fn the_window_in_front_at_the_start_leaves_it_open() {
        let mut last = Some(APP);
        let app = front(APP, "Chrome_WidgetWin_1");
        assert!(!moved_away(Some(&app), Some(OWN), &mut last));
    }

    #[test]
    fn another_app_coming_to_the_front_closes_it() {
        let mut last = Some(0x10);
        let app = front(APP, "Chrome_WidgetWin_1");
        assert!(moved_away(Some(&app), Some(OWN), &mut last));
    }

    #[test]
    fn the_desktop_coming_to_the_front_closes_it() {
        for class in ["Progman", "WorkerW"] {
            let mut last = Some(0x10);
            let desktop = front(0x30, class);
            assert!(moved_away(Some(&desktop), Some(OWN), &mut last), "{class}");
        }
    }

    #[test]
    fn the_mini_player_itself_coming_to_the_front_leaves_it_open() {
        let mut last = Some(0x10);
        let own = front(OWN, "Window Class");
        assert!(!moved_away(Some(&own), Some(OWN), &mut last));
        assert_eq!(last, Some(OWN));
    }

    #[test]
    fn the_shell_and_the_tray_menu_coming_to_the_front_leave_it_open() {
        for class in NEUTRAL {
            let mut last = Some(APP);
            let shell = front(0x20, class);
            assert!(!moved_away(Some(&shell), Some(OWN), &mut last), "{class}");
        }
    }

    #[test]
    fn no_window_in_front_leaves_it_open_and_the_last_one_kept() {
        let mut last = Some(0x10);
        assert!(!moved_away(None, Some(OWN), &mut last));
        assert_eq!(last, Some(0x10));
    }

    #[test]
    fn the_first_app_coming_back_after_the_taskbar_closes_it() {
        let mut last = Some(APP);
        let taskbar = front(0x20, "Shell_TrayWnd");
        let app = front(APP, "Chrome_WidgetWin_1");
        assert!(!moved_away(Some(&taskbar), Some(OWN), &mut last));
        assert!(moved_away(Some(&app), Some(OWN), &mut last));
    }
}
