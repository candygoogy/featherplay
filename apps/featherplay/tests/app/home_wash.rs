//! Home's header wash, on the offscreen platform with fixture data (no window is shown): the
//! resumed song's cover tints the page from the window's top, behind the clear top bar; grey
//! art takes the accent's colour instead; with nothing to resume the page stays plain; and
//! once Home scrolls past it, the wash is gone and the top bar is back on the ground.

use featherplay::fixtures::Fixtures;
use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::HomeData;
use slint::{ComponentHandle, LogicalPosition, Rgba8Pixel};

use crate::support;

/// The dark theme's ground.
const GROUND: [u8; 3] = [0x0F, 0x0F, 0x0F];

/// A point right of the search field, in the top bar's band: only the wash or the ground
/// shows there.
const BAND: (usize, usize) = (1262, 6);

fn at(scene: &Scene, (x, y): (usize, usize)) -> Rgba8Pixel {
    let s = scene.ui.window().take_snapshot().expect("snapshot");
    s.as_slice()[y * s.width() as usize + x]
}

fn rgb(p: Rgba8Pixel) -> [u8; 3] {
    [p.r, p.g, p.b]
}

fn resize(scene: &Scene, width: u32, height: u32) {
    let w = scene.ui.window();
    w.dispatch_event(slint::platform::WindowEvent::ScaleFactorChanged { scale_factor: 1.0 });
    w.set_size(slint::PhysicalSize::new(width, height));
    w.dispatch_event(slint::platform::WindowEvent::Resized {
        size: slint::LogicalSize::new(width as f32, height as f32),
    });
}

/// One test: the platform can be installed once per process.
/// The radio's third track wears a warm cover, and the 16th (past the radio: one of the
/// artist's records) a grey one, whatever the machine has cached.
fn covers() {
    let f = Fixtures::load().expect("fixtures");
    let radio = f.radio().expect("radio");
    let artist = f.artist().expect("artist");
    let records: Vec<_> = artist.albums.into_iter().chain(artist.singles).collect();
    let warm = radio[3].thumbnail.clone().expect("track 3's cover");
    let grey = records[16 - radio.len()]
        .thumbnail
        .clone()
        .expect("track 16's cover");
    support::home_with_covers(
        "home-wash-home",
        &[
            (
                &warm,
                image::RgbImage::from_pixel(96, 96, image::Rgb([224, 120, 40])),
            ),
            (
                &grey,
                image::RgbImage::from_pixel(96, 96, image::Rgb([128, 128, 128])),
            ),
        ],
    );
}

#[test]
fn home_washes_from_the_resumed_cover_and_scrolls_it_away() {
    covers();
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    resize(&scene, 1280, 800);
    // A warm cover (the radio's third track).
    scene.play_track(false, 3).expect("player");
    scene.open("home").expect("home");
    scene.theme(true);
    scene.settle();
    let data = scene.ui.global::<HomeData>();
    assert!(data.get_wash_behind_bar(), "no wash with a song to resume");
    let warm = at(&scene, BAND);
    assert_ne!(rgb(warm), GROUND, "the top bar's band shows the ground");
    assert!(
        warm.r > warm.b,
        "the warm cover's wash isn't warm: {warm:?}"
    );

    // Grey art: the accent's red.
    scene.play_track(false, 16).expect("grey art");
    scene.settle();
    let grey = at(&scene, BAND);
    assert!(
        grey.r > grey.g + 8 && grey.r > grey.b + 8,
        "grey art's wash isn't the accent's red: {grey:?}"
    );

    // Scrolled well past the header: no wash, the band is the ground again.
    let w = scene.ui.window();
    let point = LogicalPosition::new(760.0, 500.0);
    for _ in 0..40 {
        if !data.get_wash_behind_bar() {
            break;
        }
        offscreen::wheel(w, point, -2.0);
        scene.settle();
    }
    assert!(
        !data.get_wash_behind_bar(),
        "the wash stays behind the bar after a long scroll"
    );
    assert_eq!(rgb(at(&scene, BAND)), GROUND, "the band keeps a tint");

    // Nothing to resume: plain from the top, whatever the scroll.
    for _ in 0..40 {
        offscreen::wheel(w, point, 2.0);
        scene.settle();
    }
    scene.play(true).expect("idle");
    scene.settle();
    assert!(!data.get_wash_behind_bar(), "a wash with nothing to resume");
    assert_eq!(rgb(at(&scene, BAND)), GROUND, "a plain Home has a tint");
}
