//! A failed lyrics lookup says why, and only an LRCLIB outage is asked again by itself.
//! - An outage with the song's plain lyrics to fall back on: they show, under "Synced lyrics
//!   are unavailable right now".
//! - LRCLIB down and the song with no lyrics of its own: "Lyrics are unavailable right now".
//!
//! In both, while the view stays open LRCLIB is asked once more a minute later, and the
//! synced lyrics found then take their place: one extra lookup at most, never a loop, and none
//! once the view has closed. Any other failure: "Couldn't load lyrics", asked again only on
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

/// Checks what the view shows for `kind` before LRCLIB is asked again.
fn shows_why(data: &LyricsData, kind: SnapshotLyrics) {
    match kind {
        SnapshotLyrics::Outage => {
            assert_eq!(data.get_state(), LyricsState::Plain, "{kind:?}");
            assert!(
                data.get_synced_unavailable(),
                "{kind:?}: the note blames the song"
            );
        }
        _ => {
            assert_eq!(data.get_state(), LyricsState::Error, "{kind:?}");
            assert_eq!(
                data.get_message(),
                "Lyrics are unavailable right now",
                "{kind:?}"
            );
        }
    }
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

    for kind in [SnapshotLyrics::Outage, SnapshotLyrics::Unavailable] {
        data.set_full(true);
        scene.settle();

        // The wording says why; nothing before the minute is up, then one lookup, and the
        // synced lines take over.
        lyrics::show_for_snapshot(&scene.ctx, kind);
        scene.settle();
        shows_why(&data, kind);
        let asked = lyrics::lookups();
        wait(&scene, 59);
        assert_eq!(lyrics::lookups(), asked, "{kind:?}: asked again too soon");
        shows_why(&data, kind);
        wait(&scene, 2);
        assert_eq!(
            lyrics::lookups(),
            asked + 1,
            "{kind:?}: LRCLIB wasn't asked again"
        );
        assert_eq!(data.get_state(), LyricsState::Synced, "{kind:?}");
        assert!(
            !data.get_synced_unavailable(),
            "{kind:?}: the note stayed with the synced lines"
        );
        wait(&scene, 300);
        assert_eq!(lyrics::lookups(), asked + 1, "{kind:?}: asked in a loop");

        // The view closes before the minute is up: nothing is asked.
        lyrics::show_for_snapshot(&scene.ctx, kind);
        scene.settle();
        shows_why(&data, kind);
        let asked = lyrics::lookups();
        data.set_full(false);
        scene.settle();
        wait(&scene, 120);
        assert_eq!(
            lyrics::lookups(),
            asked,
            "{kind:?}: asked with no lyrics view open"
        );
    }
}
