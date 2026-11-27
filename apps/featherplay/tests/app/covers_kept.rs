//! The covers a page holds follow the window's scale: Home made before the window knew its
//! scale (a start whose Home comes from the disk at once) had its cards decoded at 1.0 and
//! drawn blurred at 150 %. On the offscreen platform with art on every cover.

use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::HomeData;
use slint::platform::WindowEvent;
use slint::{ComponentHandle, Model};

use crate::support::{fixture_covers, frames, resize, settle_in_frames};

/// Home's shelf cards' cover widths, by shelf and card (0: none).
fn widths(scene: &Scene) -> Vec<u32> {
    let d = scene.ui.global::<HomeData>();
    d.get_shelves()
        .iter()
        .flat_map(|s| {
            s.items
                .iter()
                .map(|c| c.cover.size().width)
                .collect::<Vec<_>>()
        })
        .collect()
}

/// Home on the fixtures at scale 1.0, its covers in.
fn home() -> Scene {
    fixture_covers();
    let scene = Scene::new(1280, 800, false, true).expect("scene");
    scene.open("home").expect("home");
    scene.ui.show().expect("show");
    scene.settle();
    offscreen::freeze_clock();
    settle_in_frames(&scene);
    scene
}

/// Frames until `done` holds, the covers' loads waited for in between.
fn until(scene: &Scene, what: &str, mut done: impl FnMut(&Scene) -> bool) {
    for _ in 0..600 {
        if done(scene) {
            return;
        }
        scene.settle_threads();
        frames(scene.ui.window(), 1, |_| {});
    }
    panic!("{what}: {:?}", widths(scene));
}

#[test]
fn covers_held_by_the_shown_page_are_decoded_again_for_a_new_scale() {
    let scene = home();
    let before = widths(&scene);
    assert!(
        before.iter().filter(|&&w| w > 0).count() > 4,
        "Home's cards have art: {before:?}"
    );

    // Another screen's scale, the page at the same logical size.
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 2.0 });
    resize(w, 2560, 1600);
    scene.ctx.covers.set_scale(2.0);
    until(
        &scene,
        "the cards' covers aren't at twice the size",
        |scene| {
            let now = widths(scene);
            for (b, n) in before.iter().zip(&now) {
                assert!(
                    *b == 0 || *n > 0,
                    "a card went blank across the change: {now:?}"
                );
            }
            before.iter().zip(&now).all(|(b, n)| *n == 2 * b)
        },
    );
}
