//! The loading skeleton's sweep stays inside the placeholder shapes (offscreen platform, no
//! window shown, motion on, a frozen clock stepped one frame at a time). The page's empty
//! margin keeps the ground's colour on every frame of a full sweep, while the shapes
//! themselves do light up as it passes.

use std::time::Duration;

use featherplay::offscreen;
use featherplay::ui::{AppWindow, LibraryData, Nav, PageKind, Theme};
use slint::{ComponentHandle, Rgba8Pixel, SharedPixelBuffer};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;
/// One frame at 30 Hz: enough to catch a band that is hundreds of pixels wide.
const STEP: Duration = Duration::from_millis(33);

/// The pixels of a strip: `x0..x1` by `y0..y1`, in physical pixels (scale 1).
fn strip(
    shot: &SharedPixelBuffer<Rgba8Pixel>,
    (x0, x1): (u32, u32),
    (y0, y1): (u32, u32),
) -> Vec<Rgba8Pixel> {
    let w = shot.width() as usize;
    let px = shot.as_slice();
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| px[y as usize * w + x as usize]))
        .collect()
}

/// One test: the platform can be installed once per process.
#[test]
fn the_sweep_never_shows_outside_the_placeholders() {
    let _pump = offscreen::install().expect("platform");
    let ui = AppWindow::new().expect("window");
    ui.window()
        .set_size(slint::PhysicalSize::new(WIDTH, HEIGHT));
    ui.global::<Theme>().set_motion(true);
    ui.show().expect("show");
    offscreen::freeze_clock();

    // Library > Liked, still loading: the song-row skeleton.
    let library = ui.global::<LibraryData>();
    library.set_tab(0);
    library.set_loading(true);
    ui.global::<Nav>().set_page(PageKind::Library);
    // Past the skeleton's delay and its fade in.
    for _ in 0..20 {
        offscreen::advance(STEP);
        ui.window().take_snapshot().expect("snapshot");
    }

    // Three sweeps' worth of frames.
    let frames: Vec<_> = (0..200)
        .map(|_| {
            offscreen::advance(STEP);
            ui.window().take_snapshot().expect("snapshot")
        })
        .collect();

    // The page's right margin, beside the rows and above the floating bar: nothing is
    // drawn there, so it stays the ground's one colour.
    let margin = |shot| strip(shot, (WIDTH - 24, WIDTH - 4), (300, 560));
    let ground = margin(&frames[0])[0];
    for (i, f) in frames.iter().enumerate() {
        let lit = margin(f).iter().filter(|p| **p != ground).count();
        assert_eq!(
            lit, 0,
            "frame {i}: {lit} margin pixels lit outside the rows"
        );
    }

    // The rows' area does move: the sweep runs inside the shapes.
    let rows = |shot| strip(shot, (300, WIDTH - 60), (300, 560));
    let first = rows(&frames[0]);
    let moved = frames.iter().filter(|f| rows(f) != first).count();
    assert!(
        moved > 10,
        "the sweep didn't light the placeholders ({moved} frames moved)"
    );
}
