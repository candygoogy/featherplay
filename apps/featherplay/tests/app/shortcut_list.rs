//! The keyboard shortcuts Settings lists (`Shortcuts` in ui/shortcuts.slint), on the
//! offscreen platform with fixture data (no window is shown): every listed key, pressed as
//! the dialog draws it, is taken by the window's key handler as that row's shortcut. Ctrl+/
//! opens the list from any page, not from a text field, and Escape closes it.

use std::cell::RefCell;
use std::rc::Rc;

use featherplay::snapshot::Scene;
use featherplay::ui::{SettingsData, SettingsDialog, ShortcutRow, Shortcuts};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, Model, SharedString};

/// The window at 1280 × 800 with the fixture radio playing, on `page`, keys in it.
fn scene(page: &str) -> Scene {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.play(false).expect("player");
    scene.open(page).expect("page");
    scene.settle();
    scene
        .ui
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    scene
}

/// A keycap as the key event it stands for, and whether it's a modifier.
fn key(cap: &str) -> (SharedString, bool) {
    match cap {
        "Ctrl" => (Key::Control.into(), true),
        "Alt" => (Key::Alt.into(), true),
        "Space" => (" ".into(), false),
        "Esc" => (Key::Escape.into(), false),
        "←" => (Key::LeftArrow.into(), false),
        "→" => (Key::RightArrow.into(), false),
        "↑" => (Key::UpArrow.into(), false),
        "↓" => (Key::DownArrow.into(), false),
        other => (other.to_lowercase().into(), false),
    }
}

/// Presses `caps` together: the modifiers held, then the key, then all released.
fn press(w: &slint::Window, caps: &[&str]) {
    let keys: Vec<_> = caps.iter().map(|c| key(c).0).collect();
    for k in &keys {
        w.dispatch_event(WindowEvent::KeyPressed { text: k.clone() });
    }
    for k in keys.iter().rev() {
        w.dispatch_event(WindowEvent::KeyReleased { text: k.clone() });
    }
}

/// Each way to press a row: its keys, or each side of an empty one ("or").
fn ways(row: &ShortcutRow) -> Vec<Vec<String>> {
    let caps: Vec<String> = row.keys.iter().map(|k| k.to_string()).collect();
    caps.split(|c| c.is_empty())
        .map(<[String]>::to_vec)
        .collect()
}

#[test]
fn every_listed_shortcut_is_taken_as_listed() {
    let scene = scene("home");
    let w = scene.ui.window();
    let shortcuts = scene.ui.global::<Shortcuts>();
    let used = Rc::new(RefCell::new(Vec::new()));
    let seen = used.clone();
    shortcuts.on_used(move |kind| seen.borrow_mut().push(kind));

    let rows: Vec<ShortcutRow> = shortcuts
        .get_playback()
        .iter()
        .chain(shortcuts.get_moving().iter())
        .collect();
    assert!(rows.len() >= 12, "the list lost rows: {}", rows.len());
    for row in &rows {
        for way in ways(row) {
            let caps: Vec<&str> = way.iter().map(String::as_str).collect();
            used.borrow_mut().clear();
            press(w, &caps);
            assert_eq!(
                used.borrow().first(),
                Some(&row.kind),
                "{caps:?} ({})",
                row.label
            );
            // Back where it started: Home, the keys with the window, no dialog.
            scene.ui.global::<SettingsData>().invoke_close_dialog();
            scene.open("home").expect("home");
            scene.settle();
            press(w, &["Esc"]);
        }
    }
}

#[test]
fn ctrl_slash_opens_the_list_on_any_page_and_escape_closes_it() {
    let scene = scene("album");
    let w = scene.ui.window();
    let data = scene.ui.global::<SettingsData>();
    press(w, &["Ctrl", "/"]);
    scene.settle();
    assert_eq!(data.get_dialog(), SettingsDialog::Shortcuts);
    // It keeps the keys: Ctrl+K doesn't reach the search field behind it, so Escape still
    // closes it.
    press(w, &["Ctrl", "K"]);
    press(w, &["Esc"]);
    assert_eq!(data.get_dialog(), SettingsDialog::None);

    // Not from the search field: the key types there.
    press(w, &["Ctrl", "K"]);
    press(w, &["Ctrl", "/"]);
    assert_eq!(data.get_dialog(), SettingsDialog::None);
}
