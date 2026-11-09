//! Shared by the tests that drive a long Home at 4K with art on every card (`home_covers`,
//! `home_shelves_dropped`) and by the smoothness benchmark: generated covers for the fixtures,
//! the long Home, the scene at 4K, frames timed in this thread's CPU time, and private memory.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::Duration;

use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{AppWindow, CardItem, Frame, HomeData, HomeShelfRow, Theme, TrackRow};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{
    ComponentHandle, LogicalPosition, LogicalSize, Model, ModelRc, PhysicalSize, VecModel,
};

/// One frame at 240 Hz (Slint's clock counts whole milliseconds).
pub const FRAME_240: Duration = Duration::from_millis(4);
/// The generated covers: how many, and their side in pixels (the CDN's card size).
const COVERS: usize = 40;
const COVER_SIDE: u32 = 544;

/// Every test starts in a fresh, empty home of its own, whatever `FEATHERPLAY_HOME` the run
/// was given: the settings a hand-tested home had saved (tooltips off) failed three tests.
/// This is the one place: nextest runs each test in its own process, and the C runtime
/// calls `fresh_home` as the process starts, before `main`. A test that wants something in
/// its home sets its own after that (`home_with_covers`, `mini_player`, `startup_session`).
#[used]
#[unsafe(link_section = ".CRT$XCU")]
static FRESH_HOME: extern "C" fn() = fresh_home;

/// The home is under the tests' temp folder, named after the process, and removed as the
/// process ends.
extern "C" fn fresh_home() {
    unsafe extern "C" {
        fn atexit(f: extern "C" fn()) -> i32;
    }
    extern "C" fn remove() {
        let _ = std::fs::remove_dir_all(own_home());
    }
    let home = own_home();
    // Windows hands a process ID out again: one that died may have left its home behind.
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("test home");
    // SAFETY: before `main`, on the process's only thread; `remove` is a plain function.
    unsafe {
        std::env::set_var("FEATHERPLAY_HOME", &home);
        atexit(remove);
    }
}

fn own_home() -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("home-{}", std::process::id()))
}

/// This thread's CPU time, from Windows' per-thread cycle counter.
pub mod cpu {
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThread() -> isize;
        fn QueryThreadCycleTime(thread: isize, cycles: *mut u64) -> i32;
    }

    /// Cycles this thread has run.
    pub fn cycles() -> u64 {
        let mut c = 0;
        // SAFETY: the pseudo-handle of the calling thread and a valid out pointer.
        unsafe { QueryThreadCycleTime(GetCurrentThread(), &mut c) };
        c
    }

    /// Cycles per millisecond: the most a 100 ms spin got (a preempted spin gets fewer).
    pub fn rate() -> f64 {
        static RATE: OnceLock<f64> = OnceLock::new();
        *RATE.get_or_init(|| {
            (0..5)
                .map(|_| {
                    let (t, c) = (Instant::now(), cycles());
                    while t.elapsed() < Duration::from_millis(100) {
                        std::hint::spin_loop();
                    }
                    (cycles() - c) as f64 / (t.elapsed().as_secs_f64() * 1000.0)
                })
                .fold(0.0, f64::max)
        })
    }

    pub fn ms(cycles: u64) -> f64 {
        cycles as f64 / rate()
    }
}

/// This process's private bytes, in MB.
pub fn private_mb() -> f64 {
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
    unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut c, c.cb) };
    c.private as f64 / 1024.0 / 1024.0
}

pub fn quantile(v: &[f64], q: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mut v = v.to_vec();
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * q).round() as usize]
}

/// Noisy images (as costly to draw as photos), made once, as `FEATHERPLAY_FIXTURE_COVERS` for
/// the scene unless it's set: every fixture thumbnail then has art. `SMOOTHNESS_COVERS` names
/// a folder of real covers instead.
/// Points `FEATHERPLAY_HOME` at a home of this test's own whose design-covers folder holds
/// `covers` (a fixture thumbnail URL and its art), as the dev home's downloaded covers would.
/// For tests that read a cover's colour: they don't depend on what a machine has cached, and
/// a CI runner has nothing cached. Call it before the scene loads the fixtures.
pub fn home_with_covers(name: &str, covers: &[(&str, image::RgbImage)]) {
    use sha1::{Digest, Sha1};
    // One per process: nextest runs a file's tests side by side.
    let home =
        Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("{name}-{}", std::process::id()));
    let dir = home.join("design-covers");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("covers dir");
    for (url, img) in covers {
        // The name the fixtures look for: 12 hex digits of the URL's SHA-1.
        let hex: String = Sha1::digest(url.as_bytes())
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        img.save(dir.join(format!("{}.jpg", &hex[..12])))
            .expect("cover");
    }
    // SAFETY: set before the scene starts any other thread.
    unsafe { std::env::set_var("FEATHERPLAY_HOME", &home) };
}

pub fn fixture_covers() {
    if std::env::var_os("FEATHERPLAY_FIXTURE_COVERS").is_some() {
        return;
    }
    let dir = match std::env::var_os("SMOOTHNESS_COVERS") {
        Some(d) => PathBuf::from(d),
        None => {
            let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("smoothness-covers");
            std::fs::create_dir_all(&dir).expect("covers dir");
            let mut seed: u32 = 0x9E37_79B9;
            for n in 0..COVERS {
                let file = dir.join(format!("{n:02}.jpg"));
                if file.is_file() {
                    continue;
                }
                let img = image::RgbImage::from_fn(COVER_SIDE, COVER_SIDE, |x, y| {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    let [a, b, c, _] = seed.to_le_bytes();
                    let base = (x * 255 / COVER_SIDE) as u8 / 2 + (y * 255 / COVER_SIDE) as u8 / 4;
                    image::Rgb([base.wrapping_add(a / 4), b / 2, c])
                });
                img.save(&file).expect("cover");
            }
            dir
        }
    };
    // SAFETY: set before the scene starts any other thread.
    unsafe { std::env::set_var("FEATHERPLAY_FIXTURE_COVERS", dir) };
}

/// Home's shelves repeated to `shelves`, each card shelf filled to `items` cards, all in new
/// models (so the cards are new items).
pub fn long_home(ui: &AppWindow, shelves: usize, items: usize) {
    let d = ui.global::<HomeData>();
    let loaded: Vec<HomeShelfRow> = d.get_shelves().iter().collect();
    assert!(!loaded.is_empty(), "no shelves loaded");
    let many: Vec<HomeShelfRow> = (0..shelves)
        .map(|i| {
            let mut row = loaded[i % loaded.len()].clone();
            let cards: Vec<CardItem> = row.items.iter().collect();
            if !cards.is_empty() {
                let n = cards.len().max(items);
                let filled: Vec<CardItem> =
                    (0..n).map(|k| cards[k % cards.len()].clone()).collect();
                row.items = ModelRc::from(Rc::new(VecModel::from(filled)));
            }
            row
        })
        .collect();
    d.set_shelves(ModelRc::from(Rc::new(VecModel::from(many))));
}

/// The scene on the fixtures with art on every card, playing, Home open, presenting at
/// `width`×`height` physical pixels at `scale` with Up next open, motion on, the clock frozen.
/// Covers are decoded for `scale`, set before Home loads, as the app does.
pub fn home_at(width: u32, height: u32, scale: f32) -> Scene {
    fixture_covers();
    let scene = Scene::new(1280, 800, false, true).expect("scene");
    scene.ui.global::<Theme>().set_motion(true);
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::ScaleFactorChanged {
        scale_factor: scale,
    });
    scene.ctx.covers.set_scale(scale);
    scene.play(false).expect("player");
    scene.open("home").expect("home");
    scene.theme(true);
    scene.ui.show().expect("show");
    scene.settle();
    offscreen::freeze_clock();
    scene.ui.global::<Frame>().set_queue_open(true);
    w.set_size(PhysicalSize::new(width, height));
    w.dispatch_event(WindowEvent::Resized {
        size: LogicalSize::new(width as f32 / scale, height as f32 / scale),
    });
    scene.settle();
    frames(w, 60, |_| {});
    scene
}

/// One frame's CPU time (ms) and the physical pixels it copied to the window.
pub struct Frame1 {
    pub cpu: f64,
    pub damaged: u64,
}

/// `n` frames of 4 ms; `each` runs first in each one (input).
pub fn frames(w: &slint::Window, n: u32, mut each: impl FnMut(u32)) -> Vec<Frame1> {
    (0..n)
        .map(|i| {
            let c = cpu::cycles();
            each(i);
            offscreen::advance(FRAME_240);
            let presented = offscreen::present(w).expect("present");
            Frame1 {
                cpu: cpu::ms(cpu::cycles() - c),
                damaged: presented.damaged,
            }
        })
        .collect()
}

/// A wheel run at `at`: `events` events of `notches` every 8 frames, then the ease settling.
pub fn wheel_run(w: &slint::Window, at: LogicalPosition, events: u32, notches: f32) -> Vec<Frame1> {
    w.dispatch_event(WindowEvent::PointerMoved { position: at });
    frames(w, events * 8 + 60, |i| {
        if i < events * 8 && i % 8 == 0 {
            offscreen::wheel(w, at, notches);
        }
    })
}

/// Down to the end of the page and back to the top, so every row has been drawn.
pub fn sweep(w: &slint::Window, at: LogicalPosition) {
    for notches in [-16.0, 16.0] {
        let _ = wheel_run(w, at, 20, notches);
    }
}

/// The side panel at the right of a `width`-logical-px window: its left edge and width.
pub fn panel(ui: &AppWindow, width: f32) -> (f32, f32) {
    let side = ui.global::<Theme>().get_nowplaying_side_w();
    (width - side, side)
}

/// The side panel's pill tabs (`SideTabs`): their row's top and height, logical px.
pub const TABS_Y: f32 = 12.0;
pub const TABS_H: f32 = 40.0;

/// Clicks tab `i` of the side panel's pill tabs, as the user does.
pub fn click_tab(ui: &AppWindow, width: f32, i: usize) {
    let (x0, side) = panel(ui, width);
    let s5 = ui.global::<Theme>().get_s5();
    let seg = (side - 2.0 * s5) / 3.0;
    let at = LogicalPosition::new(x0 + s5 + seg * (i as f32 + 0.5), TABS_Y + TABS_H / 2.0);
    let w = ui.window();
    w.dispatch_event(WindowEvent::PointerMoved { position: at });
    w.dispatch_event(WindowEvent::PointerPressed {
        position: at,
        button: PointerEventButton::Left,
    });
    w.dispatch_event(WindowEvent::PointerReleased {
        position: at,
        button: PointerEventButton::Left,
    });
}

/// `rows` repeated to `n` rows, in a new model; only the first copy keeps its playing mark.
pub fn long_rows(rows: &[TrackRow], n: usize) -> ModelRc<TrackRow> {
    let many: Vec<TrackRow> = (0..n)
        .map(|i| {
            let mut row = rows[i % rows.len()].clone();
            row.playing &= i < rows.len();
            row
        })
        .collect();
    ModelRc::from(Rc::new(VecModel::from(many)))
}
