//! The open mini player takes a song change's cover colours with the main window's bar,
//! step by step through the same crossfade: opened halfway through one, and with the main
//! window hidden, where the fade runs for the mini player alone. Grey art's neutral accent,
//! with its own hover and glow, is the mini player's too. On the offscreen platform with the
//! fixtures; no window is shown.

use featherplay::mini;
use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{MiniPlayer, Theme};
use featherplay_controller::fixtures::Fixtures;
use slint::ComponentHandle;

use crate::support::{self, FRAME_240};

fn open(scene: &Scene) -> MiniPlayer {
    let mini = mini::create(&scene.ctx).expect("mini player");
    mini.show().expect("show");
    scene.settle();
    mini
}

/// Runs `n` frames of the frozen clock.
fn advance(n: u32) {
    for _ in 0..n {
        offscreen::advance(FRAME_240);
    }
}

#[test]
fn the_mini_player_crossfades_with_the_bar_opened_midway_and_with_the_window_hidden() {
    let f = Fixtures::load().expect("fixtures");
    let radio = f.radio().expect("radio");
    let cover = |i: usize| radio[i].thumbnail.clone().expect("a cover");
    let solid = |rgb| image::RgbImage::from_pixel(96, 96, image::Rgb(rgb));
    support::home_with_covers(
        "mini-follow",
        &[
            (&cover(0), solid([200, 40, 40])),
            (&cover(1), solid([40, 80, 200])),
            (&cover(2), solid([40, 160, 60])),
        ],
    );

    let scene = Scene::new(1280, 800, false, false).expect("scene");
    let main = scene.ui.global::<Theme>();
    main.set_motion(true);
    scene.theme(true);
    scene.play_track(false, 0).expect("player");
    scene.ui.show().expect("show");
    scene.settle();
    offscreen::freeze_clock();
    advance(200);
    assert!(main.get_follow(), "the accent follows the cover");
    let red = main.get_bar();

    // A new song: the mini player opens halfway through the bar's crossfade, and goes on
    // from where the bar is.
    scene.play_track(false, 1).expect("next song");
    scene.settle();
    advance(10);
    let mini = open(&scene);
    let own = mini.global::<Theme>();
    assert_eq!(own.get_bar(), main.get_bar());
    advance(10);
    let midway = own.get_bar();
    assert_eq!(
        midway,
        main.get_bar(),
        "the mini player fell behind the bar"
    );
    advance(200);
    let blue = main.get_bar();
    assert!(
        midway != red && midway != blue,
        "the bar crossfades: {red:?} to {blue:?}, {midway:?} midway"
    );
    assert_eq!(own.get_bar(), blue);
    assert_eq!(own.get_accent(), main.get_accent());

    // The main window hidden: the mini player alone shows the next song's crossfade.
    scene.ui.hide().expect("hide");
    scene.play_track(false, 2).expect("next song");
    scene.settle();
    advance(20);
    let midway = own.get_bar();
    assert_eq!(midway, main.get_bar());
    advance(200);
    let green = own.get_bar();
    assert_eq!(green, main.get_bar());
    assert_ne!(green, blue, "the mini player took the new cover's tint");
    assert!(
        midway != blue && midway != green,
        "with the window hidden the mini player crossfades: {blue:?} to {green:?}, {midway:?} midway"
    );
}

#[test]
fn grey_art_gives_the_mini_player_the_neutral_accent_too() {
    // The radio's track 3 (from 0) wears a grey cover, track 0 a red one.
    let f = Fixtures::load().expect("fixtures");
    let radio = f.radio().expect("radio");
    let cover = |i: usize| radio[i].thumbnail.clone().expect("a cover");
    let solid = |rgb| image::RgbImage::from_pixel(96, 96, image::Rgb(rgb));
    support::home_with_covers(
        "mini-neutral",
        &[
            (&cover(0), solid([200, 40, 40])),
            (&cover(3), solid([128, 128, 128])),
        ],
    );

    let scene = Scene::new(1280, 800, true, false).expect("scene");
    let main = scene.ui.global::<Theme>();
    scene.theme(true);
    scene.play_track(false, 3).expect("grey");
    scene.settle();
    assert!(main.get_neutral());

    // Opened on grey art.
    let mini = open(&scene);
    let own = mini.global::<Theme>();
    assert!(own.get_neutral(), "opened on grey art");
    assert_eq!(own.get_accent_hover(), main.get_accent_hover());

    // Songs change while it's open.
    scene.play_track(false, 0).expect("red");
    scene.settle();
    assert!(!main.get_neutral() && !own.get_neutral());
    scene.play_track(false, 3).expect("grey");
    scene.settle();
    assert!(own.get_neutral(), "grey art while open");
    assert_eq!(own.get_accent(), main.get_accent());
    assert_eq!(own.get_cover_glow(), main.get_cover_glow());
}
