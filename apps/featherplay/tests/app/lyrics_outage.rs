//! An LRCLIB outage: the song's plain lyrics show instead, under "Synced lyrics are unavailable
//! right now", and while the view stays open on them LRCLIB is asked once more a minute later;
//! synced lyrics found then take their place. One extra lookup at most, never a loop, and none
//! once the view has closed. On the offscreen platform with the fixtures (the re-ask finds the
//! recorded synced record) and a frozen clock; no window is shown.

use std::time::Duration;

use featherplay::lyrics::{self, SnapshotLyrics};
use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{LyricsData, LyricsState};
use slint::ComponentHandle;

fn wait(scene: &Scene, secs: u64) {
    offscreen::advance(Duration::from_secs(secs));
    scene.settle();
}

/// One test: the platform can be installed once per process.
#[test]
fn an_outage_falls_back_then_asks_lrclib_once_more() {
    let scene = Scene::new(1280, 800, true, true).expect("scene");
    scene.play(false).expect("player");
    scene.open("now-playing").expect("now playing");
    scene.ui.show().expect("show");
    let data = scene.ui.global::<LyricsData>();
    data.set_full(true);
    scene.settle();
    offscreen::freeze_clock();

    // LRCLIB failed: the plain lyrics, and the note says why they aren't synced.
    lyrics::show_for_snapshot(&scene.ctx, SnapshotLyrics::Outage);
    scene.settle();
    assert_eq!(data.get_state(), LyricsState::Plain);
    assert!(data.get_synced_unavailable(), "the note blames the song");
    let asked = lyrics::lookups();

    // Nothing before the minute is up; then one lookup, and the synced lines take over.
    wait(&scene, 59);
    assert_eq!(lyrics::lookups(), asked, "asked again too soon");
    assert_eq!(data.get_state(), LyricsState::Plain);
    wait(&scene, 2);
    assert_eq!(lyrics::lookups(), asked + 1, "LRCLIB wasn't asked again");
    assert_eq!(data.get_state(), LyricsState::Synced);
    assert!(!data.get_synced_unavailable());
    wait(&scene, 300);
    assert_eq!(lyrics::lookups(), asked + 1, "asked in a loop");

    // The view closes before the minute is up: nothing is asked.
    lyrics::show_for_snapshot(&scene.ctx, SnapshotLyrics::Outage);
    scene.settle();
    assert!(data.get_synced_unavailable());
    let asked = lyrics::lookups();
    data.set_full(false);
    scene.settle();
    wait(&scene, 120);
    assert_eq!(lyrics::lookups(), asked, "asked with no lyrics view open");
}
