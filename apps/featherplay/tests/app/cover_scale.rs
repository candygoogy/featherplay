//! Covers asked for before the window exists (the bar's, Home's card) come at the scale of the
//! screen its saved place opens on, not at 1.0 to be drawn blurred, and a new scale asks for
//! them again. On the offscreen platform with art on every cover.

use featherplay::snapshot::Scene;
use featherplay::ui::{HomeData, Player};
use featherplay::{bar, window_place};
use featherplay_platform::window::Monitor;
use featherplay_store::WindowPlace;
use slint::ComponentHandle;

use crate::support::{fixture_covers, frames};

/// The card's cover on Home, logical pixels (`hero::HERO_COVER`).
const CARD: f32 = 224.0;

/// The bar's and Home's card's cover widths, physical pixels.
fn widths(scene: &Scene) -> (u32, u32) {
    (
        scene.ui.global::<Player>().get_cover().size().width,
        scene.ui.global::<HomeData>().get_hero_cover().size().width,
    )
}

/// Frames until both covers are `want` wide, the covers' loads waited for in between.
fn until(scene: &Scene, want: (u32, u32)) {
    for _ in 0..600 {
        if widths(scene) == want {
            return;
        }
        scene.settle_threads();
        frames(scene.ui.window(), 1, |_| {});
    }
    panic!("covers at {:?}, not {want:?}", widths(scene));
}

#[test]
fn covers_asked_for_before_the_window_come_at_its_screens_scale() {
    fixture_covers();
    // The window, not yet made, knows no scale: 1.0.
    let scene = Scene::new(1280, 800, false, true).expect("scene");
    let screen = Monitor {
        rect: (0, 0, 3840, 2160),
        work: (0, 0, 3840, 2112),
        scale: 1.5,
        primary: true,
    };
    let place = WindowPlace {
        width: 1280.0,
        height: 800.0,
        position: Some([100.0, 60.0]),
        scale: 1.5,
        maximised: false,
        monitor: Some([0, 0, 3840, 2160]),
    };
    let scale = window_place::opening_scale(&scene.ui, Some(&place), &[screen]);
    assert_eq!(scale, Some(1.5), "the saved place's screen");
    scene.ctx.set_scale(1.5);
    scene.play(false).expect("player");
    scene.open("home").expect("home");
    let at = |scale: f32| {
        let px = |logical: f32| (logical * scale).round() as u32;
        (px(bar::cover_px(&scene.ui) as f32), px(CARD))
    };
    until(&scene, at(1.5));

    // Another screen's scale: the bar's and the card's covers come again at it.
    scene.ctx.set_scale(2.0);
    until(&scene, at(2.0));
}
