//! The main window opens where it was left: its size, its position and whether it was
//! maximised, kept in the settings (`[main_window]`). The rules for where a saved place opens
//! on today's monitors are the controller crate's (`featherplay_controller::window_place`);
//! this is the window's side.
//!
//! - **Opening** (`restore`): the place is given to the window before it's made, so it's made
//!   there at that size, with no move after it appears. Maximised, on Linux it's made so too;
//!   on Windows it's made hidden at its maximised bounds, maximised once it exists with no
//!   change of size, and shown then (`made`), its first frame drawn at the size it shows at.
//!   A start in the tray makes its hidden window the same way, and it shows there. A start at
//!   sign-in, which mustn't take the focus, shows it before maximising it, hidden from the
//!   screen meanwhile (DWM's cloak): winit shows a window without the focus as a restored one.
//!   With nothing saved (the first run, an older settings file) it opens at its preferred
//!   size where the desktop puts it, as before.
//! - **Saving**: each move or resize (winit's `Moved` and `Resized`, which `wheel`'s filter
//!   passes here) restarts a short timer, and once the window has stood still the place is
//!   saved, if it changed: one write per move, not one per pixel. Quitting saves it too
//!   (`quitting`). A minimised or hidden window isn't saved then.
//! - **The restored bounds**: Windows keeps them while the window is maximised or minimised
//!   (`window::restored_bounds`). Elsewhere only a window on screen and not maximised says
//!   where it is, so a maximised one keeps the bounds saved before it, and Wayland, which tells
//!   no app where its windows are, keeps the size alone (the compositor places it).

use std::cell::Cell;
use std::time::Duration;

use featherplay_controller::window_place::{self as rules, Frame};
use featherplay_platform::window::{self, ScreenRect};
use featherplay_store::WindowPlace;
use i_slint_core::item_tree::ItemTreeRc;
use i_slint_core::layout::Orientation;
use i_slint_core::window::WindowInner;
use slint::winit_030::WinitWindowAccessor;
use slint::{ComponentHandle, LogicalSize, PhysicalPosition, Timer, TimerMode};

use crate::app::Ctx;
use crate::autostart::Start;
use crate::pages::settings;
use crate::ui::AppWindow;

/// How long the window stands still before its place is saved.
const SETTLES: Duration = Duration::from_millis(600);
/// How long a window maximised to another size takes to draw a frame at it, at most.
const REDRAWN: Duration = Duration::from_millis(150);

thread_local! {
    /// The real window's place is kept (`restore` ran): the offscreen windows of the tests
    /// and snapshots have none.
    static KEPT: Cell<bool> = const { Cell::new(false) };
    /// Windows: the window opens maximised, which `made` does once it exists.
    static MAXIMISE: Cell<bool> = const { Cell::new(false) };
    /// Windows: the bounds restoring it brings back, for a window made at its maximised ones.
    static NORMAL: Cell<Option<ScreenRect>> = const { Cell::new(None) };
    /// The position the window was made at, until it's first read back (`as_asked`).
    static ASKED: Cell<Option<(i32, i32)>> = const { Cell::new(None) };
    /// How far from that position the window says it is (`as_asked`).
    static OFFSET: Cell<(i32, i32)> = const { Cell::new((0, 0)) };
    /// Saves the place once the window stands still (`changed`). Single-shot: no timer runs
    /// while the window doesn't move.
    static SETTLE: Timer = Timer::default();
}

/// Before the main window is made: it opens where it was left, and from now on its place is
/// kept. Returns whether it's maximised once it's made and shown then (`made`), which only
/// Windows does.
pub fn restore(ui: &AppWindow) -> bool {
    KEPT.set(true);
    let Some(saved) = settings::main_window() else {
        return false;
    };
    let (min, _) = layout(ui);
    let monitors = window::monitors();
    let Some(open) = rules::opening(&saved, &monitors, min, frame()) else {
        log!("main window: the saved place can't be used, opening as on a first run");
        return false;
    };
    let w = ui.window();
    w.set_size(LogicalSize::new(open.size.0 as f32, open.size.1 as f32));
    if let Some((x, y)) = open.position {
        w.set_position(PhysicalPosition::new(x, y));
        ASKED.set(Some((x, y)));
    }
    // winit makes a window maximised on Windows by showing it so and hiding it again, a flash
    // that takes the focus, and shows a maximised window with the focus whatever the start
    // asked. There it's made at its maximised bounds and maximised once it's made (`made`).
    // On Linux the window manager or compositor maps it maximised.
    if open.maximised {
        if cfg!(windows) {
            if let Some(max) = rules::maximised(&open, &monitors, frame(), window::frame_insets) {
                w.set_size(LogicalSize::new(max.size.0 as f32, max.size.1 as f32));
                w.set_position(PhysicalPosition::new(max.position.0, max.position.1));
                NORMAL.set(Some(max.normal));
            }
            MAXIMISE.set(true);
        } else {
            w.set_maximized(true);
        }
    }
    log!(
        "main window: opens {:.0}x{:.0} at {}{}",
        open.size.0,
        open.size.1,
        open.position
            .map_or_else(|| "the desktop's place".into(), |(x, y)| format!("{x},{y}")),
        if open.maximised { ", maximised" } else { "" },
    );
    MAXIMISE.get()
}

/// The main window has been made (its handle is known), hidden: one to open maximised is
/// maximised now, without the focus, and shown as `start` shows it (`restore` returned true;
/// in the tray it stays hidden). Made at its maximised bounds, it keeps its size, so it's shown
/// at once, its first frame drawn at that size. Otherwise (Windows placed it elsewhere: a
/// monitor of another scale) it's shown on the event loop's next turn, once Slint has heard of
/// its new size.
///
/// winit shows a window without the focus (a start at sign-in) as a restored one, then
/// maximises it with the focus: that one is shown first, cloaked, then maximised, and
/// uncloaked once its frame at the maximised size is drawn.
///
/// Returns whether it was one to open maximised, which this has shown as `start` asks.
pub fn made(ui: &AppWindow, handle: window::Handle, start: Start) -> bool {
    if !MAXIMISE.replace(false) {
        return false;
    }
    let quiet = start == Start::Quiet;
    if quiet {
        if let Err(e) = window::cloak(handle, true) {
            log!("main window: cloak: {e}");
        }
        show(ui);
    }
    let drawn = match window::maximise_quietly(handle, NORMAL.take()) {
        Ok(size) => {
            let drawn = ui.window().size();
            size == (drawn.width, drawn.height)
        }
        Err(e) => {
            log!("main window: maximise: {e}");
            true
        }
    };
    if !drawn {
        log!("main window: maximised to another size than it was made at, shown once redrawn");
    }
    match start {
        Start::Window if drawn => show(ui),
        Start::Window => {
            let weak = ui.as_weak();
            Timer::single_shot(Duration::ZERO, move || {
                if let Some(ui) = weak.upgrade() {
                    show(&ui);
                }
            });
        }
        Start::Quiet => {
            let uncloak = move || {
                if let Err(e) = window::cloak(handle, false) {
                    log!("main window: uncloak: {e}");
                }
            };
            if drawn {
                uncloak();
            } else {
                Timer::single_shot(REDRAWN, uncloak);
            }
        }
        Start::Tray => {}
    }
    true
}

fn show(ui: &AppWindow) {
    if let Err(e) = ui.show() {
        log!("show window: {e}");
    }
}

/// The main window moved or changed size: its place is saved once it stands still.
pub fn changed() {
    if !KEPT.get() {
        return;
    }
    SETTLE.with(|t| {
        t.start(TimerMode::SingleShot, SETTLES, || {
            if let Some(ctx) = Ctx::get() {
                keep(&ctx, false);
            }
        });
    });
}

/// Featherplay is quitting: the place is saved now, before the controller's last save,
/// also from the tray, where Windows still knows it.
pub fn quitting(ctx: &Ctx) {
    if !KEPT.get() {
        return;
    }
    SETTLE.with(Timer::stop);
    keep(ctx, true);
}

/// Saves where the window is, if that's a place to keep and not the one kept already.
fn keep(ctx: &Ctx, quitting: bool) {
    let Some(ui) = ctx.window() else { return };
    if let Some(place) = current(ctx, &ui, quitting)
        && settings::main_window() != Some(place)
    {
        settings::set_main_window(ctx, place);
    }
}

/// Where the window is: its restored bounds and whether it's maximised. `None` while it's
/// minimised or hidden, but at quit on Windows, which still knows its bounds then.
fn current(ctx: &Ctx, ui: &AppWindow, quitting: bool) -> Option<WindowPlace> {
    let monitors = window::monitors();
    if let Some(r) = ctx.handle().and_then(window::restored_bounds) {
        if r.minimised && !quitting {
            return None;
        }
        return Some(rules::place_of(
            Some(r.position),
            r.size,
            r.scale,
            r.maximised,
            &monitors,
        ));
    }
    let w = ui.window();
    if !w.is_visible() || w.is_minimized() {
        return None;
    }
    if w.is_maximized() {
        // Its bounds before it was maximised aren't known here: the ones kept stay.
        let (_, preferred) = layout(ui);
        let before = settings::main_window().unwrap_or(WindowPlace {
            width: preferred.0,
            height: preferred.1,
            position: None,
            scale: f64::from(w.scale_factor()),
            maximised: true,
            monitor: None,
        });
        return Some(WindowPlace {
            maximised: true,
            ..before
        });
    }
    let size = w.size();
    // None on Wayland: the compositor doesn't say.
    let position = w
        .with_winit_window(|w| w.outer_position().ok())
        .flatten()
        .map(|p| as_asked((p.x, p.y)));
    Some(rules::place_of(
        position,
        (size.width, size.height),
        w.scale_factor(),
        false,
        &monitors,
    ))
}

/// `at`, where the window says it is, as the position to ask for to put it there again. On
/// X11 the two differ: a window asked to go to a point lands off it by its frame, or as the
/// window manager sees fit, while the position it reports is its frame's. The first reading
/// after a start that asked for a position (before the user can have moved it, as it settles
/// once it's made) finds that offset, and each one after takes it off, so the window doesn't
/// creep across the screen from one start to the next.
fn as_asked(at: (i32, i32)) -> (i32, i32) {
    if let Some(asked) = ASKED.take() {
        OFFSET.set((at.0 - asked.0, at.1 - asked.1));
    }
    let (dx, dy) = OFFSET.get();
    (at.0 - dx, at.1 - dy)
}

/// The window's least and preferred inner size, logical pixels, as its layout says
/// (`ui/app.slint`).
fn layout(ui: &AppWindow) -> ((f64, f64), (f64, f64)) {
    let component = WindowInner::from_pub(ui.window()).component();
    let component = ItemTreeRc::borrow_pin(&component);
    let h = component.as_ref().layout_info(Orientation::Horizontal);
    let v = component.as_ref().layout_info(Orientation::Vertical);
    (
        (f64::from(h.min), f64::from(v.min)),
        (
            f64::from(h.preferred_bounded()),
            f64::from(v.preferred_bounded()),
        ),
    )
}

/// The frame round the window at scale 1, logical pixels.
fn frame() -> Frame {
    let (l, t, r, b) = window::frame_insets(1.0);
    (f64::from(l), f64::from(t), f64::from(r), f64::from(b))
}
