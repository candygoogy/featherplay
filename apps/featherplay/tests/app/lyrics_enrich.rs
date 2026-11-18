//! Translate and Romanize: the round buttons beside ⋯ in Now playing's lyrics and the two
//! rows in Settings > Lyrics turn the same switches, and the lines under each lyric come and
//! go with them. No network: the enriched snapshot supplies made-up answers.

use featherplay::lyrics::{self, SnapshotLyrics};
use featherplay::pages::settings;
use featherplay::snapshot::Scene;
use featherplay::ui::LyricsData;
use i_slint_core::accessibility::AccessibilityAction;
use i_slint_core::items::AccessibleRole;
use slint::ComponentHandle;

use crate::support::label;

/// The element labelled `name` with `role`, pressed as a screen reader does (a settings row
/// may be out of view).
fn press_as(scene: &Scene, role: AccessibleRole, name: &str) {
    label::any_where(scene, |item, l| l == name && item.accessible_role() == role)
        .first()
        .unwrap_or_else(|| panic!("no {role:?} says {name:?}"))
        .accessible_action(&AccessibilityAction::Default);
    scene.settle();
}

#[test]
fn the_round_buttons_take_the_lines_under_each_lyric_away_and_back() {
    let scene = Scene::new(1568, 900, true, false).expect("scene");
    let data = scene.ui.global::<LyricsData>();
    scene.play(false).expect("player");
    scene.open("now-playing").expect("now playing");
    data.set_full(true);
    lyrics::show_for_snapshot(&scene.ctx, SnapshotLyrics::Enriched);
    scene.settle();

    let first = "明日へ歩き出す";
    let both = format!("{first}. ashita e arukidasu. I start walking towards tomorrow");
    assert!(
        label::shows(&scene, &both),
        "the line playing has no romanization and English"
    );

    press_as(&scene, AccessibleRole::Button, "Romanize");
    assert!(!data.get_romanization_on() && !settings::lyrics_romanization());
    let english = format!("{first}. I start walking towards tomorrow");
    assert!(label::shows(&scene, &english), "the romanization stayed");

    press_as(&scene, AccessibleRole::Button, "Translate to English");
    assert!(!data.get_translation_on() && !settings::lyrics_translation());
    assert!(label::shows(&scene, first), "the English stayed");
    assert!(
        !data.get_extras(),
        "the lines stay spread apart with nothing under them"
    );

    // Back on, the session's answers return at once.
    press_as(&scene, AccessibleRole::Button, "Translate to English");
    assert!(data.get_translation_on() && settings::lyrics_translation());
    assert!(label::shows(&scene, &english));
}

#[test]
fn the_settings_rows_turn_the_same_switches() {
    let scene = Scene::new(1568, 900, true, false).expect("scene");
    let data = scene.ui.global::<LyricsData>();
    scene.play(false).expect("player");
    scene.open("settings").expect("settings");
    // Lyrics is below the fold: its rows are made a frame or two after the page.
    for _ in 0..8 {
        scene.ui.window().take_snapshot().expect("a frame");
        scene.settle_once();
    }
    scene.settle();

    press_as(&scene, AccessibleRole::Switch, "Translate to English");
    assert!(data.get_translation_on() && settings::lyrics_translation());
    press_as(&scene, AccessibleRole::Switch, "Romanize");
    assert!(data.get_romanization_on() && settings::lyrics_romanization());
    press_as(&scene, AccessibleRole::Switch, "Translate to English");
    assert!(!data.get_translation_on() && !settings::lyrics_translation());
    assert!(data.get_romanization_on(), "Romanize kept its own switch");
}

#[test]
fn an_english_song_keeps_its_lines_while_the_translation_is_asked_for() {
    let scene = Scene::new(1568, 900, true, false).expect("scene");
    let data = scene.ui.global::<LyricsData>();
    scene.play(false).expect("player");
    scene.open("now-playing").expect("now playing");
    data.set_full(true);
    // The fixture's English lyrics, with Translate on and its answer on the way: it may come
    // back English, with nothing to add, so no bars and no wider spacing meanwhile.
    lyrics::show_for_snapshot(&scene.ctx, SnapshotLyrics::Synced);
    data.set_translation_on(true);
    data.set_translation_loading(true);
    scene.settle();
    assert!(!data.get_has_non_latin());
    assert!(!data.get_awaiting_english() && !data.get_extras());

    // Lyrics in another script can't be English: their bars show at once.
    lyrics::show_for_snapshot(&scene.ctx, SnapshotLyrics::Translating);
    scene.settle();
    assert!(data.get_awaiting_english() && data.get_extras());
}
