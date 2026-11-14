//! Translation and romanization keep their switches, and their line-aligned decorations use
//! the reference hierarchy. No network: the enriched snapshot supplies made-up sample text.

use featherplay::lyrics::{self, SnapshotLyrics};
use featherplay::pages::settings;
use featherplay::snapshot::Scene;
use featherplay::ui::LyricsData;
use slint::{ComponentHandle, Model};

#[test]
fn translation_and_romanization_toggle_and_line_up_with_the_lyrics() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    let data = scene.ui.global::<LyricsData>();

    assert!(!data.get_translation_on());
    assert!(!data.get_romanization_on());
    data.invoke_set_translation(true);
    data.invoke_set_romanization(true);
    assert!(settings::lyrics_translation());
    assert!(settings::lyrics_romanization());

    // Put the switches back before filling the snapshot: its helper turns them on without
    // saving or asking Google.
    data.invoke_set_translation(false);
    data.invoke_set_romanization(false);
    scene.play(false).expect("player");
    scene.open("now-playing").expect("now playing");
    data.set_full(true);
    lyrics::show_for_snapshot(&scene.ctx, SnapshotLyrics::Enriched);
    scene.settle();

    assert!(data.get_translation_on() && data.get_romanization_on());
    assert!(data.get_has_non_latin());
    let lines = data.get_lines();
    let decorations = data.get_decorations();
    assert_eq!(lines.row_count(), 4);
    assert_eq!(decorations.row_count(), lines.row_count());
    let first = decorations.row_data(0).expect("first decoration");
    assert_eq!(first.romanization, "ima koko de utatte iru");
    assert_eq!(first.translation, "I'm singing here right now");
}
