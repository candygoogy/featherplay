//! Private memory: what the app lets go of while its window shows nothing, and the log line
//! that lets a measurement see it.
//!
//! Minimised or hidden in the tray, the window shows nothing, so the pages' covers, the
//! in-memory cover cache but for the shown page's covers (`Covers::show(None)`), the
//! renderer's layers, images and text, and the window's pixel buffer (32 MB at 4K;
//! `vendor/i-slint-renderer-skia/VENDORED.md`, change 5) all go. Shown again, the page's
//! covers come back from that cache and the first frame paints the window whole, as it does
//! after a restore anyway (`repaint`).
//!
//! A GPU surface (a `gpu` build's main window) is kept through the first `KEEP_GPU` hidden, so
//! a quick restore draws at once; then it goes too, with the device: most of a GPU window's
//! memory (`release_gpu`).
//!
//! While sound plays, the private bytes go to the log once a minute (`tick`, on a timer of its
//! own that runs only while sound plays, so nothing runs for it while paused).

use std::cell::Cell;
use std::time::{Duration, Instant};

use i_slint_core::window::WindowInner;
use slint::{ComponentHandle, Timer, TimerMode};

use crate::app::Ctx;

pub(crate) use featherplay_platform::memory::compact_heaps;
pub use featherplay_platform::memory::private_bytes;

/// How often `tick` logs.
const EVERY: Duration = Duration::from_secs(60);
/// How long a hidden window keeps its GPU surface.
pub const KEEP_GPU: Duration = Duration::from_secs(30);

thread_local! {
    /// The window shows nothing (minimised or in the tray).
    static HIDDEN: Cell<bool> = const { Cell::new(false) };
    /// When `tick` last logged.
    static LOGGED: Cell<Option<Instant>> = const { Cell::new(None) };
    /// `tick`'s timer, running while sound plays.
    static LOG_TIMER: Timer = Timer::default();
    /// Lets go of the GPU surface `KEEP_GPU` after the window was hidden.
    static GPU_TIMER: Timer = Timer::default();
}

fn mb(bytes: u64) -> f64 {
    bytes as f64 / 1024.0 / 1024.0
}

/// The window was minimised or hidden in the tray (`true`), or shows again.
pub fn hidden(ctx: &Ctx, hidden: bool) {
    if HIDDEN.replace(hidden) == hidden {
        return;
    }
    // Nothing on screen to resolve ahead of a click, until it shows again.
    crate::prepare::hidden(ctx, hidden);
    if hidden {
        let before = private_bytes();
        ctx.covers.show(None);
        crate::pages::ahead::clear();
        if let Some(ui) = ctx.window() {
            release_drawing(ui.window());
        }
        GPU_TIMER.with(|t| t.start(TimerMode::SingleShot, KEEP_GPU, release_gpu));
        compact_heaps();
        log!(
            "window shows nothing: private {:.0} MB, {:.0} MB after letting go",
            mb(before),
            mb(private_bytes())
        );
    } else {
        GPU_TIMER.with(Timer::stop);
        ctx.covers.show(Some(ctx.current_page().kind()));
        if let Some(ui) = ctx.window() {
            resume_drawing(ui.window());
        }
        // The bar's clock stopped while nothing showed the time, and Home's band didn't
        // follow the player.
        if let Some(ctx) = Ctx::get() {
            crate::bar::showing(&ctx);
            crate::pages::home::shown_again(&ctx);
        }
    }
}

/// Whether the window shows nothing (`hidden`).
pub fn is_hidden() -> bool {
    HIDDEN.get()
}

/// Whether the hidden window still keeps its GPU surface (if it has one): `KEEP_GPU` hasn't
/// passed since it was hidden.
pub fn keeps_gpu() -> bool {
    GPU_TIMER.with(Timer::running)
}

/// The window has shown nothing for `KEEP_GPU`: a second zero size lets go of its GPU surface
/// and device (VENDORED.md, change 29); a software surface stays as it is.
fn release_gpu() {
    if !is_hidden() {
        return;
    }
    if let Some(ui) = Ctx::get().and_then(|ctx| ctx.window()) {
        release_drawing(ui.window());
    }
}

/// The renderer lets go of its caches and the window's pixel buffer: a zero size, which the
/// vendored skia renderer takes for that (VENDORED.md, change 5).
pub(crate) fn release_drawing(window: &slint::Window) {
    let adapter = WindowInner::from_pub(window).window_adapter();
    if let Err(e) = adapter.renderer().resize(slint::PhysicalSize::new(0, 0)) {
        log!("memory: letting the renderer go: {e}");
    }
}

/// The renderer draws again, the whole window: its size (a restore passes it too, but this
/// doesn't depend on that), then a repaint of everything.
pub(crate) fn resume_drawing(window: &slint::Window) {
    let size = window.size();
    if size.width == 0 || size.height == 0 {
        return;
    }
    let adapter = WindowInner::from_pub(window).window_adapter();
    if let Err(e) = adapter.renderer().resize(size) {
        log!("memory: drawing again: {e}");
    }
    crate::repaint::all(window);
}

/// Sound plays (`true`) or stopped: the memory log's timer runs only while it plays, shown or
/// not. The first line comes as sound starts, if a minute has passed since the last.
pub fn playing(on: bool) {
    LOG_TIMER.with(|timer| {
        if !on {
            timer.stop();
            return;
        }
        if timer.running() {
            return;
        }
        if let Some(ctx) = Ctx::get() {
            tick(&ctx);
        }
        timer.start(TimerMode::Repeated, EVERY, || {
            if let Some(ctx) = Ctx::get() {
                tick(&ctx);
            }
        });
    });
}

/// Logs the private bytes, once a minute at most.
fn tick(ctx: &Ctx) {
    let now = Instant::now();
    let due = LOGGED.get().is_none_or(|t| now.duration_since(t) >= EVERY);
    if !due {
        return;
    }
    LOGGED.set(Some(now));
    log!(
        "memory: private {:.0} MB, decoded covers cached {:.1} MB{}",
        mb(private_bytes()),
        mb(ctx.covers.cached_bytes() as u64),
        if is_hidden() { ", window hidden" } else { "" }
    );
}
