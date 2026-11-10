//! What the window shows, through the renderer's own frames as the app's window presents
//! them (`offscreen::install_presenting`: partial rendering into a buffer that behaves like
//! softbuffer's on Windows, only the damage copied to the window). No window is shown.
//!
//! The window must never show through: every pixel opaque on every frame, during motion
//! and scrolling, maximized at 1080p and at 1440p with 150 % scaling, and again after
//! Windows has dropped the window's surface (a restore from minimized).

use std::time::Duration;

use featherplay::offscreen;
use featherplay::repaint;
use featherplay::snapshot::Scene;
use featherplay::ui::{Frame, Theme};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition, LogicalSize, PhysicalSize, Rgba8Pixel};
use slint::{SharedPixelBuffer, Window};

/// One frame at 60 Hz.
const FRAME: Duration = Duration::from_millis(16);

/// Pixels the window shows with alpha below 255.
fn see_through(shot: &SharedPixelBuffer<Rgba8Pixel>) -> usize {
    shot.as_slice().iter().filter(|p| p.a < 255).count()
}

/// Steps one frame: the clock, then the renderer's frame and its present.
fn frame(w: &Window) -> offscreen::Presented {
    offscreen::advance(FRAME);
    offscreen::present(w).expect("present")
}

/// `n` frames; each one must be opaque.
fn opaque_frames(w: &Window, n: u32, what: &str, mut each: impl FnMut(u32)) {
    for i in 0..n {
        each(i);
        frame(w);
        let shown = offscreen::shown(w).expect("shown");
        let holes = see_through(&shown);
        assert_eq!(holes, 0, "{what}, frame {i}: {holes} pixels show through");
    }
}

/// At rest, the window shows what a full render of the scene shows. Anti-aliased edges
/// may land a few levels apart (along a frame's dirty rectangles, or a vector glyph at
/// 150 %); stale or missing paint is a different colour altogether.
fn matches_full_render(w: &Window, what: &str) {
    let shown = offscreen::shown(w).expect("shown");
    let full = w.take_snapshot().expect("snapshot");
    let near = |a: u8, b: u8| a.abs_diff(b) <= 40;
    let differ = shown
        .as_slice()
        .iter()
        .zip(full.as_slice())
        .filter(|(a, b)| !(near(a.r, b.r) && near(a.g, b.g) && near(a.b, b.b) && a.a == b.a))
        .count();
    assert_eq!(
        differ, 0,
        "{what}: {differ} pixels differ from a full render"
    );
}

fn resize(w: &Window, width: u32, height: u32) {
    w.set_size(PhysicalSize::new(width, height));
    let scale = w.scale_factor();
    w.dispatch_event(WindowEvent::Resized {
        size: LogicalSize::new(width as f32 / scale, height as f32 / scale),
    });
}

fn scroll(w: &Window, at: LogicalPosition) {
    w.dispatch_event(WindowEvent::PointerScrolled {
        position: at,
        delta_x: 0.0,
        delta_y: -60.0,
    });
}

/// A page enter, then a scroll down its list, every frame checked; settled, it matches a
/// full render.
fn enter_and_scroll(scene: &Scene, page: &str, at: LogicalPosition, what: &str) {
    let w = scene.ui.window();
    scene.open(page).expect("page");
    scene.settle();
    opaque_frames(w, 20, &format!("{what}: entering {page}"), |_| {});
    w.dispatch_event(WindowEvent::PointerMoved { position: at });
    opaque_frames(w, 40, &format!("{what}: scrolling {page}"), |i| {
        if i < 20 {
            scroll(w, at);
        }
    });
    matches_full_render(w, &format!("{what}: {page} after scrolling"));
}

/// One test: the platform can be installed once per process.
#[test]
fn the_window_never_shows_through() {
    let scene = Scene::new(1280, 800, false, true).expect("scene");
    scene.ui.global::<Theme>().set_motion(true);
    scene.play(false).expect("player");
    scene.open("home").expect("home");
    scene.theme(true);
    scene.ui.show().expect("show");
    scene.settle();
    offscreen::freeze_clock();
    let w = scene.ui.window();
    opaque_frames(w, 10, "first frames", |_| {});

    // Maximized: the page is laid out for the new size at once, not eased there frame by
    // frame (only the Up next push eases).
    resize(w, 1920, 1080);
    opaque_frames(w, 1, "maximizing", |_| {});
    assert!(
        !w.has_active_animations(),
        "a maximize started an animation"
    );
    assert_eq!(
        frame(w).damaged,
        0,
        "the frame after a maximize still repainted"
    );
    matches_full_render(w, "maximized");
    scene.ui.global::<Frame>().set_queue_open(true);
    opaque_frames(w, 3, "opening Up next", |_| {});
    assert!(w.has_active_animations(), "Up next opened without its push");
    opaque_frames(w, 30, "opening Up next", |_| {});
    scene.ui.global::<Frame>().set_queue_open(false);
    opaque_frames(w, 30, "closing Up next", |_| {});
    matches_full_render(w, "Up next closed");

    // Restored from minimized: Windows has a new surface for the window at the same size
    // (Slint ignores the 0×0 in between), and one thing changes, the hovered card.
    let hover = LogicalPosition::new(700.0, 400.0);
    offscreen::lose_surface(w).expect("lose");
    w.dispatch_event(WindowEvent::PointerMoved { position: hover });
    frame(w);
    let unpainted = see_through(&offscreen::shown(w).expect("shown"));
    assert!(
        unpainted > 1920 * 1080 / 2,
        "the model lost its point: only {unpainted} pixels unpainted after a restore"
    );
    // What the app does on that WM_PAINT: the whole window is painted and presented.
    offscreen::lose_surface(w).expect("lose");
    repaint::all(w);
    opaque_frames(w, 1, "restored", |_| {});
    matches_full_render(w, "restored");

    let list = LogicalPosition::new(1000.0, 500.0);
    for page in ["library", "home", "playlist"] {
        enter_and_scroll(&scene, page, list, "1080p");
    }

    // 2560×1440 at 150 %, as a maximized window on a scaled display.
    w.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 1.5 });
    resize(w, 2560, 1440);
    opaque_frames(w, 30, "1440p at 150 %", |_| {});
    matches_full_render(w, "1440p at 150 %");
    for page in ["library", "playlist"] {
        enter_and_scroll(&scene, page, list, "1440p at 150 %");
    }
}

/// While the playing mark steps (Up next's playing row), frames repaint small spots, and the
/// renderer leaves out the opacity layers at opacity 0 (its change 19): the rows' hover
/// buttons, tooltips and menus waiting to fade in. After each run of such frames (Up next's
/// contents fading in and out, a row's buttons fading in, out and in again) the window shows
/// what a full render does. Nothing renders the whole window in between, as nothing does in
/// the app.
#[test]
fn frames_while_the_playing_mark_steps_match_a_full_render() {
    let scene = Scene::new(1280, 800, false, true).expect("scene");
    scene.ui.global::<Theme>().set_motion(true);
    scene.play(false).expect("player");
    scene.open("playlist").expect("playlist");
    scene.theme(true);
    scene.ui.show().expect("show");
    scene.settle();
    offscreen::freeze_clock();
    let w = scene.ui.window();
    // The pixels the frames copied to the window.
    let frames = |n: u32| (0..n).map(|_| frame(w).damaged).sum::<u64>();
    let queue = |open: bool| scene.ui.global::<Frame>().set_queue_open(open);
    let point = |y: f32| {
        w.dispatch_event(WindowEvent::PointerMoved {
            position: LogicalPosition::new(600.0, y),
        })
    };
    point(20.0);
    queue(true);
    frames(30);
    matches_full_render(w, "Up next opened");
    let marked = frames(16);
    assert!(
        marked > 0,
        "the playing mark didn't step: the test lost its point"
    );
    for (y, what) in [(420.0, "a row hovered"), (470.0, "the next row hovered")] {
        point(y);
        let hovered = frames(16);
        assert!(
            hovered > marked,
            "{what}: nothing faded, the test lost its point"
        );
        matches_full_render(w, what);
    }
    point(20.0);
    frames(16);
    matches_full_render(w, "the rows left");
    // The first row's buttons have waited at opacity 0 through those frames.
    frames(16);
    point(420.0);
    frames(16);
    matches_full_render(w, "the row hovered again");
    point(20.0);
    queue(false);
    frames(30);
    matches_full_render(w, "Up next closed");
}
