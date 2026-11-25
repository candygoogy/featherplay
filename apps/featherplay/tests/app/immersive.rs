//! Immersive mode, from the lyrics menu: the window goes full screen with the song and its
//! lyrics. Escape leaves it before it leaves Now playing; leaving the lyrics ends it; the
//! controls hide after three seconds, not while a menu is open, and come back when the
//! pointer moves or presses. On the offscreen platform with the demo fixtures; no window is
//! shown.

use std::time::Duration;

use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{Immersive, LyricsData, LyricsMenu, Nav, PageKind};
use i_slint_core::accessibility::AccessibilityAction;
use i_slint_core::items::AccessibleRole;
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition};

use crate::support;
use crate::support::label;

/// Now playing's full lyrics view on the demo set's first song.
fn scene() -> Scene {
    // SAFETY: set before anything reads it, on the test's only thread so far (nextest runs
    // each test in its own process).
    unsafe { std::env::set_var("FEATHERPLAY_FIXTURES", "demo") };
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.play(false).expect("player");
    scene.open("now-playing").expect("now playing");
    scene.settle();
    scene.ui.global::<LyricsData>().invoke_toggle_view();
    scene.settle();
    assert!(
        scene.ui.global::<LyricsData>().get_showing(),
        "the full view didn't open"
    );
    // The ⋯ is made as a frame is.
    scene.ui.window().take_snapshot().expect("frame");
    scene
}

/// A press on the lyrics menu's ⋯, as a click makes it.
fn open_menu(scene: &Scene) {
    let more = label::shown_where(scene, |item, l| {
        l == "Lyrics menu" && item.accessible_role() == AccessibleRole::Button
    });
    assert_eq!(more.len(), 1, "the ⋯ isn't on screen");
    more[0].accessible_action(&AccessibilityAction::Default);
    scene.settle();
    assert!(
        scene.ui.global::<LyricsMenu>().get_open(),
        "the menu didn't open"
    );
}

/// Immersive mode from the menu, by a click on its row.
fn enter(scene: &Scene) {
    open_menu(scene);
    label::click(scene, "Immersive mode");
    scene.settle();
    assert!(
        !scene.ui.global::<LyricsMenu>().get_open(),
        "the menu stayed open"
    );
    assert!(
        scene.ui.global::<Immersive>().get_active(),
        "immersive mode didn't start"
    );
    assert!(
        scene.ui.window().is_fullscreen(),
        "the window isn't full screen"
    );
}

fn escape(scene: &Scene) {
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    w.dispatch_event(WindowEvent::KeyReleased {
        text: Key::Escape.into(),
    });
    scene.settle();
}

fn controls(scene: &Scene) -> bool {
    scene.ui.global::<Immersive>().get_controls()
}

#[test]
fn escape_leaves_immersive_mode_first_and_now_playing_next() {
    let scene = scene();
    enter(&scene);
    escape(&scene);
    assert!(!scene.ui.global::<Immersive>().get_active());
    assert!(
        !scene.ui.window().is_fullscreen(),
        "the window stayed full screen"
    );
    assert_eq!(scene.ui.global::<Nav>().get_page(), PageKind::NowPlaying);
    escape(&scene);
    assert_ne!(scene.ui.global::<Nav>().get_page(), PageKind::NowPlaying);
}

#[test]
fn the_controls_hide_and_come_back_and_leaving_the_lyrics_ends_it() {
    let scene = scene();
    offscreen::freeze_clock();
    enter(&scene);
    let middle = LogicalPosition::new(640.0, 400.0);
    let wait = |ms: u64| {
        offscreen::advance(Duration::from_millis(ms));
        scene.settle();
    };

    // Three seconds after entering, the controls go; a move brings them back for three more.
    assert!(controls(&scene));
    wait(2_900);
    assert!(controls(&scene), "the controls went before three seconds");
    wait(200);
    assert!(!controls(&scene), "the controls stayed");
    assert!(
        !label::shows(&scene, "Back"),
        "Back shows without the controls"
    );
    support::point(&scene, middle);
    scene.settle();
    assert!(controls(&scene), "a move didn't bring the controls back");
    assert!(label::shows(&scene, "Back"));

    // A press with no move brings them back too (as the press ends), and does nothing else.
    wait(3_100);
    assert!(!controls(&scene));
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::PointerPressed {
        position: middle,
        button: PointerEventButton::Left,
    });
    w.dispatch_event(WindowEvent::PointerReleased {
        position: middle,
        button: PointerEventButton::Left,
    });
    scene.settle();
    assert!(controls(&scene), "a press didn't bring the controls back");
    assert!(scene.ui.global::<Immersive>().get_active());

    // A menu open keeps them, however long it stays.
    open_menu(&scene);
    wait(10_000);
    assert!(controls(&scene), "the controls went under the menu");
    escape(&scene);
    assert!(!scene.ui.global::<LyricsMenu>().get_open());
    assert!(
        scene.ui.global::<Immersive>().get_active(),
        "closing the menu left immersive mode"
    );

    // Leaving the lyrics for the cover ends it.
    scene.ui.global::<LyricsData>().invoke_toggle_view();
    scene.settle();
    assert!(!scene.ui.global::<Immersive>().get_active());
    assert!(!scene.ui.window().is_fullscreen());
}
