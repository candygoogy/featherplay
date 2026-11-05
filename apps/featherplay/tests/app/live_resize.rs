//! A live resize (the user dragging the window's edge: between WM_ENTERSIZEMOVE and
//! WM_EXITSIZEMOVE) keeps the window's pixel buffer rather than making a new one for every
//! step. softbuffer's Win32 surface made a new DIB section for each new size, 33 MB at 4K that
//! Windows hands over zeroed a page at a time, and that was two thirds of a step's cost: the
//! window trailed the pointer (`resize_frames.rs`). During the drag the buffer only grows,
//! with room to spare, and each frame paints the window's part of it; once the drag ends it's
//! the window's size again, so a window at rest holds no more memory than before.
//! Through the renderer's own frames (`offscreen::install_presenting`); no window is shown.

use std::time::Duration;

use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::Theme;
use i_slint_renderer_skia::software_surface::buffer_size;
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalSize, PhysicalSize};

#[test]
fn outside_a_live_resize_the_buffer_is_the_windows_size() {
    assert_eq!(buffer_size((0, 0), (1280, 800), false), (1280, 800));
    assert_eq!(buffer_size((3840, 2160), (1280, 800), false), (1280, 800));
    assert_eq!(buffer_size((1000, 700), (1280, 800), false), (1280, 800));
}

#[test]
fn during_a_live_resize_a_buffer_that_fits_is_kept() {
    assert_eq!(buffer_size((2000, 1200), (1900, 1100), true), (2000, 1200));
    assert_eq!(buffer_size((2000, 1200), (2000, 1200), true), (2000, 1200));
}

#[test]
fn during_a_live_resize_a_buffer_grows_with_room_to_spare() {
    let (w, h) = buffer_size((2000, 1200), (2010, 1100), true);
    assert!(
        w >= 2010 + 2010 / 8,
        "{w} leaves no room for the next steps"
    );
    assert_eq!(h, 1200, "the height that fit changed");
    let (w, h) = buffer_size((0, 0), (1280, 800), true);
    assert!(
        w > 1280 && h > 800,
        "a first buffer mid-drag has no room: {w}×{h}"
    );
}

fn resize(w: &slint::Window, width: u32, height: u32) {
    w.set_size(PhysicalSize::new(width, height));
    w.dispatch_event(WindowEvent::Resized {
        size: LogicalSize::new(width as f32, height as f32),
    });
}

fn frame(w: &slint::Window) {
    offscreen::advance(Duration::from_millis(16));
    offscreen::present(w).expect("present");
}

/// Pixels the window shows that a full render of the scene doesn't. Anti-aliased edges may
/// land a few levels apart (as in `render_surface.rs`); stale or missing paint is a different
/// colour altogether.
fn stale(w: &slint::Window) -> usize {
    let shown = offscreen::shown(w).expect("shown");
    let full = w.take_snapshot().expect("snapshot");
    assert_eq!(
        (shown.width(), shown.height()),
        (full.width(), full.height()),
        "the window shows another size than it has"
    );
    let near = |a: u8, b: u8| a.abs_diff(b) <= 40;
    shown
        .as_slice()
        .iter()
        .zip(full.as_slice())
        .filter(|(a, b)| !(near(a.r, b.r) && near(a.g, b.g) && near(a.b, b.b) && a.a == b.a))
        .count()
}

/// One test: the platform can be installed once per process.
#[test]
fn a_drag_keeps_its_buffer_and_every_frame_shows_the_whole_window() {
    let scene = Scene::new(1280, 800, false, true).expect("scene");
    scene.ui.global::<Theme>().set_motion(true);
    scene.play(false).expect("player");
    scene.open("home").expect("home");
    scene.theme(true);
    scene.ui.show().expect("show");
    scene.settle();
    offscreen::freeze_clock();
    let w = scene.ui.window();
    for _ in 0..20 {
        frame(w);
    }
    assert_eq!(offscreen::buffer_size(w), (1280, 800));

    featherplay::live_resize::set(w, true);
    let before = offscreen::buffers_made();
    // Wider, then narrower and shorter: a drag that wanders.
    let steps: Vec<(u32, u32)> = (1..=40)
        .map(|i| (1280 + i * 6, 800))
        .chain((1..=40).map(|i| (1520 - i * 8, 800 - i * 3)))
        .collect();
    for &(width, height) in &steps {
        resize(w, width, height);
        frame(w);
        let differ = stale(w);
        assert_eq!(differ, 0, "at {width}×{height}, {differ} pixels are stale");
    }
    let made = offscreen::buffers_made() - before;
    assert!(
        made <= 3,
        "{made} buffers for one drag of {} steps",
        steps.len()
    );

    // The drag ends: the next frame is drawn into a buffer of the window's own size.
    featherplay::live_resize::set(w, false);
    frame(w);
    let (width, height) = *steps.last().expect("steps");
    assert_eq!(offscreen::buffer_size(w), (width, height));
    let differ = stale(w);
    assert_eq!(differ, 0, "after the drag, {differ} pixels are stale");
}
