//! Ctrl+Left, Ctrl+Right, Ctrl+S and Ctrl+R work the bar's Previous, Next, Shuffle and
//! Repeat from any page, on the offscreen platform with fixture data (no window is shown).
//! None of the window's keys (those, Space, Up and Down) reach playback while a text field
//! has the keys: Space types there, Up and Down move the search field's cursor or its
//! suggestions, Ctrl+Left and Ctrl+Right move by word. A held key acts once.

use featherplay::controller::Action;
use featherplay::snapshot::Scene;
use featherplay::ui::{Player, TopBarData};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, SharedString};

/// `key` pressed, with Ctrl held (`ctrl`), auto-repeated `repeats` times before the release.
fn press(w: &slint::Window, ctrl: bool, key: &str, repeats: usize) {
    let control: SharedString = Key::Control.into();
    if ctrl {
        w.dispatch_event(WindowEvent::KeyPressed {
            text: control.clone(),
        });
    }
    w.dispatch_event(WindowEvent::KeyPressed { text: key.into() });
    for _ in 0..repeats {
        w.dispatch_event(WindowEvent::KeyPressRepeated { text: key.into() });
    }
    w.dispatch_event(WindowEvent::KeyReleased { text: key.into() });
    if ctrl {
        w.dispatch_event(WindowEvent::KeyReleased { text: control });
    }
}

fn ctrl(w: &slint::Window, key: &str) {
    press(w, true, key, 0);
}

fn tap(w: &slint::Window, key: &str) {
    press(w, false, key, 0);
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

/// What the window asked the controller to do to playback since the last call.
fn playback(scene: &Scene) -> Vec<&'static str> {
    scene
        .take_actions()
        .iter()
        .filter_map(|a| match a {
            Action::Toggle => Some("toggle"),
            Action::Previous => Some("previous"),
            Action::Next => Some("next"),
            Action::SetShuffle(_) => Some("shuffle"),
            Action::SetRepeat(_) => Some("repeat"),
            Action::SetVolume { .. } => Some("volume"),
            _ => None,
        })
        .collect()
}

fn key(k: Key) -> SharedString {
    k.into()
}

/// What the four keys asked the controller for, one press each.
fn transport(scene: &Scene) -> Vec<&'static str> {
    let w = scene.ui.window();
    for k in [
        key(Key::LeftArrow),
        key(Key::RightArrow),
        "s".into(),
        "r".into(),
    ] {
        ctrl(w, &k);
    }
    playback(scene)
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
    let player = scene.ui.global::<Player>();
    let query = || scene.ui.global::<TopBarData>().get_query();
    // The default 5 % step, not the dev settings' step.
    player.set_volume_step(5);
    player.set_volume(0.5);

    // Ctrl+K: the search field takes the keys.
    ctrl(w, "k");
    tap(w, "a");
    scene.take_actions();
    for (label, k) in [("Up", key(Key::UpArrow)), ("Down", key(Key::DownArrow))] {
        tap(w, &k);
        assert_eq!(
            playback(&scene),
            Vec::<&str>::new(),
            "{label} in the search field reached playback"
        );
    }
    assert_eq!(query(), "a", "Up and Down in the search field");
    let volume = player.get_volume();
    assert!(
        (volume - 0.5).abs() < 1e-4,
        "the field's arrows moved the volume to {volume}"
    );
    tap(w, " ");
    assert_eq!(
        playback(&scene),
        Vec::<&str>::new(),
        "Space in the search field reached playback"
    );
    tap(w, "b");
    assert_eq!(query(), "a b", "Space in the search field didn't type");
    for (label, k) in [
        ("Ctrl+Left", key(Key::LeftArrow)),
        ("Ctrl+Right", key(Key::RightArrow)),
        ("Ctrl+S", "s".into()),
        ("Ctrl+R", "r".into()),
    ] {
        ctrl(w, &k);
        assert_eq!(
            playback(&scene),
            Vec::<&str>::new(),
            "{label} in the search field reached playback"
        );
    }

    // Out of the field (Escape clears, then leaves), every key works again.
    tap(w, &key(Key::Escape));
    tap(w, &key(Key::Escape));
    scene.take_actions();
    tap(w, " ");
    assert_eq!(
        playback(&scene),
        ["toggle"],
        "Space after leaving the field"
    );
    tap(w, &key(Key::UpArrow));
    assert_eq!(playback(&scene), ["volume"], "Up after leaving the field");
    let volume = player.get_volume();
    assert!(
        (volume - 0.55).abs() < 1e-4,
        "Up after leaving the field: {volume}, not 0.55"
    );
    assert_eq!(
        transport(&scene),
        ["previous", "next", "shuffle", "repeat"],
        "Ctrl+Left, Ctrl+Right, Ctrl+S and Ctrl+R after leaving the field"
    );
}

#[test]
fn a_held_key_acts_once() {
    let scene = scene("home");
    let w = scene.ui.window();
    for (label, ctrl, k, want) in [
        ("Space", false, " ".into(), "toggle"),
        ("Ctrl+Right", true, key(Key::RightArrow), "next"),
        ("Ctrl+S", true, "s".into(), "shuffle"),
    ] {
        press(w, ctrl, &k, 5);
        assert_eq!(
            playback(&scene),
            [want],
            "{label} held: auto-repeats acted again"
        );
    }
}
