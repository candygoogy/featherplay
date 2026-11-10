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
//! While sound plays, the private bytes go to the log once a minute (`tick`, from the bar's
//! clock, so nothing runs for it while paused).

use std::cell::Cell;
use std::time::{Duration, Instant};

use i_slint_core::window::WindowInner;
use slint::ComponentHandle;

use crate::app::Ctx;

/// How often `tick` logs.
const EVERY: Duration = Duration::from_secs(60);

thread_local! {
    /// The window shows nothing (minimised or in the tray).
    static HIDDEN: Cell<bool> = const { Cell::new(false) };
    /// When `tick` last logged.
    static LOGGED: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// This process's private bytes (what Task Manager calls the commit size).
pub fn private_bytes() -> u64 {
    #[repr(C)]
    #[derive(Default)]
    struct Counters {
        cb: u32,
        page_faults: u32,
        peak_working_set: usize,
        working_set: usize,
        quota_peak_paged: usize,
        quota_paged: usize,
        quota_peak_nonpaged: usize,
        quota_nonpaged: usize,
        pagefile: usize,
        peak_pagefile: usize,
        private: usize,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcess() -> isize;
        fn K32GetProcessMemoryInfo(process: isize, counters: *mut Counters, cb: u32) -> i32;
    }
    let mut c = Counters {
        cb: std::mem::size_of::<Counters>() as u32,
        ..Default::default()
    };
    // SAFETY: this process's pseudo-handle and a counters struct of the size it says.
    let ok = unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) };
    if ok == 0 { 0 } else { c.private as u64 }
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
        compact_heaps();
        log!(
            "window shows nothing: private {:.0} MB, {:.0} MB after letting go",
            mb(before),
            mb(private_bytes())
        );
    } else {
        ctx.covers.show(Some(ctx.current_page().kind()));
        if let Some(ui) = ctx.window() {
            resume_drawing(ui.window());
        }
    }
}

/// Whether the window shows nothing (`hidden`).
pub fn is_hidden() -> bool {
    HIDDEN.get()
}

/// Hands the heaps' free pages back to Windows. What was just let go is mostly free blocks
/// inside the heaps (a cover is under the heap's 512 KB threshold for a block of its own), and
/// a heap keeps free pages committed, so they'd still count as private memory.
pub(crate) fn compact_heaps() {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetProcessHeaps(count: u32, heaps: *mut isize) -> u32;
        fn HeapCompact(heap: isize, flags: u32) -> usize;
    }
    let mut heaps = [0isize; 32];
    // SAFETY: a buffer of `heaps.len()` handles; the call fills at most that many.
    let n = unsafe { GetProcessHeaps(heaps.len() as u32, heaps.as_mut_ptr()) } as usize;
    for &heap in &heaps[..n.min(heaps.len())] {
        // SAFETY: a heap of this process, from GetProcessHeaps.
        unsafe { HeapCompact(heap, 0) };
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

/// Called each second of playback (the bar's clock): logs the private bytes once a minute.
pub fn tick(ctx: &Ctx) {
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
