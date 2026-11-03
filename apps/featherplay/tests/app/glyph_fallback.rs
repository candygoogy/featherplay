//! Titles in other scripts and in styled Unicode find a font, on the offscreen platform (no
//! window is shown): Now playing, the bar and Up next draw them without one `.notdef` box.
//! The users' examples (2026-11-01) were Mathematical Alphanumeric Symbols, which no font in
//! the text's family list has, and Japanese with fullwidth brackets.

use std::sync::atomic::Ordering;

use featherplay::snapshot::{SCRIPT_TITLES, Scene};
use featherplay::ui::{Frame, QueueData};
use i_slint_renderer_skia::NOTDEF_GLYPHS;
use slint::{ComponentHandle, Model};

/// One test: the platform can be installed once per process.
#[test]
fn titles_in_other_scripts_draw_no_notdef_boxes() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.play_titled(SCRIPT_TITLES).expect("play");
    scene.open("now-playing-scripts").expect("now playing");
    scene.ui.global::<Frame>().set_queue_open(true);
    scene.settle();

    // The titles are the ones on screen: the first plays, the rest are Up next.
    let queue = scene.ui.global::<QueueData>();
    assert_eq!(queue.get_current().title, SCRIPT_TITLES[0].0);
    let upcoming: Vec<_> = queue.get_upcoming().iter().map(|r| r.title).collect();
    for (title, _) in &SCRIPT_TITLES[1..] {
        assert!(
            upcoming.iter().any(|t| t == title),
            "{title} is not in Up next"
        );
    }

    let before = NOTDEF_GLYPHS.load(Ordering::Relaxed);
    scene.ui.window().take_snapshot().expect("render");
    let boxes = NOTDEF_GLYPHS.load(Ordering::Relaxed) - before;
    assert_eq!(boxes, 0, "{boxes} glyphs drew as .notdef boxes");
}
