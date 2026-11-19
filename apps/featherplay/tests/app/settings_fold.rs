//! Settings makes the sections below the fold after its first frame, one a tick: making and
//! laying out every row with the page was most of that frame. Lyrics and Scrobbling are among
//! them in any view up to `Rows.lyrics-least-y` tall, which can't show them in its first
//! frame; a taller view makes them with the page. On the offscreen platform with the
//! fixtures; no window is shown.

use std::time::Duration;

use featherplay::snapshot::Scene;
use featherplay::ui::{SettingsData, Theme};
use slint::ComponentHandle;
use slint::platform::WindowEvent;

use crate::support::label;

/// `Rows.lyrics-least-y`: where Lyrics starts at the least, down the view.
const LEAST: f32 = 1500.0;
/// `Theme.top-bar-h`, over the view.
const TOP_BAR: f32 = 48.0;

/// Settings at `width` × `height` logical pixels, as its first frame leaves it.
fn first_frame(width: u32, height: u32) -> Scene {
    let scene = Scene::new(width, height, true, false).expect("scene");
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 1.0 });
    w.dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(width as f32, height as f32),
    });
    scene.play(false).expect("player");
    scene.settle();
    scene.open("settings").expect("settings");
    w.take_snapshot().expect("the first frame");
    scene
}

/// The frames after the first, a tick each: every section is made by the end.
fn later_frames(scene: &Scene) {
    for _ in 0..8 {
        scene.ui.window().take_snapshot().expect("snapshot");
        std::thread::sleep(Duration::from_millis(2));
        scene.settle_once();
    }
    scene.settle();
}

/// Whether a row labelled `row` is made, in view or not.
fn made(scene: &Scene, row: &str) -> bool {
    !label::any_where(scene, |_, l| l == row).is_empty()
}

/// Where the section titled `title` starts, down the window: its title's top. The index
/// beside the column names the sections too, in narrower entries.
fn section_top(scene: &Scene, title: &str) -> f32 {
    label::any_where(scene, |item, l| {
        l == title && item.geometry().width() > 400.0
    })
    .first()
    .map(|item| label::rect(item).origin.y)
    .unwrap_or_else(|| panic!("no section titled {title}"))
}

#[test]
fn lyrics_and_scrobbling_are_made_after_the_first_frame_where_it_cant_show_them() {
    // The tallest view the rule makes them late in.
    let height = LEAST + TOP_BAR - 4.0;
    let scene = first_frame(1280, height as u32);
    assert_eq!(scene.ui.global::<Theme>().get_top_bar_h(), TOP_BAR);
    assert!(
        made(&scene, "Prefer song versions"),
        "Playback isn't in the first frame"
    );
    for row in ["Show lyrics", "Scrobble"] {
        assert!(
            !made(&scene, row),
            "{row:?} was made with the first frame, below the fold"
        );
    }
    later_frames(&scene);
    for row in ["Show lyrics", "Scrobble"] {
        assert!(made(&scene, row), "{row:?} was never made");
    }
    let top = section_top(&scene, "Lyrics");
    assert!(
        top >= height,
        "Lyrics starts at {top}, in the {height} px window that made it late"
    );
}

#[test]
fn a_view_tall_enough_to_show_them_makes_them_with_the_page() {
    let scene = first_frame(1280, (LEAST + TOP_BAR + 4.0) as u32);
    for row in ["Show lyrics", "Scrobble"] {
        assert!(
            made(&scene, row),
            "{row:?} wasn't in the first frame of a view that could show it"
        );
    }
}

/// The rule's ground: Lyrics starts at least `LEAST` down the view even with Playback and
/// Sound at their shortest (the widest column, so no row's text wraps more than it must,
/// and the rows that come and go hidden). A change that makes them shorter fails here, and
/// the least has to follow it.
#[test]
fn lyrics_starts_no_higher_than_the_rule_takes_it_to() {
    let scene = first_frame(2560, 1440);
    let data = scene.ui.global::<SettingsData>();
    // Crossfade off hides its "fade on skip" row; the sleep timer off hides its length.
    data.set_crossfade(0);
    data.set_sleep_timer(false);
    later_frames(&scene);
    let top = section_top(&scene, "Lyrics") - TOP_BAR;
    assert!(
        top >= LEAST,
        "Lyrics starts {top} px down the view, above the {LEAST} px the rule counts on"
    );
}
