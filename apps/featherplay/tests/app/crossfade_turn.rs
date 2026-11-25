//! A crossfade's turn on Now playing: the song being left stays on screen (its title, its art,
//! its colours) while it's the louder, and the new one shows at the crossing, the
//! crossfade's middle; a cut shows the new song at once. On the offscreen platform at
//! 1600 x 1000, art on every cover, the clock frozen and stepped 4 ms a frame.

use std::time::Duration;

use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{NowPlayingData, Player, Theme};
use slint::ComponentHandle;

use crate::support::{fixture_covers, idle, resize, step};

fn scene() -> Scene {
    fixture_covers();
    let scene = Scene::new(1280, 800, false, true).expect("scene");
    scene.ui.global::<Theme>().set_motion(true);
    resize(scene.ui.window(), 1600, 1000);
    scene.sound(true).expect("player");
    scene.open("now-playing").expect("now playing");
    scene.ui.show().expect("show");
    scene.settle();
    offscreen::freeze_clock();
    idle(&scene, 100);
    scene
}

fn shown(scene: &Scene) -> String {
    scene.ui.global::<Player>().get_track_id().to_string()
}

/// Frames until `ms` of the frozen clock have passed (4 ms a frame), letting work done on
/// other threads (the new cover's colours and field) land meanwhile.
fn run(scene: &Scene, ms: u64) {
    for _ in 0..ms / 4 {
        scene.run_queued();
        step(scene);
    }
}

#[test]
fn a_crossfade_shows_the_new_song_at_its_crossing() {
    let scene = scene();
    let first = shown(&scene);
    // The field's bank of rows on screen, by its pixels' address: the fixture covers are
    // all alike in colour, so their banks' bytes can't tell them apart, but each bank the
    // turn blends is a buffer of its own.
    let field = || {
        scene
            .ui
            .global::<NowPlayingData>()
            .get_backdrop()
            .to_rgba8()
            .map(|b| b.as_bytes().as_ptr() as usize)
            .expect("a field")
    };
    let before = field();
    scene
        .crossfade_to(1, Duration::from_secs(5))
        .expect("crossfade");
    // Its cover's colours and field are worked out meanwhile (the clock stands still).
    scene.settle();
    run(&scene, 1800);
    assert_eq!(shown(&scene), first, "the new song showed before the turn");
    assert!(field() == before, "the field moved before the turn");
    // The turn runs from 1.9 s to 3.1 s; the new song shows at 2.5 s.
    run(&scene, 500);
    assert_eq!(
        shown(&scene),
        first,
        "the new song showed before the crossing"
    );
    let during = field();
    assert!(during != before, "the field didn't start to turn");
    run(&scene, 300);
    let second = shown(&scene);
    assert_ne!(second, first, "the new song didn't show at the crossing");
    run(&scene, 1000);
    let after = field();
    assert!(
        after != during && after != before,
        "the field stopped half-way"
    );
}

#[test]
fn a_cut_shows_the_new_song_at_once() {
    let scene = scene();
    let first = shown(&scene);
    scene.play_track(false, 1).expect("cut");
    step(&scene);
    assert_ne!(shown(&scene), first);
}
