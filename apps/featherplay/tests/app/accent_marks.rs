//! The accent rule (2026-11-22), on the offscreen platform with fixture data (no window is
//! shown): the cover's accent is only a mark (the progress fill, the playing marks, focus
//! rings). Whatever is on or selected is neutral: the bar's play button and Up next button,
//! whose glyph at rest is its neighbours' colour and which shows "open" as a round fill; the
//! chips, switches and Settings' index. A playing mark on artwork is white. A loud accent no
//! neutral could be taken for shows it.

use featherplay::snapshot::Scene;
use featherplay::ui::{Frame, HomeData, ItemKind, Theme};
use slint::{Color, ComponentHandle};

use crate::support::{self, label};

const W: u32 = 1568;
const H: u32 = 900;

/// Pixels of `img` in `r` (logical) that are the loud accent.
fn lit(scene: &Scene, img: &image::RgbaImage, r: label::LogicalRect) -> usize {
    let s = scene.ui.window().scale_factor();
    let mut n = 0;
    for y in (r.min_y() * s).max(0.0) as u32..((r.max_y() * s) as u32).min(img.height()) {
        for x in (r.min_x() * s).max(0.0) as u32..((r.max_x() * s) as u32).min(img.width()) {
            let p = img.get_pixel(x, y).0;
            if p[0] > 150 && p[1] < 110 && p[2] > 150 {
                n += 1;
            }
        }
    }
    n
}

/// The brightest pixel of `img` in `r`: a glyph's colour on the bar.
fn brightest(scene: &Scene, img: &image::RgbaImage, r: label::LogicalRect) -> [u8; 4] {
    let s = scene.ui.window().scale_factor();
    let mut best = [0u8; 4];
    for y in (r.min_y() * s) as u32..(r.max_y() * s) as u32 {
        for x in (r.min_x() * s) as u32..(r.max_x() * s) as u32 {
            let p = img.get_pixel(x, y).0;
            if p[..3].iter().map(|&v| u32::from(v)).sum::<u32>()
                > best[..3].iter().map(|&v| u32::from(v)).sum::<u32>()
            {
                best = p;
            }
        }
    }
    best
}

fn scene() -> Scene {
    let scene = Scene::new(W, H, true, true).expect("scene");
    support::resize(scene.ui.window(), W, H);
    scene.play_track(false, 0).expect("a song");
    scene.open("home").expect("home");
    scene.settle();
    scene
        .ui
        .global::<Theme>()
        .set_follow_accent(Color::from_rgb_u8(255, 0, 255));
    scene
}

/// The bar: neither the play button nor the Up next button is drawn in the accent; at rest
/// the Up next glyph is the colour of its neighbours' (the volume button's), and open, a
/// neutral fill sits behind it.
#[test]
fn the_bars_play_and_up_next_buttons_stay_neutral() {
    let scene = scene();
    let ui = &scene.ui;
    let bar_top = H as f32 - ui.global::<Theme>().get_bar_clear();
    let button = |names: &[&str]| {
        let found = label::shown_where(&scene, |item, l| {
            names.contains(&l) && label::rect(item).min_y() >= bar_top
        });
        label::rect(
            found
                .first()
                .unwrap_or_else(|| panic!("no {names:?} in the bar")),
        )
    };
    ui.global::<Frame>().set_queue_open(false);
    scene.settle();
    let shut = support::frame(&scene);
    let queue = button(&["Up next"]);
    let play = button(&["Pause", "Play"]);
    let volume = button(&["Mute", "Unmute"]);
    assert_eq!(
        lit(&scene, &shut, queue),
        0,
        "the Up next button in the accent"
    );
    assert_eq!(lit(&scene, &shut, play), 0, "the play button in the accent");
    let (q, v) = (
        brightest(&scene, &shut, queue),
        brightest(&scene, &shut, volume),
    );
    assert!(
        support::dist(q, v) <= 24,
        "the Up next glyph at rest is {q:?}, its neighbour's {v:?}"
    );

    ui.global::<Frame>().set_queue_open(true);
    scene.settle();
    let open = support::frame(&scene);
    let queue = button(&["Up next"]);
    assert_eq!(
        lit(&scene, &open, queue),
        0,
        "the open Up next button in the accent"
    );
    // The round fill: a corner of the button's circle, off the glyph, changes when it opens.
    let s = ui.window().scale_factor();
    let at = ((queue.min_x() + 7.0) * s) as u32;
    let y = ((queue.center().y) * s) as u32;
    assert!(
        support::dist(shut.get_pixel(at, y).0, open.get_pixel(at, y).0) > 12,
        "no fill behind the open Up next button"
    );
}

/// The pages: above the bar, Library (its chips, the sidebar's) and Settings (its index, its
/// switches) show no accent at all.
#[test]
fn pages_show_no_accent_above_the_bar() {
    let scene = scene();
    let bar_top = H as f32 - scene.ui.global::<Theme>().get_bar_clear();
    let above = label::LogicalRect::new(
        i_slint_core::lengths::LogicalPoint::new(0.0, 0.0),
        i_slint_core::lengths::LogicalSize::new(W as f32, bar_top - 1.0),
    );
    for page in ["library", "settings"] {
        scene.open(page).expect("page");
        scene.settle();
        let shown = support::frame(&scene);
        assert_eq!(lit(&scene, &shown, above), 0, "{page} shows the accent");
        // The bar's progress fill does: the accent is there to be seen.
        let bar = label::LogicalRect::new(
            i_slint_core::lengths::LogicalPoint::new(0.0, bar_top),
            i_slint_core::lengths::LogicalSize::new(W as f32, H as f32 - bar_top),
        );
        assert!(
            lit(&scene, &shown, bar) > 0,
            "no progress fill in the accent"
        );
    }
}

/// The playing mark on the source Shortcut's art (here the red Liked songs tile) is white,
/// not the song's accent: the brightest pixel in the art's bottom-left corner, where the mark
/// sits, is white.
#[test]
fn the_source_tiles_playing_mark_is_white() {
    let scene = scene();
    let data = scene.ui.global::<HomeData>();
    data.set_playing_kind(ItemKind::Playlist);
    data.set_playing_id("LM".into());
    scene.settle();
    let shown = support::frame(&scene);
    let found = label::shown_where(&scene, |_, l| {
        l == "Pause Liked songs" || l == "Shuffle Liked songs"
    });
    let art = label::rect(found.first().expect("the Liked songs tile's art"));
    let corner = label::LogicalRect::new(
        i_slint_core::lengths::LogicalPoint::new(art.min_x(), art.max_y() - art.height() * 0.35),
        i_slint_core::lengths::LogicalSize::new(art.width() * 0.35, art.height() * 0.35),
    );
    assert_eq!(lit(&scene, &shown, corner), 0, "the mark in the accent");
    let mark = brightest(&scene, &shown, corner);
    assert!(
        mark[..3].iter().all(|&c| c >= 235),
        "the mark's brightest pixel is {mark:?}, not white"
    );
}
