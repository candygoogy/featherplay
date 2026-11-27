//! The playing mark's clock steps only while the playing song's row is on screen
//! (`Mark.rows`, from `prepare.rs`): Liked songs with its last row playing holds the clock
//! still at the top, and the wave moves the moment that row scrolls in. Offscreen platform
//! with fixture data and a frozen clock.

use crate::support::run_for;
use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{LibraryData, Mark, Player, Theme};
use slint::{ComponentHandle, LogicalPosition, Model};

/// One test: the platform can be installed once per process.
#[test]
fn the_clock_steps_only_with_the_playing_row_on_screen() {
    let scene = Scene::new(1280, 800, false, false).expect("scene");
    scene.ui.global::<Theme>().set_motion(true);
    scene.open("library").expect("library");
    scene.settle();
    offscreen::freeze_clock();
    let w = scene.ui.window();
    let mark = scene.ui.global::<Mark>();

    // Its last row the playing song, far under the screen.
    let liked = scene.ui.global::<LibraryData>().get_liked();
    let last = liked.row_count() - 1;
    let mut row = liked.row_data(last).expect("row");
    row.playing = true;
    liked.set_row_data(last, row);
    let player = scene.ui.global::<Player>();
    player.set_has_track(true);
    player.set_playing(true);
    featherplay::prepare::marks(&scene.ctx);
    run_for(w, 300);
    assert!(!mark.get_rows(), "the row off screen counts as shown");
    let frame = mark.get_frame();
    run_for(w, 400);
    assert_eq!(
        mark.get_frame(),
        frame,
        "the clock stepped with the row off screen"
    );

    // Scrolled to the end: the row shows, the wave moves.
    for _ in 0..10 {
        offscreen::wheel(w, LogicalPosition::new(700.0, 500.0), -20.0);
        run_for(w, 100);
    }
    assert!(mark.get_rows(), "the row on screen isn't told");
    let frame = mark.get_frame();
    run_for(w, 400);
    assert!(
        mark.get_frame() >= frame + 5,
        "the clock didn't step with the row on screen ({} steps)",
        mark.get_frame() - frame
    );

    // Back at the top: it stops again.
    for _ in 0..10 {
        offscreen::wheel(w, LogicalPosition::new(700.0, 500.0), 20.0);
        run_for(w, 100);
    }
    assert!(!mark.get_rows());
}
