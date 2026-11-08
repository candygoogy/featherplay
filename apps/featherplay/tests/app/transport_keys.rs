//! Ctrl+Left, Ctrl+Right, Ctrl+S and Ctrl+R work the bar's Previous, Next, Shuffle and
//! Repeat from any page, on the offscreen platform with fixture data (no window is shown).
//! Not while a text field has the keys (Ctrl+Left and Ctrl+Right move by word there), and a
//! held key acts once.

use featherplay::controller::Action;
use featherplay::snapshot::Scene;
use slint::ComponentHandle;
use slint::platform::{Key, WindowEvent};

/// `key` pressed with Ctrl held, auto-repeated `repeats` times before the release.
fn ctrl(w: &slint::Window, key: &str, repeats: usize) {
    w.dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    w.dispatch_event(WindowEvent::KeyPressed { text: key.into() });
    for _ in 0..repeats {
        w.dispatch_event(WindowEvent::KeyPressRepeated { text: key.into() });
    }
    w.dispatch_event(WindowEvent::KeyReleased { text: key.into() });
    w.dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
}

/// The window at 1280 × 800 with the fixture radio playing, keys in it.
fn scene(page: &str) -> Scene {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.play(false).expect("player");
    scene.open(page).expect("page");
    scene.settle();
    scene
        .ui
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    scene.take_actions();
    scene
}

/// What the four keys asked the controller for, one press each.
fn transport(scene: &Scene) -> Vec<&'static str> {
    let w = scene.ui.window();
    let left: slint::SharedString = Key::LeftArrow.into();
    let right: slint::SharedString = Key::RightArrow.into();
    for key in [left.as_str(), right.as_str(), "s", "r"] {
        ctrl(w, key, 0);
    }
    scene
        .take_actions()
        .iter()
        .filter_map(|a| match a {
            Action::Previous => Some("previous"),
            Action::Next => Some("next"),
            Action::SetShuffle(_) => Some("shuffle"),
            Action::SetRepeat(_) => Some("repeat"),
            _ => None,
        })
        .collect()
}

#[test]
fn the_keys_work_previous_next_shuffle_and_repeat_on_any_page() {
    let scene = scene("home");
    for page in ["home", "settings", "now-playing"] {
        scene.open(page).expect(page);
        scene.settle();
        scene.take_actions();
        assert_eq!(
            transport(&scene),
            ["previous", "next", "shuffle", "repeat"],
            "on {page}"
        );
    }
}

#[test]
fn the_keys_leave_a_text_field_alone() {
    let scene = scene("home");
    let w = scene.ui.window();
    // Ctrl+K: the search field takes the keys.
    ctrl(w, "k", 0);
    scene.take_actions();
    assert_eq!(transport(&scene), Vec::<&str>::new());

    // Out of the field, they work again.
    let escape: slint::SharedString = Key::Escape.into();
    w.dispatch_event(WindowEvent::KeyPressed {
        text: escape.clone(),
    });
    w.dispatch_event(WindowEvent::KeyReleased { text: escape });
    assert_eq!(transport(&scene), ["previous", "next", "shuffle", "repeat"]);
}

#[test]
fn a_held_key_acts_once() {
    let scene = scene("home");
    let right: slint::SharedString = Key::RightArrow.into();
    ctrl(scene.ui.window(), &right, 5);
    ctrl(scene.ui.window(), "s", 5);
    let actions = scene.take_actions();
    let next = actions.iter().filter(|a| matches!(a, Action::Next)).count();
    let shuffle = actions
        .iter()
        .filter(|a| matches!(a, Action::SetShuffle(_)))
        .count();
    assert_eq!((next, shuffle), (1, 1));
}
