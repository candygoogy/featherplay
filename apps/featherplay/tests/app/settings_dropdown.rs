//! A setting's dropdown (the output device here): a click opens its menu under it, each
//! device's adapter under its name, a check on the pick; the keyboard moves from the pick and
//! Enter picks; Escape and a click outside close it, and the keys come back to the dropdown,
//! where Enter opens it again.

use featherplay::snapshot::Scene;
use featherplay::ui::{ChoiceMenu, SettingId, SettingsData};
use i_slint_core::accessibility::AccessibleStringProperty;
use i_slint_core::items::AccessibleRole;
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, SharedString};

use crate::support::{self, label};

fn tap(scene: &Scene, text: SharedString) {
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    w.dispatch_event(WindowEvent::KeyReleased { text });
    scene.settle();
}

/// The output device's dropdown, and the value it says.
fn dropdown(scene: &Scene) -> (label::ItemRc, String) {
    let item = label::shown_where(scene, |item, l| {
        l == "Output device" && item.accessible_role() == AccessibleRole::Combobox
    })
    .into_iter()
    .next()
    .expect("the output device's dropdown");
    let value = item
        .accessible_string_property(AccessibleStringProperty::Value)
        .unwrap_or_default()
        .to_string();
    (item, value)
}

#[test]
fn a_dropdown_opens_its_menu_and_picks_by_click_or_key() {
    let scene = Scene::new(1920, 2200, true, false).expect("scene");
    support::resize(scene.ui.window(), 1920, 2200);
    scene.play(false).expect("player");
    scene.open("settings").expect("settings");
    scene.settle();
    let menu = scene.ui.global::<ChoiceMenu>();
    let data = scene.ui.global::<SettingsData>();

    let (item, value) = dropdown(&scene);
    assert_eq!(value, "System default");
    let c = label::rect(&item).center();
    support::click(&scene, LogicalPosition::new(c.x, c.y));
    scene.settle();
    assert!(menu.get_open() && menu.get_setting() == SettingId::OutputDevice);
    // Name and adapter, one choice to a screen reader; the menu sits under the dropdown.
    let speakers = label::find(&scene, "Speakers, Realtek(R) Audio").expect("Speakers' row");
    assert_eq!(speakers.accessible_role(), AccessibleRole::RadioButton);
    assert!(label::rect(&speakers).min_y() > label::rect(&item).max_y());

    // A click outside closes it, picking nothing.
    support::click(&scene, LogicalPosition::new(300.0, 300.0));
    scene.settle();
    assert!(!menu.get_open(), "a click outside");
    assert_eq!(data.get_device_index(), 0);

    // The keys came back to the dropdown: Enter opens it on the pick, Down and Enter pick
    // the next one.
    tap(&scene, Key::Return.into());
    assert!(menu.get_open(), "Enter on the dropdown");
    assert_eq!(menu.get_focused(), 0);
    tap(&scene, Key::DownArrow.into());
    tap(&scene, Key::Return.into());
    assert!(!menu.get_open());
    assert_eq!(data.get_device_index(), 1);
    assert_eq!(dropdown(&scene).1, "Speakers");

    // Escape closes it, the keys still on the dropdown.
    tap(&scene, Key::Return.into());
    tap(&scene, Key::Escape.into());
    assert!(!menu.get_open(), "Escape");
    tap(&scene, Key::Return.into());
    assert!(menu.get_open(), "the keys didn't come back");
}
