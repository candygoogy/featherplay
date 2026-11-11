//! A theme switch recolours the player bar at once, in the frame that shows the rest of the
//! window in the new theme, even while a new cover's colours are still worked out; a new
//! song's colour still crossfades into it. On the offscreen platform with the fixtures; no
//! window is shown.

use featherplay::fixtures::Fixtures;
use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::Theme;
use slint::ComponentHandle;

use crate::support::{self, frames};

const W: u32 = 1280;
const H: u32 = 800;

#[test]
fn a_theme_switch_recolours_the_bar_at_once_and_a_new_song_still_fades() {
    // Two songs with covers of their own colours, so the bar's tint changes with the song.
    let f = Fixtures::load().expect("fixtures");
    let radio = f.radio().expect("radio");
    let cover = |i: usize| radio[i].thumbnail.clone().expect("a cover");
    let solid = |rgb| image::RgbImage::from_pixel(96, 96, image::Rgb(rgb));
    support::home_with_covers(
        "theme-switch",
        &[
            (&cover(0), solid([200, 40, 40])),
            (&cover(1), solid([40, 80, 200])),
        ],
    );

    let scene = Scene::new(W, H, false, true).expect("scene");
    let theme = scene.ui.global::<Theme>();
    theme.set_motion(true);
    let w = scene.ui.window();
    scene.theme(true);
    scene.play_track(false, 0).expect("player");
    scene.ui.show().expect("show");
    scene.settle();
    offscreen::freeze_clock();
    frames(w, 200, |_| {});
    // Just inside the bar's top edge, in the middle, clear of its buttons.
    let y = H - theme.get_bar_clear() as u32 + 4;
    let bar = || offscreen::shown_row(w, y).expect("row")[W as usize / 2];
    assert!(theme.get_follow(), "the accent follows the cover");

    let dark = bar();
    scene.theme(false);
    frames(w, 1, |_| {});
    let first = bar();
    frames(w, 200, |_| {});
    let light = bar();
    assert_ne!(light, dark, "the bar changed with the theme");
    assert_eq!(
        first, light,
        "the frame after the switch shows an in-between bar"
    );

    scene.play_track(false, 1).expect("next song");
    scene.settle();
    frames(w, 40, |_| {});
    let midway = bar();
    frames(w, 200, |_| {});
    let next = bar();
    assert_ne!(next, light, "the bar took the new cover's tint");
    assert!(
        midway != light && midway != next,
        "a new song's tint crossfades: {light:?} to {next:?}, {midway:?} after 160 ms"
    );
}

#[test]
fn a_theme_switch_while_a_cover_is_analysed_recolours_at_once_then_fades_to_it() {
    let f = Fixtures::load().expect("fixtures");
    let radio = f.radio().expect("radio");
    let cover = |i: usize| radio[i].thumbnail.clone().expect("a cover");
    let solid = |rgb| image::RgbImage::from_pixel(96, 96, image::Rgb(rgb));
    support::home_with_covers(
        "theme-switch-analysed",
        &[
            (&cover(0), solid([200, 40, 40])),
            (&cover(1), solid([40, 80, 200])),
        ],
    );

    let scene = Scene::new(W, H, false, true).expect("scene");
    let theme = scene.ui.global::<Theme>();
    theme.set_motion(true);
    let w = scene.ui.window();
    scene.theme(true);
    scene.play_track(false, 0).expect("player");
    scene.ui.show().expect("show");
    scene.settle();
    offscreen::freeze_clock();
    frames(w, 200, |_| {});
    let y = H - theme.get_bar_clear() as u32 + 4;
    let bar = || offscreen::shown_row(w, y).expect("row")[W as usize / 2];
    assert!(theme.get_follow(), "the accent follows the cover");

    // The first cover's tint on light, then back on dark.
    let dark = bar();
    scene.theme(false);
    frames(w, 200, |_| {});
    let light = bar();
    scene.theme(true);
    frames(w, 200, |_| {});
    assert_eq!(bar(), dark);

    // A new song whose cover isn't analysed yet: nothing runs the loop's queued work until
    // `settle`. The switch shows the first cover's tint for light at once, and keeps it.
    scene.play_track(false, 1).expect("next song");
    frames(w, 1, |_| {});
    scene.theme(false);
    frames(w, 1, |_| {});
    assert_eq!(
        bar(),
        light,
        "the switch waited for the new cover's colours"
    );
    frames(w, 200, |_| {});
    assert_eq!(bar(), light);

    // The analysis ends: the new cover's tint crossfades in, as for any new song.
    scene.settle();
    frames(w, 40, |_| {});
    let midway = bar();
    frames(w, 200, |_| {});
    let next = bar();
    assert_ne!(next, light, "the bar took the new cover's tint");
    assert!(
        midway != light && midway != next,
        "the new cover's tint crossfades: {light:?} to {next:?}, {midway:?} after 160 ms"
    );
}
