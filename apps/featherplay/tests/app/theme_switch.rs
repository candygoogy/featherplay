//! A theme switch crossfades every colour together: the window and the player bar are at
//! the same blend in every frame, the open mini player too, and nothing runs once it's over.
//! Without motion it recolours at once, in one frame, even while a new cover's colours are
//! still worked out; a new song's colour still crossfades into it after. On the offscreen
//! platform with the fixtures; no window is shown.

use std::time::Instant;

use featherplay::snapshot::Scene;
use featherplay::ui::Theme;
use featherplay::{mini, offscreen, repaint};
use featherplay_controller::fixtures::Fixtures;
use slint::ComponentHandle;

use crate::support::{self, FRAME_240, frames};

const W: u32 = 1280;
const H: u32 = 800;

/// Where `px` is between `from` (0) and `to` (1), on the channel that moves most.
fn blend(px: [u8; 4], from: [u8; 4], to: [u8; 4]) -> f32 {
    let i = (0..3)
        .max_by_key(|&i| (i32::from(to[i]) - i32::from(from[i])).abs())
        .expect("a channel");
    (f32::from(px[i]) - f32::from(from[i])) / (f32::from(to[i]) - f32::from(from[i]))
}

fn rgba(c: slint::Color) -> [u8; 4] {
    [c.red(), c.green(), c.blue(), c.alpha()]
}

/// A scene on dark, shown, with motion, the radio's first song playing in its red cover's
/// tint, every fade done, on the frozen clock.
fn red_song(name: &str, w: u32, h: u32) -> Scene {
    let f = Fixtures::load().expect("fixtures");
    let radio = f.radio().expect("radio");
    let cover = |i: usize| radio[i].thumbnail.clone().expect("a cover");
    let solid = |rgb| image::RgbImage::from_pixel(96, 96, image::Rgb(rgb));
    support::home_with_covers(
        name,
        &[
            (&cover(0), solid([200, 40, 40])),
            (&cover(1), solid([40, 80, 200])),
        ],
    );
    let scene = Scene::new(w, h, false, true).expect("scene");
    scene.ui.global::<Theme>().set_motion(true);
    scene.theme(true);
    scene.play_track(false, 0).expect("player");
    scene.ui.show().expect("show");
    scene.settle();
    offscreen::freeze_clock();
    frames(scene.ui.window(), 200, |_| {});
    assert!(
        scene.ui.global::<Theme>().get_follow(),
        "the accent follows the cover"
    );
    scene
}

#[test]
fn a_theme_switch_crossfades_the_window_the_bar_and_the_mini_player_together() {
    let scene = red_song("theme-switch-fade", W, H);
    let theme = scene.ui.global::<Theme>();
    let w = scene.ui.window();
    let mini = mini::create(&scene.ctx).expect("mini player");
    mini.show().expect("show");
    scene.settle();
    let own = mini.global::<Theme>();
    // Just inside the bar's top edge, in the middle, clear of its buttons; and the sidebar's
    // ground above the bar, clear of its shadow and of the rows.
    let y = H - theme.get_bar_clear() as u32 + 4;
    let bar = || offscreen::shown_row(w, y).expect("row")[W as usize / 2];
    let side_y = H - theme.get_bar_clear() as u32 - 40;
    let ground = || offscreen::shown_row(w, side_y).expect("row")[8];
    let (dark_bar, dark_ground) = (bar(), ground());
    assert_eq!(
        dark_ground,
        rgba(theme.get_surface()),
        "not the sidebar's ground"
    );

    scene.theme(false);
    // 48 ms into the switch's 200.
    frames(w, 12, |_| {});
    let (mid_bar, mid_ground) = (bar(), ground());
    assert!(theme.get_switching(), "the switch ended early");
    let shade = theme.get_shade();
    assert_eq!(
        own.get_shade(),
        shade,
        "the mini player isn't at the window's blend"
    );
    assert_eq!(
        own.get_bar(),
        theme.get_bar(),
        "the mini player's bar fell behind"
    );
    assert_eq!(own.get_ground(), theme.get_ground());

    frames(w, 100, |_| {});
    let (light_bar, light_ground) = (bar(), ground());
    assert_ne!(light_bar, dark_bar, "the bar changed with the theme");
    assert_eq!(light_ground, rgba(theme.get_surface()));
    assert_eq!(theme.get_shade(), 0.0, "the window didn't end on light");
    assert_eq!(own.get_shade(), 0.0, "the mini player didn't end on light");
    assert_eq!(own.get_bar(), theme.get_bar());

    // The two at the same place between the themes, well inside it.
    let (b, g) = (
        blend(mid_bar, dark_bar, light_bar),
        blend(mid_ground, dark_ground, light_ground),
    );
    assert!(
        (0.2..0.95).contains(&g),
        "48 ms in, the ground is {g} of the way to light (shade {shade})"
    );
    assert!(
        (b - g).abs() < 0.05,
        "the bar is {b} of the way to light where the ground is {g}"
    );
}

/// Over and settled, nothing runs: no animation, no timer, no frame asked for.
#[test]
fn a_theme_switch_while_idle_starts_nothing_once_it_ends() {
    let scene = Scene::new(W, H, false, true).expect("scene");
    let theme = scene.ui.global::<Theme>();
    theme.set_motion(true);
    scene.theme(true);
    scene.ui.show().expect("show");
    scene.settle();
    let w = scene.ui.window();
    offscreen::freeze_clock();
    frames(w, 200, |_| {});
    assert!(
        !w.has_active_animations() && slint::platform::duration_until_next_timer_update().is_none(),
        "the scene isn't idle before the switch"
    );

    scene.theme(false);
    frames(w, 4, |_| {});
    assert!(theme.get_switching(), "no crossfade");
    frames(w, 100, |_| {});
    assert!(!theme.get_switching());
    let after = frames(w, 20, |_| {});
    assert!(
        !w.has_active_animations()
            && slint::platform::duration_until_next_timer_update().is_none()
            && after.iter().all(|f| f.damaged == 0),
        "the switch left something running: animating {}, next timer {:?}, damaged {:?}",
        w.has_active_animations(),
        slint::platform::duration_until_next_timer_update(),
        after.iter().map(|f| f.damaged).collect::<Vec<_>>()
    );
}

/// Without Windows' animation effects the switch is at once: the next frame shows the new
/// theme, the bar included. A new song's tint still crossfades in after it.
#[test]
fn without_motion_a_theme_switch_recolours_at_once_and_a_new_song_still_fades() {
    let scene = red_song("theme-switch-still", W, H);
    let theme = scene.ui.global::<Theme>();
    theme.set_motion(false);
    let w = scene.ui.window();
    let y = H - theme.get_bar_clear() as u32 + 4;
    let bar = || offscreen::shown_row(w, y).expect("row")[W as usize / 2];

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
fn without_motion_a_switch_while_a_cover_is_analysed_recolours_at_once_then_fades_to_it() {
    let scene = red_song("theme-switch-analysed", W, H);
    let theme = scene.ui.global::<Theme>();
    theme.set_motion(false);
    let w = scene.ui.window();
    let y = H - theme.get_bar_clear() as u32 + 4;
    let bar = || offscreen::shown_row(w, y).expect("row")[W as usize / 2];

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

/// The cost of a switch's frames at 1600 x 1000, each a full repaint, beside a full repaint
/// of the window at rest. A measurement: `--run-ignored only`.
#[test]
#[ignore]
fn switch_frames_cost_about_a_full_repaint() {
    let (w, h) = (1600, 1000);
    let scene = red_song("theme-switch-cost", w, h);
    let window = scene.ui.window();
    let full = (0..10)
        .map(|_| {
            repaint::all(window);
            offscreen::advance(FRAME_240);
            let started = Instant::now();
            offscreen::present(window).expect("present");
            started.elapsed()
        })
        .max()
        .expect("frames");
    scene.theme(false);
    let mut switch = Vec::new();
    for _ in 0..60 {
        offscreen::advance(FRAME_240);
        let started = Instant::now();
        let shown = offscreen::present(window).expect("present");
        if shown.damaged > 0 {
            switch.push(started.elapsed());
        }
    }
    let worst = switch.iter().max().expect("switch frames");
    println!(
        "full repaint at rest {full:?}; {} switch frames, the worst {worst:?}",
        switch.len()
    );
    assert!(
        *worst < full * 3,
        "a switch frame took {worst:?}, a full repaint {full:?}"
    );
}
