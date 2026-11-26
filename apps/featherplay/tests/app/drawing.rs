//! What draws the window (Settings › Window › Performance): a hidden window keeps its GPU
//! surface for `memory::KEEP_GPU`, then lets it go; shown again before that, it keeps it.

use std::time::Duration;

use featherplay::memory::{self, KEEP_GPU};
use featherplay::offscreen;
use featherplay::snapshot::Scene;
use slint::ComponentHandle;

/// The window minimised, or shown again, as the app's WM_SIZE hook reports it.
fn hide(scene: &Scene, hidden: bool) {
    scene.ui.window().set_minimized(hidden);
    memory::hidden(&scene.ctx, hidden);
}

fn wait(by: Duration) {
    offscreen::advance(by);
}

#[test]
fn a_hidden_window_keeps_the_gpu_for_thirty_seconds() {
    let scene = Scene::new(1100, 800, true, false).expect("scene");
    scene.ui.show().expect("show");
    scene.settle();
    offscreen::freeze_clock();
    let second = Duration::from_secs(1);

    hide(&scene, true);
    assert!(memory::keeps_gpu(), "hidden: kept");
    wait(KEEP_GPU - second);
    assert!(memory::keeps_gpu(), "just under the time: still kept");
    wait(2 * second);
    assert!(!memory::keeps_gpu(), "after the time: let go");

    // Shown again before the time: kept, and a later hide counts from its own start.
    hide(&scene, false);
    hide(&scene, true);
    wait(KEEP_GPU / 2);
    hide(&scene, false);
    assert!(!memory::keeps_gpu(), "shown: nothing waits to let go");
    hide(&scene, true);
    wait(KEEP_GPU - second);
    assert!(memory::keeps_gpu(), "hidden again: the time starts over");
    wait(2 * second);
    assert!(!memory::keeps_gpu());
    hide(&scene, false);
}
