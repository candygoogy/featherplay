//! Library > Liked, shown again while its list is on screen (a revisit, a Try again, the
//! refresh behind a cached list): the fresh result lands in place. The list keeps its
//! scroll position and never falls back to the skeleton. Offscreen platform with fixture
//! data; no window is shown.

use std::time::{Duration, Instant};

use featherplay::snapshot::Scene;
use featherplay::ui::LibraryData;
use slint::{ComponentHandle, LogicalPosition, Model, Rgba8Pixel, SharedPixelBuffer};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;

/// Runs timers and animations for `ms` (the wheel's ease), rendering as the window would.
fn run_for(scene: &Scene, ms: u64) {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        slint::platform::update_timers_and_animations();
        scene.ui.window().take_snapshot().expect("snapshot");
        std::thread::sleep(Duration::from_millis(4));
    }
}

/// The list's area: right of the sidebar, below the chips, above the floating bar.
fn list_area(shot: &SharedPixelBuffer<Rgba8Pixel>) -> Vec<Rgba8Pixel> {
    let w = shot.width() as usize;
    shot.as_slice()
        .chunks(w)
        .skip(240)
        .take(440)
        .flat_map(|row| row[260..].iter().copied())
        .collect()
}

fn shot(scene: &Scene) -> Vec<Rgba8Pixel> {
    list_area(&scene.ui.window().take_snapshot().expect("snapshot"))
}

/// One test: the platform can be installed once per process.
#[test]
fn a_fresh_result_keeps_the_list_where_it_was() {
    let scene = Scene::new(WIDTH, HEIGHT, true, false).expect("scene");
    scene.open("library").expect("library");
    scene.settle();
    let library = scene.ui.global::<LibraryData>();
    let rows = library.get_liked().row_count();
    assert!(rows > 10, "the fixture's liked songs: {rows}");
    let top = shot(&scene);

    // Scrolled down three notches.
    featherplay::offscreen::wheel(scene.ui.window(), LogicalPosition::new(700.0, 500.0), -3.0);
    run_for(&scene, 400);
    let scrolled = shot(&scene);
    assert!(scrolled != top, "the list didn't scroll");

    // The page asks again: the list stays, at the same place, through the load and after.
    scene.ctx.show_current();
    slint::platform::update_timers_and_animations();
    assert_eq!(
        library.get_liked().row_count(),
        rows,
        "the list was emptied while the fresh result loads"
    );
    scene.settle();
    run_for(&scene, 300);
    assert_eq!(library.get_liked().row_count(), rows);
    assert!(
        shot(&scene) == scrolled,
        "the list moved or flashed when the fresh result landed"
    );
}
