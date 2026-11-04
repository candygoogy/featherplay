//! A left click on the tray icon, on the offscreen platform (no window is shown): it opens
//! and closes the mini player by default; with "Tray click opens the main window" on, it
//! brings the main window back from the tray instead, and the mini player stays shut.

use featherplay::mini;
use featherplay::shell;
use featherplay::snapshot::Scene;
use featherplay::ui::{Frame, SettingKey, SettingsData};
use slint::ComponentHandle;

/// A click on the primary screen, as the tray reports it (physical pixels).
const CLICK: (i32, i32) = (200, 200);

/// The window on the Settings page with the switch set to `on`, whatever the dev home's
/// settings say.
fn scene(on: bool) -> Scene {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("settings").expect("settings");
    scene.settle();
    scene
        .ui
        .global::<SettingsData>()
        .invoke_toggled(SettingKey::TrayClickOpensWindow, on);
    scene
}

/// Hides the main window to the tray, as "Keep playing in the tray" does.
fn hide_to_tray(scene: &Scene) {
    scene.ui.global::<Frame>().set_hidden(true);
    scene.ui.hide().expect("hide");
    assert!(!scene.ui.window().is_visible());
}

#[test]
fn the_switch_sets_the_setting() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("settings").expect("settings");
    scene.settle();
    let data = scene.ui.global::<SettingsData>();
    data.invoke_toggled(SettingKey::TrayClickOpensWindow, true);
    assert!(data.get_tray_click_opens_window());
    assert!(featherplay::pages::settings::tray_click_opens_window());
    data.invoke_toggled(SettingKey::TrayClickOpensWindow, false);
    assert!(!data.get_tray_click_opens_window());
    assert!(!featherplay::pages::settings::tray_click_opens_window());
}

#[test]
fn off_a_tray_click_toggles_the_mini_player() {
    let scene = scene(false);
    hide_to_tray(&scene);

    shell::tray_click(CLICK.0, CLICK.1);
    scene.settle();
    assert!(mini::is_open(), "the click didn't open the mini player");
    assert!(!scene.ui.window().is_visible(), "the main window came back");

    shell::tray_click(CLICK.0, CLICK.1);
    scene.settle();
    assert!(!mini::is_open(), "a second click didn't close it");
}

#[test]
fn on_a_tray_click_shows_the_main_window_and_not_the_mini_player() {
    let scene = scene(true);
    hide_to_tray(&scene);

    shell::tray_click(CLICK.0, CLICK.1);
    scene.settle();
    assert!(scene.ui.window().is_visible(), "the window didn't show");
    assert!(!scene.ui.global::<Frame>().get_hidden());
    assert!(!mini::is_open(), "the mini player opened");

    // Clicked again with the window up, it stays up.
    shell::tray_click(CLICK.0, CLICK.1);
    scene.settle();
    assert!(scene.ui.window().is_visible());
    assert!(!mini::is_open());
}
