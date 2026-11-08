//! The lyrics' panel takes the playing song's cover colour whatever page is shown: a song
//! that changes while another page is open repaints the Lyrics tab's panel too, not only the
//! accent. On the offscreen platform with the fixtures; no window is shown.

use featherplay::fixtures::Fixtures;
use featherplay::follow::{self, colour};
use featherplay::snapshot::Scene;
use featherplay::ui::{LyricsData, Theme};
use slint::{Color, ComponentHandle};

use crate::support;

const WARM: [u8; 3] = [224, 120, 40];
const TEAL: [u8; 3] = [30, 140, 150];

fn solid([r, g, b]: [u8; 3]) -> image::RgbImage {
    image::RgbImage::from_pixel(96, 96, image::Rgb([r, g, b]))
}

/// The radio's third and fourth tracks wear a warm and a teal cover, whatever the machine
/// has cached.
fn covers() {
    let f = Fixtures::load().expect("fixtures");
    let radio = f.radio().expect("radio");
    let warm = radio[3].thumbnail.clone().expect("track 3's cover");
    let teal = radio[4].thumbnail.clone().expect("track 4's cover");
    support::home_with_covers(
        "lyrics-cover",
        &[(&warm, solid(WARM)), (&teal, solid(TEAL))],
    );
}

/// The dark panel a solid `rgb` cover gives.
fn expected_ground(scene: &Scene, rgb: [u8; 3]) -> Color {
    let img = image::DynamicImage::ImageRgb8(solid(rgb)).to_rgba8();
    let seed = colour::dominant(&img);
    let fixed = (!scene.ui.global::<Theme>().get_follow())
        .then(|| scene.ui.global::<Theme>().get_preset_dark());
    follow::lyric_inks_for(Some(seed), fixed).dark.ground
}

/// Within a few steps per channel: the cache stores the covers as JPEG.
fn close(a: Color, b: Color) -> bool {
    let d = |x: u8, y: u8| x.abs_diff(y) <= 6;
    d(a.red(), b.red()) && d(a.green(), b.green()) && d(a.blue(), b.blue())
}

/// One test: the platform can be installed once per process.
#[test]
fn the_lyrics_panel_follows_a_song_change_on_another_page() {
    covers();
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.theme(true);
    scene.open("library").expect("library");
    scene.play_track(false, 3).expect("warm");
    scene.settle();
    let data = scene.ui.global::<LyricsData>();
    let warm = expected_ground(&scene, WARM);
    let shown = data.get_ink_dark().ground;
    assert!(
        close(shown, warm),
        "warm cover: panel {shown:?}, want {warm:?}"
    );

    // The song changes while Library stays on screen.
    scene.play_track(false, 4).expect("teal");
    scene.settle();
    let teal = expected_ground(&scene, TEAL);
    assert!(!close(warm, teal), "the two covers give the same panel");
    let shown = data.get_ink_dark().ground;
    assert!(
        close(shown, teal),
        "teal cover: panel {shown:?}, want {teal:?}"
    );
}
