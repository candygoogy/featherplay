//! Skipping through songs with a lyrics view open costs nothing: after a track change the
//! lookup waits about a second, and only the track that stays gets looked up. Opening a view
//! still looks up at once. On the offscreen platform with the fixtures and a frozen clock; no
//! window is shown.

use std::time::Duration;

use featherplay::lyrics;
use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{LyricsData, LyricsState};
use slint::ComponentHandle;

/// One test: the platform can be installed once per process.
#[test]
fn rapid_skips_look_up_only_the_track_that_stays() {
    let scene = Scene::new(1280, 800, true, true).expect("scene");
    scene.play(false).expect("player");
    let data = scene.ui.global::<LyricsData>();

    // Opening Now playing (its lyric strip) looks the song up at once.
    let asked = lyrics::lookups();
    scene.open("now-playing").expect("now playing");
    scene.ui.show().expect("show");
    scene.settle();
    assert_eq!(lyrics::lookups(), asked + 1);
    assert_eq!(data.get_state(), LyricsState::Synced);

    // Three skips, 200 ms apart: nothing is looked up yet.
    offscreen::freeze_clock();
    let asked = lyrics::lookups();
    for track in 1..=3 {
        scene.play_track(false, track).expect("track");
        offscreen::advance(Duration::from_millis(200));
        scene.run_queued();
    }
    assert_eq!(lyrics::lookups(), asked, "a skip looked its track up");
    assert_eq!(data.get_state(), LyricsState::Loading);

    // A second after the last skip, its track is looked up, once.
    offscreen::advance(Duration::from_millis(1000));
    scene.settle();
    assert_eq!(
        lyrics::lookups(),
        asked + 1,
        "not one lookup for the track that stayed"
    );
    assert_eq!(data.get_state(), LyricsState::Synced);
}
