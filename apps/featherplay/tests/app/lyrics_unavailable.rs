//! A failed lyrics lookup says why. LRCLIB down and the song with no lyrics of its own:
//! "Lyrics are unavailable right now", and while the view stays open LRCLIB is asked once
//! more a minute later, as after an outage with a plain fallback (`lyrics_outage.rs`); lyrics
//! found then take its place. Any other failure: "Couldn't load lyrics", asked again only on
//! Try again or the next open. On the offscreen platform with the fixtures (the re-ask finds
//! the recorded synced record) and a frozen clock; no window is shown.

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
fn a_failed_lookup_says_why_and_an_outage_is_asked_again() {
    let scene = Scene::new(1280, 800, true, true).expect("scene");
    scene.play(false).expect("player");
    scene.open("now-playing").expect("now playing");
    scene.ui.show().expect("show");
    let data = scene.ui.global::<LyricsData>();
    data.set_full(true);
    scene.settle();
    offscreen::freeze_clock();

    // Any other failure: no re-ask by itself.
    lyrics::show_for_snapshot(&scene.ctx, SnapshotLyrics::Failed);
    scene.settle();
    assert_eq!(data.get_state(), LyricsState::Error);
    assert_eq!(data.get_message(), "Couldn't load lyrics");
    let asked = lyrics::lookups();
    wait(&scene, 120);
    assert_eq!(
        lyrics::lookups(),
        asked,
        "a failed lookup asked again by itself"
    );

    // LRCLIB down, nothing else: the wording says so, and a minute later one more lookup.
    lyrics::show_for_snapshot(&scene.ctx, SnapshotLyrics::Unavailable);
    scene.settle();
    assert_eq!(data.get_state(), LyricsState::Error);
    assert_eq!(data.get_message(), "Lyrics are unavailable right now");
    let asked = lyrics::lookups();
    wait(&scene, 59);
    assert_eq!(lyrics::lookups(), asked, "asked again too soon");
    wait(&scene, 2);
    assert_eq!(lyrics::lookups(), asked + 1, "LRCLIB wasn't asked again");
    assert_eq!(data.get_state(), LyricsState::Synced);
    wait(&scene, 300);
    assert_eq!(lyrics::lookups(), asked + 1, "asked in a loop");

    // Now playing (the view and its lyric strip) is left before the minute is up: nothing
    // is asked.
    lyrics::show_for_snapshot(&scene.ctx, SnapshotLyrics::Unavailable);
    scene.settle();
    let asked = lyrics::lookups();
    scene.open("home").expect("home");
    scene.settle();
    wait(&scene, 120);
    assert_eq!(lyrics::lookups(), asked, "asked with no lyrics view open");
}
