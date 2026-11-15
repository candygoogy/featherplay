//! Helpers several test modules share: a fresh home for every test, generated covers for the
//! fixtures and long lyrics, the long Home and long lists, the scene at 4K, frames on the
//! frozen clock timed in this thread's CPU time, the pointer, the window's elements by their
//! accessible label, pixel comparisons, a bare window and its `Ctx`, the ceilings' three
//! tries, and private memory.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Duration;

use featherplay::app::Ctx;
use featherplay::controller::Action;
use featherplay::covers::Covers;
use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{
    AppWindow, CardItem, Frame, HomeData, HomeShelfRow, LyricsData, Theme, TrackRow,
};
use featherplay_controller::catalog::Catalog;
use featherplay_controller::fixtures::Fixtures;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{
    ComponentHandle, LogicalPosition, LogicalSize, Model, ModelRc, PhysicalSize, VecModel,
};
use tokio::sync::mpsc;

/// One frame at 240 Hz (Slint's clock counts whole milliseconds).
pub const FRAME_240: Duration = Duration::from_millis(4);
/// One frame at 60 Hz.
pub const FRAME_60: Duration = Duration::from_millis(16);
/// The generated covers: how many, and their side in pixels (the CDN's card size).
const COVERS: usize = 40;
const COVER_SIDE: u32 = 544;

/// Every test starts in a fresh, empty home of its own, whatever `FEATHERPLAY_HOME` the run
/// was given: the settings a hand-tested home had saved (tooltips off) failed three tests.
/// This is the one place: nextest runs each test in its own process, and the C runtime
/// calls `fresh_home` as the process starts, before `main` (the MSVC runtime's `.CRT$XCU`
/// initialisers on Windows, `.init_array` on Linux). A test that wants something in its home
/// sets its own after that (`home_with_covers`, `mini_player`).
#[used]
#[cfg_attr(windows, unsafe(link_section = ".CRT$XCU"))]
#[cfg_attr(not(windows), unsafe(link_section = ".init_array"))]
static FRESH_HOME: extern "C" fn() = fresh_home;

/// The home is under the tests' temp folder, named after the process, and removed as the
/// process ends. A failed test's stays, with what the test saved: libtest then exits without
/// running the exit handlers.
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
    // A wait that runs out of time fails the test instead of passing a minute late.
    featherplay::snapshot::settle_must_finish();
    // SAFETY: before `main`, on the process's only thread; `remove` is a plain function.
    unsafe {
        std::env::set_var("FEATHERPLAY_HOME", &home);
        atexit(remove);
    }
}

fn own_home() -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("home-{}", std::process::id()))
}

/// Pixel checks whose numbers come from text drawn in the body font: Segoe UI Variable on
/// Windows, another font on Linux, where a row's height or a label's ink lands a few pixels
/// elsewhere.
/// With `FEATHERPLAY_PIXELS=unchecked` (CI's Linux job) a test that measures text still
/// renders every state and saves it (`save`), and skips those checks. Windows always checks.
pub mod pixels {
    use slint::{Rgba8Pixel, SharedPixelBuffer};

    pub fn checked() -> bool {
        cfg!(windows) || std::env::var_os("FEATHERPLAY_PIXELS").is_none_or(|v| v != "unchecked")
    }

    /// With `FEATHERPLAY_TEST_SHOTS=<dir>`, writes `shot` there as `<test>-<name>.png`, the
    /// test named after its module.
    pub fn save(name: &str, shot: &SharedPixelBuffer<Rgba8Pixel>) {
        let Some(dir) = std::env::var_os("FEATHERPLAY_TEST_SHOTS") else {
            return;
        };
        let test = std::thread::current()
            .name()
            .and_then(|n| n.split("::").next())
            .unwrap_or("test")
            .to_owned();
        let img = image::RgbaImage::from_raw(shot.width(), shot.height(), shot.as_bytes().to_vec())
            .expect("a whole image");
        let path = std::path::Path::new(&dir).join(format!("{test}-{name}.png"));
        img.save(&path).expect("test shot");
    }
}

/// This thread's CPU time, from Windows' per-thread cycle counter.
pub mod cpu {
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    /// Cycles this thread has run.
    pub fn cycles() -> u64 {
        featherplay_platform::thread::cycles().unwrap_or(0)
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
    featherplay_platform::memory::private_bytes() as f64 / 1024.0 / 1024.0
}

pub fn quantile(v: &[f64], q: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mut v = v.to_vec();
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * q).round() as usize]
}

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

/// Noisy images (as costly to draw as photos), made once, as `FEATHERPLAY_FIXTURE_COVERS` for
/// the scene unless it's set: every fixture thumbnail then has art. `SMOOTHNESS_COVERS` names
/// a folder of real covers instead.
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

/// An LRCLIB record as long as a real song's, as `FEATHERPLAY_FIXTURE_LYRICS`: 80 timed lines,
/// a verse break every eight, about one in four long enough to wrap in the full view. All of
/// them start before the fixture's position (2:41), so the app arms no line timer: a test
/// moves `current` itself, as `lyrics::sync` does. Call it before the scene starts.
pub fn long_lyrics() {
    const WORDS: [&str; 24] = [
        "light", "harbour", "morning", "never", "walking", "through", "the", "city", "again",
        "under", "neon", "rain", "and", "your", "voice", "is", "calling", "me", "back", "home",
        "tonight", "forever", "slowly", "burning",
    ];
    let mut seed: u32 = 0x2545_F491;
    let mut next = |n: u32| {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed % n
    };
    let mut synced = String::new();
    let mut plain = String::new();
    for i in 0..80u32 {
        let t = 10.0 + f64::from(i) * 1.8;
        let text = if i % 8 == 7 {
            String::new()
        } else {
            let words = if next(4) == 0 {
                12 + next(4)
            } else {
                4 + next(4)
            };
            let line: Vec<&str> = (0..words).map(|_| WORDS[next(24) as usize]).collect();
            let line = line.join(" ");
            line[..1].to_uppercase() + &line[1..]
        };
        synced.push_str(&format!(
            "[{:02}:{:05.2}] {text}\n",
            (t / 60.0) as u32,
            t % 60.0
        ));
        plain.push_str(&text);
        plain.push('\n');
    }
    let record = serde_json::json!({
        "id": 1, "name": "Long", "trackName": "Long", "artistName": "Fixture",
        "albumName": "Fixture", "duration": 367.0, "instrumental": false,
        "plainLyrics": plain, "syncedLyrics": synced,
    });
    let file = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("lyrics-long.json");
    std::fs::write(&file, record.to_string()).expect("lyrics file");
    // SAFETY: set before the scene starts any other thread.
    unsafe { std::env::set_var("FEATHERPLAY_FIXTURE_LYRICS", file) };
}

/// Resizes the window to `width`×`height` physical pixels at its scale, as the system does.
pub fn resize(w: &slint::Window, width: u32, height: u32) {
    w.set_size(PhysicalSize::new(width, height));
    let scale = w.scale_factor();
    w.dispatch_event(WindowEvent::Resized {
        size: LogicalSize::new(width as f32 / scale, height as f32 / scale),
    });
}

/// The scene on the fixtures with art on every card, `page` open, presenting at
/// `width`×`height` physical pixels at `scale` with Up next open, motion on, the clock frozen;
/// the fixture radio's song playing with sound (`sound`: the marks step, the bar ticks) or
/// playing as the controller reports it, silent. Covers are decoded for `scale`, set before
/// the page loads, as the app does.
pub fn scene_at(width: u32, height: u32, scale: f32, page: &str, sound: bool) -> Scene {
    fixture_covers();
    let scene = Scene::new(1280, 800, false, true).expect("scene");
    scene.ui.global::<Theme>().set_motion(true);
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::ScaleFactorChanged {
        scale_factor: scale,
    });
    scene.ctx.covers.set_scale(scale);
    if sound {
        scene.sound(true)
    } else {
        scene.play(false)
    }
    .expect("player");
    scene.open(page).expect("page");
    scene.theme(true);
    scene.ui.show().expect("show");
    scene.settle();
    offscreen::freeze_clock();
    scene.ui.global::<Frame>().set_queue_open(true);
    resize(w, width, height);
    scene.settle();
    frames(w, 60, |_| {});
    scene
}

/// A window and its `Ctx` without a `Scene` around them, for the tests that play the
/// controller's part themselves: the offscreen platform, a runtime, a catalogue with no
/// source yet, covers cached in `cache` (from `fixtures`' art, when given) and the actions
/// the window sends. Nothing is wired. The fields drop in order: the window before the
/// runtime and the platform.
pub struct Bare {
    pub ctx: Rc<Ctx>,
    pub ui: AppWindow,
    pub catalog: Catalog,
    pub sent: mpsc::UnboundedReceiver<Action>,
    pub runtime: tokio::runtime::Runtime,
    pub pump: offscreen::Pump,
}

pub fn bare(cache: PathBuf, fixtures: Option<Arc<Fixtures>>) -> Bare {
    let pump = offscreen::install().expect("platform");
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("runtime");
    let catalog = Catalog::new(runtime.handle().clone()).expect("catalog");
    let covers = Covers::new(runtime.handle().clone(), cache, fixtures);
    let (actions, sent) = mpsc::unbounded_channel();
    let ui = AppWindow::new().expect("window");
    let ctx = Ctx::new(&ui, catalog.clone(), covers, actions);
    Bare {
        ctx,
        ui,
        catalog,
        sent,
        runtime,
        pump,
    }
}

/// `scene_at` on Home, silent.
pub fn home_at(width: u32, height: u32, scale: f32) -> Scene {
    scene_at(width, height, scale, "home", false)
}

/// The user's conditions at 4K 150 %: Home, playing with sound, art on every cover, the long
/// lyrics, Up next open; the frames after it settled run.
pub fn playing_4k() -> Scene {
    long_lyrics();
    let scene = scene_at(3840, 2160, 1.5, "home", true);
    idle(&scene, 100);
    scene
}

/// Opens `page` and lets it settle and its entry run.
pub fn show(scene: &Scene, page: &str) {
    scene.open(page).expect("page");
    scene.settle();
    idle(scene, 120);
}

/// One frame's CPU time (ms) and the physical pixels it copied to the window.
#[derive(Clone, Copy)]
pub struct Frame1 {
    pub cpu: f64,
    pub damaged: u64,
}

/// One turn of the event loop on the frozen clock at 240 Hz: the clock moves on (timers and
/// animations), the answers that have arrived run, then the render and the copy, in this
/// thread's CPU time.
pub fn step(scene: &Scene) -> Frame1 {
    let c = cpu::cycles();
    offscreen::advance(FRAME_240);
    scene.run_queued();
    let p = offscreen::present(scene.ui.window()).expect("present");
    Frame1 {
        cpu: cpu::ms(cpu::cycles() - c),
        damaged: p.damaged,
    }
}

/// Frames until nothing loads: the wait on a frozen clock for a load that hangs on a timer.
/// The lyrics are looked up a second after they're asked for and their lines come a few a
/// frame, and `Scene::settle` alone moves no clock, so it would wait its whole ceiling.
pub fn settle_in_frames(scene: &Scene) {
    for _ in 0..2000 {
        scene.settle_once();
        frames(scene.ui.window(), 1, |_| {});
        if featherplay::busy::pending() == 0 {
            return;
        }
    }
    panic!(
        "{} loads still pending after 2000 frames",
        featherplay::busy::pending()
    );
}

/// `n` frames of `step`.
pub fn idle(scene: &Scene, n: usize) {
    for _ in 0..n {
        step(scene);
    }
}

/// Presents a frame and returns what the window then shows.
pub fn frame(scene: &Scene) -> image::RgbaImage {
    let w = scene.ui.window();
    offscreen::present(w).expect("present");
    let px = offscreen::shown(w).expect("shown");
    image::RgbaImage::from_raw(px.width(), px.height(), px.as_bytes().to_vec()).expect("image")
}

/// How far apart two pixels' colours are: the sum of the red, green and blue differences.
pub fn dist(a: [u8; 4], b: [u8; 4]) -> u32 {
    (0..3).map(|i| u32::from(a[i].abs_diff(b[i]))).sum()
}

/// Whether two pixels are different colours altogether, as text is from its ground.
pub fn far(a: [u8; 4], b: [u8; 4]) -> bool {
    dist(a, b) > 120
}

/// Moves the frozen clock on by `ms`, a frame every 16 ms rendered as the window would: for
/// the tests that take snapshots rather than present.
pub fn run_for(w: &slint::Window, ms: u64) {
    for _ in 0..ms.div_ceil(16) {
        offscreen::advance(FRAME_60);
        w.take_snapshot().expect("snapshot");
    }
}

/// Runs `check` until a run's ceilings hold, three runs at most, and fails with the last
/// run's excess: CPU time per frame is noisy while other work loads the machine. `check` gets
/// the run's number, asserts what holds in any build itself and returns the ceilings' excess,
/// which only an optimised build is held to, so a debug build makes one run.
pub fn three_tries(mut check: impl FnMut(u32) -> Result<(), String>) {
    let mut last = String::new();
    for attempt in 0..3 {
        match check(attempt) {
            Err(over) if !cfg!(debug_assertions) => last = over,
            _ => return,
        }
    }
    panic!("{last}");
}

/// The window's size in logical pixels.
fn logical(scene: &Scene) -> LogicalSize {
    let w = scene.ui.window();
    w.size().to_logical(w.scale_factor())
}

/// What the pointer acts on: a scene, or a bare window (`AppWindow::window()`).
pub trait OnWindow {
    fn on_window(&self) -> &slint::Window;
}

impl OnWindow for Scene {
    fn on_window(&self) -> &slint::Window {
        self.ui.window()
    }
}

impl OnWindow for slint::Window {
    fn on_window(&self) -> &slint::Window {
        self
    }
}

/// The pointer moves to `at`.
pub fn point(on: &impl OnWindow, at: LogicalPosition) {
    on.on_window()
        .dispatch_event(WindowEvent::PointerMoved { position: at });
}

/// The pointer moves to `at` and presses the left button.
pub fn press(on: &impl OnWindow, at: LogicalPosition) {
    point(on, at);
    on.on_window().dispatch_event(WindowEvent::PointerPressed {
        position: at,
        button: PointerEventButton::Left,
    });
}

/// The left button lets go at `at`.
pub fn release(on: &impl OnWindow, at: LogicalPosition) {
    on.on_window().dispatch_event(WindowEvent::PointerReleased {
        position: at,
        button: PointerEventButton::Left,
    });
}

/// A left click at `at`: the pointer moves there, presses and lets go.
pub fn click(on: &impl OnWindow, at: LogicalPosition) {
    click_with(on, at, PointerEventButton::Left);
}

/// A click of `button` at `at`: the pointer moves there, presses and lets go.
pub fn click_with(on: &impl OnWindow, at: LogicalPosition, button: PointerEventButton) {
    let w = on.on_window();
    w.dispatch_event(WindowEvent::PointerMoved { position: at });
    w.dispatch_event(WindowEvent::PointerPressed {
        position: at,
        button,
    });
    w.dispatch_event(WindowEvent::PointerReleased {
        position: at,
        button,
    });
}

/// Somewhere with nothing under it that reacts: the top bar's empty middle.
pub fn away(scene: &Scene) -> LogicalPosition {
    LogicalPosition::new(logical(scene).width * 0.45, 30.0)
}

/// The bar's vertical middle.
pub fn bar_y(scene: &Scene) -> f32 {
    let t = scene.ui.global::<Theme>();
    logical(scene).height - t.get_bar_inset() - t.get_bar_h() / 2.0
}

/// The bar's microphone, found by clicking along the bar on Now playing: where a click
/// turns the lyrics on (each hit turned off again).
pub fn lyrics_button(scene: &Scene) -> LogicalPosition {
    let data = scene.ui.global::<LyricsData>();
    let (width, y) = (logical(scene).width, bar_y(scene));
    let mut hits = Vec::new();
    for x in (0..400).step_by(3) {
        let at = LogicalPosition::new(width - 400.0 + x as f32, y);
        click(scene, at);
        if data.get_full() {
            hits.push(at.x);
            data.set_full(false);
        }
        point(scene, away(scene));
        idle(scene, 2);
    }
    assert!(!hits.is_empty(), "the bar's lyrics button not found");
    idle(scene, 200);
    LogicalPosition::new((hits[0] + hits[hits.len() - 1]) / 2.0, y)
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

/// The bar's volume slider, as left, top, right and bottom edges: the one visible "Volume"
/// slider (found by its label: the wheel no longer tells, since all the bar takes it).
pub fn volume_slider(scene: &Scene) -> [f32; 4] {
    use i_slint_core::items::AccessibleRole;
    let slider = label::shown_where(scene, |item, l| {
        l == "Volume" && item.accessible_role() == AccessibleRole::Slider
    });
    let r = label::rect(slider.first().expect("no volume slider"));
    [r.min_x(), r.min_y(), r.max_x(), r.max_y()]
}

/// The window's elements found by their accessible label, as a screen reader finds them (a
/// `Text` is labelled with its text).
pub mod label {
    use std::ops::ControlFlow;

    use featherplay::snapshot::Scene;
    use i_slint_core::accessibility::{AccessibilityAction, AccessibleStringProperty};
    pub use i_slint_core::item_tree::ItemRc;
    pub use i_slint_core::lengths::LogicalRect;
    use i_slint_core::window::WindowInner;
    use slint::LogicalPosition;

    use super::OnWindow;

    /// Every element, shown or hidden, whose label passes `keep`, in the tree's order.
    pub fn any_where(
        on: &impl OnWindow,
        mut keep: impl FnMut(&ItemRc, &str) -> bool,
    ) -> Vec<ItemRc> {
        let root = ItemRc::new_root(WindowInner::from_pub(on.on_window()).component());
        let mut found = Vec::new();
        root.visit_descendants(|item| {
            if item
                .accessible_string_property(AccessibleStringProperty::Label)
                .is_some_and(|l| keep(item, &l))
            {
                found.push(item.clone());
            }
            ControlFlow::<()>::Continue(())
        });
        found
    }

    /// The shown elements whose label passes `keep`.
    pub fn shown_where(
        on: &impl OnWindow,
        mut keep: impl FnMut(&ItemRc, &str) -> bool,
    ) -> Vec<ItemRc> {
        any_where(on, |item, l| item.is_visible() && keep(item, l))
    }

    /// The shown elements labelled `label`.
    pub fn all(on: &impl OnWindow, label: &str) -> Vec<ItemRc> {
        shown_where(on, |_, l| l == label)
    }

    /// The first shown element labelled `label`.
    pub fn find(on: &impl OnWindow, label: &str) -> Option<ItemRc> {
        all(on, label).into_iter().next()
    }

    /// Whether a shown element is labelled `label`.
    pub fn shows(on: &impl OnWindow, label: &str) -> bool {
        find(on, label).is_some()
    }

    /// Where `item` is, in window coordinates.
    pub fn rect(item: &ItemRc) -> LogicalRect {
        let g = item.geometry();
        LogicalRect::new(item.map_to_window(g.origin), g.size)
    }

    /// Where the shown elements labelled `label` are, in window coordinates.
    pub fn rects(on: &impl OnWindow, label: &str) -> Vec<LogicalRect> {
        all(on, label).iter().map(rect).collect()
    }

    /// The middle of the first shown element labelled `label`.
    pub fn centre(on: &impl OnWindow, label: &str) -> LogicalPosition {
        let item = find(on, label).unwrap_or_else(|| panic!("nothing visible says {label:?}"));
        let c = rect(&item).center();
        LogicalPosition::new(c.x, c.y)
    }

    /// A left click in the middle of the first shown element labelled `label`, as the user
    /// makes it.
    pub fn click(on: &impl OnWindow, label: &str) {
        super::click(on, centre(on, label));
    }

    /// Presses the first shown element labelled `label`, as a screen reader does.
    pub fn act(on: &impl OnWindow, label: &str) {
        find(on, label)
            .unwrap_or_else(|| panic!("nothing visible says {label:?}"))
            .accessible_action(&AccessibilityAction::Default);
    }

    /// `act`, then lets what it starts settle.
    pub fn press(scene: &Scene, label: &str) {
        act(scene, label);
        scene.settle();
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
    click(ui.window(), at);
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
