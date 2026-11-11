//! Black-and-white art takes the neutral accent, not the logo's red: near-white on dark,
//! graphite on light. So does a song whose cover fails to load. Red is left for when nothing
//! plays. On the offscreen platform with the fixtures; no window is shown.

use featherplay::snapshot::Scene;
use featherplay::ui::Theme;
use featherplay_controller::fixtures::Fixtures;
use slint::{Color, ComponentHandle};

use crate::support;

fn hex(v: u32) -> Color {
    Color::from_rgb_u8((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// One test: the platform can be installed once per process.
#[test]
fn grey_and_failed_art_take_the_neutral_accent_and_nothing_playing_keeps_red() {
    // The radio's track 3 (from 0) wears a grey cover; track 4 has a cover that isn't there,
    // so it fails to load.
    let f = Fixtures::load().expect("fixtures");
    let radio = f.radio().expect("radio");
    let grey = radio[3].thumbnail.clone().expect("track 3's cover");
    assert!(radio[4].thumbnail.is_some(), "track 4 has no cover to fail");
    let flat = image::RgbImage::from_pixel(96, 96, image::Rgb([128, 128, 128]));
    support::home_with_covers("neutral-accent", &[(&grey, flat)]);

    let scene = Scene::new(1280, 800, true, false).expect("scene");
    let theme = scene.ui.global::<Theme>();
    scene.theme(true);
    scene.play_track(false, 3).expect("grey");
    scene.settle();
    assert_eq!(theme.get_accent(), hex(0xE6E6E6), "grey art on dark");
    assert!(theme.get_neutral());
    scene.theme(false);
    scene.settle();
    assert_eq!(theme.get_accent(), hex(0x3A3A3A), "grey art on light");

    scene.play_track(false, 4).expect("no cover");
    scene.settle();
    assert_eq!(theme.get_accent(), hex(0x3A3A3A), "a cover that failed");
    assert!(theme.get_neutral());

    scene.play_track(true, 0).expect("idle");
    scene.settle();
    assert_eq!(theme.get_accent(), hex(0xCC1036), "nothing playing");
    assert!(!theme.get_neutral());
}
