//! Songs YouTube refuses while the DNS sends its hosts away from Google: the toast says why
//! (`featherplay_core::failure::songs_refused_by_network`), on one line, and its button turns
//! on "Look up YouTube directly" (Settings > Playback). Which text the controller picks is
//! tested beside it (`controller::notice`).

use std::ops::ControlFlow;

use featherplay::app;
use featherplay::controller::UiEvent;
use featherplay::pages;
use featherplay::snapshot::Scene;
use featherplay::ui::{SettingKey, SettingsData, Toast, ToastTone};
use featherplay_core::failure::songs_refused_by_network;
use i_slint_core::accessibility::AccessibleStringProperty;
use i_slint_core::item_tree::ItemRc;
use i_slint_core::items::AccessibleRole;
use i_slint_core::window::WindowInner;
use slint::ComponentHandle;

/// The width of the toast, which is as wide as its line and its button up to 480 px.
fn line_width(scene: &Scene) -> f32 {
    let line = scene.ui.global::<Toast>().get_text();
    let root = ItemRc::new_root(WindowInner::from_pub(scene.ui.window()).component());
    let mut width = None;
    root.visit_descendants(|item| {
        let label = item.accessible_string_property(AccessibleStringProperty::Label);
        if item.accessible_role() == AccessibleRole::Text && label.is_some_and(|l| l == line) {
            width = Some(item.geometry().size.width);
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    });
    width.expect("the toast")
}

#[test]
fn the_dns_reason_fits_the_toast_and_its_button_looks_up_youtube_directly() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("home").expect("home");
    scene.settle();
    assert!(!pages::settings::dns_over_https(), "off by default");

    app::on_event(UiEvent::RefusedByDns(songs_refused_by_network()));
    scene.settle();

    let toast = scene.ui.global::<Toast>();
    assert!(toast.get_open());
    assert_eq!(toast.get_tone(), ToastTone::Error);
    assert_eq!(toast.get_action().as_str(), "Look up directly");
    // The line and the button share the pill's 480 px, the button keeping its own.
    let width = line_width(&scene);
    assert!(width < 480.0, "the line is cut short in a {width} px pill");

    toast.invoke_action_clicked();
    scene.settle();
    assert!(!toast.get_open());
    assert!(pages::settings::dns_over_https());
    assert!(scene.ui.global::<SettingsData>().get_dns_over_https());
}

#[test]
fn look_up_youtube_directly_is_a_setting_off_by_default() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("settings").expect("settings");
    scene.settle();
    let data = scene.ui.global::<SettingsData>();
    assert!(!data.get_dns_over_https());

    data.invoke_toggled(SettingKey::DnsOverHttps, true);
    assert!(data.get_dns_over_https());
    assert!(pages::settings::dns_over_https());
    data.invoke_toggled(SettingKey::DnsOverHttps, false);
    assert!(!pages::settings::dns_over_https());
}
