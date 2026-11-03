//! Wheel scrolling on a long playlist, offscreen, through the window's own present path:
//! a mouse wheel notch eases over many frames and then stops with nothing left running; a
//! direct delta (`wheel::Route::Direct`: a touchpad step, or any notch with Windows'
//! animation effects off) lands in a single frame. While `Frame.scrolling` holds the hover
//! (the pointer hasn't moved since the wheel), the wheel still scrolls and no row lights up.
//! The clock steps 4 ms, as at 240 Hz.

use std::rc::Rc;
use std::time::Duration;

use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{Frame, PlaylistData, TrackRow};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition, Model, ModelRc, VecModel};

const FRAME: Duration = Duration::from_millis(4);

/// The share of the window each of `n` frames copied to the screen.
fn damage(w: &slint::Window, n: u32) -> Vec<f64> {
    let size = w.size();
    let window = f64::from(size.width) * f64::from(size.height);
    (0..n)
        .map(|_| {
            offscreen::advance(FRAME);
            offscreen::present(w).expect("present").damaged as f64 / window
        })
        .collect()
}

/// Frames that moved the list: they repaint most of the window (the whole list). Smaller
/// repaints follow a move: the hover passing to the row now under the pointer, and the
/// scrollbar thumb, which eases its length for 150 ms when the list's height estimate changes.
fn moving(frames: &[f64]) -> Vec<usize> {
    (0..frames.len()).filter(|&i| frames[i] > 0.5).collect()
}

#[test]
fn a_notch_eases_and_a_direct_step_lands_at_once() {
    // Still: hover changes are instant, so only the scrolling repaints the rows.
    let scene = Scene::new(1280, 800, true, true).expect("scene");
    scene.play(false).expect("player");
    scene.open("playlist").expect("playlist");
    scene.ui.show().expect("show");
    scene.settle();
    let d = scene.ui.global::<PlaylistData>();
    let rows: Vec<TrackRow> = d.get_tracks().iter().collect();
    let many: Vec<TrackRow> = rows.iter().cycle().take(200).cloned().collect();
    d.set_tracks(ModelRc::from(Rc::new(VecModel::from(many))));
    scene.settle();
    offscreen::freeze_clock();
    let w = scene.ui.window();
    let list = LogicalPosition::new(640.0, 500.0);
    w.dispatch_event(WindowEvent::PointerMoved { position: list });
    damage(w, 30);

    offscreen::wheel(w, list, -1.0);
    // The ease is 180 ms: 45 frames.
    let eased = damage(w, 150);
    let moved = moving(&eased);
    assert!(moved.len() >= 30, "a notch moved in frames {moved:?}");
    assert!(
        moved.iter().all(|&i| i < 50),
        "a notch still moved after 200 ms: {moved:?}"
    );
    assert!(
        eased[100..].iter().all(|&d| d == 0.0),
        "still repainting 400 ms after a notch"
    );
    assert!(
        !w.has_active_animations(),
        "an animation outlived the notch"
    );

    w.dispatch_event(WindowEvent::PointerScrolled {
        position: list,
        delta_x: 0.0,
        delta_y: -60.0,
    });
    let direct = damage(w, 100);
    assert_eq!(moving(&direct), vec![0], "a direct step moved over frames");
    assert!(
        !w.has_active_animations(),
        "a direct step left an animation"
    );
    // The row now under the pointer took the hover: a repaint of two rows.
    assert!(direct[1] > 0.01, "no hover change after a step: {direct:?}");

    // The first wheel event under the holder takes the hover from the row; the next ones
    // light nothing up.
    scene.ui.global::<Frame>().set_scrolling(true);
    for _ in 0..2 {
        w.dispatch_event(WindowEvent::PointerScrolled {
            position: list,
            delta_x: 0.0,
            delta_y: -60.0,
        });
        damage(w, 10);
    }
    w.dispatch_event(WindowEvent::PointerScrolled {
        position: list,
        delta_x: 0.0,
        delta_y: -60.0,
    });
    let held = damage(w, 100);
    assert_eq!(
        moving(&held),
        vec![0],
        "the wheel didn't pass the hover holder"
    );
    assert!(
        held[1..].iter().all(|&d| d < 0.01),
        "a row lit up while the hover was held: {held:?}"
    );
}
