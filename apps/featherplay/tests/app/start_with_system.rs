//! Settings > Window's first rows, offscreen (no window is shown), on an entry of the test's
//! own (`Autostart::for_tests`: never the one the system runs): the switch adds and removes
//! the entry, "Start in the tray" shows only while it's on, and an entry the system turned
//! off reads as off when Settings opens.

use featherplay::pages::settings;
use featherplay::snapshot::Scene;
use featherplay::ui::{SettingKey, SettingsData};
use featherplay_platform::autostart::{Autostart, State};
use slint::ComponentHandle;

/// The test's entry, deleted on drop.
struct Entry(Autostart);

impl Drop for Entry {
    fn drop(&mut self) {
        self.0.remove_for_tests();
    }
}

fn scene(name: &str) -> (Scene, Entry) {
    let entry = Entry(Autostart::for_tests(name));
    entry.0.remove_for_tests();
    settings::use_autostart_entry(entry.0.clone());
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("settings").expect("settings");
    scene.settle();
    (scene, entry)
}

#[test]
fn the_switch_adds_and_removes_the_entry() {
    let (scene, entry) = scene("settings-switch");
    let data = scene.ui.global::<SettingsData>();
    assert!(!data.get_start_with_system(), "off by default");

    data.invoke_toggled(SettingKey::StartWithSystem, true);
    assert!(data.get_start_with_system());
    assert_eq!(entry.0.state(), State::On);
    let command = entry.0.command().expect("the entry's command");
    assert!(command.ends_with(" --autostart"), "{command}");

    data.invoke_toggled(SettingKey::StartInTray, true);
    assert!(data.get_start_in_tray());

    data.invoke_toggled(SettingKey::StartWithSystem, false);
    assert!(!data.get_start_with_system());
    assert_eq!(entry.0.state(), State::Off);
}

#[test]
fn an_entry_the_system_turned_off_reads_as_off() {
    let (scene, entry) = scene("settings-turned-off");
    let data = scene.ui.global::<SettingsData>();
    data.invoke_toggled(SettingKey::StartWithSystem, true);
    entry.0.turn_off_for_tests().expect("turn off");

    // Settings opens again: it reads the entry.
    scene.open("home").expect("home");
    scene.settle();
    scene.open("settings").expect("settings");
    scene.settle();
    assert!(!data.get_start_with_system());
    assert_eq!(
        entry.0.state(),
        State::TurnedOffBySystem,
        "featherplay left the system's choice alone"
    );
}
