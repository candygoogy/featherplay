//! The tray's mini player (`ui/mini_player.slint`): a left click on the tray icon opens it
//! over the notification area, another click, Escape or a click elsewhere closes it.
//! Pinned (its pin button, saved as `mini_pinned`), a click elsewhere, the tray icon's menu
//! and showing the main window leave it open: it stays on top (the window always is), its ×
//! closes it and a press off its controls drags it. It still opens at the tray.
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
use crate::pages::settings;
use crate::popup;
use crate::ui::{AppWindow, MiniData, MiniPlayer, Player, SettingsData, Theme};
use crate::ui_bridge::format_duration;

/// The window's size in logical pixels.
pub const SIZE: (f32, f32) = (340.0, 170.0);
/// The cover's size in logical pixels.
const COVER: u32 = 64;
/// A click on the tray icon takes the activation from an open mini player, which closes;
/// the click itself arrives just after and must not open it again.
const REOPEN_GUARD: Duration = Duration::from_millis(400);

struct Mini {
    ui: MiniPlayer,
    cover_url: RefCell<String>,
}

thread_local! {
    static MINI: RefCell<Option<Rc<Mini>>> = const { RefCell::new(None) };
    /// When the mini player last closed on its own (lost activation, Escape).
    static DISMISSED: Cell<Option<Instant>> = const { Cell::new(None) };
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
    ui.global::<MiniData>().set_pinned(settings::mini_pinned());
    wire(&ui, &main);
    let mini = Rc::new(Mini {
        ui: ui.clone_strong(),
        cover_url: RefCell::new(String::new()),
    });
    // Registered first: a cover already decoded arrives inside `sync`.
    MINI.with_borrow_mut(|m| *m = Some(mini.clone()));
    sync(ctx, &mini, &main);
    Ok(ui)
}

/// Opens the mini player for a click at `(x, y)` (physical screen pixels): above the
/// taskbar's notification area, or beside a taskbar on the left or right.
pub fn open(ctx: &Rc<Ctx>, x: i32, y: i32) -> Result<(), String> {
    let started = Instant::now();
    let ui = create(ctx)?;
    let border = popup::border(&ui.global::<Theme>());
    let (left, top) = popup::show_at(ui.window(), (x, y), SIZE, border)?;
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
    // ⊘ as the tray icon menu's: toast and Undo when the window is on screen, else just
    // rated and skipped.
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

/// The pin button: pins or unpins the open mini player, and saves it for the next one.
fn toggle_pin() {
    let Some(mini) = MINI.with_borrow(Clone::clone) else {
        return;
    };
    let data = mini.ui.global::<MiniData>();
    let pinned = !data.get_pinned();
    data.set_pinned(pinned);
    if let Some(ctx) = Ctx::get() {
        settings::set_mini_pinned(&ctx, pinned);
    }
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
