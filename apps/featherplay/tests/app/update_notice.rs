//! The update notices in the window (offscreen, fixture data, no request): a stop for
//! failures shows plain words in the toast, with the link the controller found; a newer
//! release puts a banner at the top of About; Settings has "Check for updates".

use featherplay::controller::UiEvent;
use featherplay::snapshot::Scene;
use featherplay::ui::{AboutData, SettingKey, SettingsData, Toast};
use featherplay::updates::{Help, Release};
use featherplay::{app, pages, updates};
use slint::{ComponentHandle, Rgba8Pixel, SharedPixelBuffer};

fn release() -> Release {
    Release {
        version: "0.2.0".into(),
        url: "https://github.com/candygoogy/featherplay/releases/tag/v0.2.0".into(),
        assets: Vec::new(),
    }
}

#[test]
fn a_stop_shows_its_words_and_the_link_found() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("album").expect("album");
    scene.settle();
    let toast = scene.ui.global::<Toast>();

    app::on_event(UiEvent::Stopped {
        text: "Songs can't play. featherplay may need an update.",
        help: Some(Help {
            label: "Details",
            url: "https://github.com/candygoogy/featherplay/issues/42".into(),
        }),
    });
    assert!(toast.get_open());
    assert_eq!(
        toast.get_text().as_str(),
        "Songs can't play. featherplay may need an update."
    );
    assert_eq!(toast.get_action().as_str(), "Details");
}

#[test]
fn a_stop_with_nothing_to_link_has_no_button() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("album").expect("album");
    scene.settle();
    let toast = scene.ui.global::<Toast>();

    app::on_event(UiEvent::Stopped {
        text: "This song isn't available.",
        help: None,
    });
    assert!(toast.get_open());
    assert_eq!(toast.get_text().as_str(), "This song isn't available.");
    assert_eq!(toast.get_action().as_str(), "");
}

fn shot(scene: &Scene) -> SharedPixelBuffer<Rgba8Pixel> {
    scene.ui.window().take_snapshot().expect("snapshot")
}

#[test]
fn a_newer_release_puts_a_banner_at_the_top_of_about() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("about").expect("about");
    scene.settle();
    let about = scene.ui.global::<AboutData>();
    assert_eq!(about.get_update_version().as_str(), "", "no banner yet");
    let before = shot(&scene);

    app::on_event(UiEvent::Release(release()));
    scene.settle();
    assert_eq!(about.get_update_version().as_str(), "0.2.0");
    let after = shot(&scene);
    // The banner sits above the logo, so the top of the page changes.
    let width = before.width() as usize;
    let top = |s: &SharedPixelBuffer<Rgba8Pixel>| s.as_slice()[..width * 200].to_vec();
    assert_ne!(top(&before), top(&after), "the banner shows at the top");

    // An older or the same version clears it.
    pages::about::show_update(&scene.ctx, None);
    assert_eq!(about.get_update_version().as_str(), "");
}

#[test]
fn check_for_updates_is_a_setting_on_by_default() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("settings").expect("settings");
    scene.settle();
    let data = scene.ui.global::<SettingsData>();
    assert!(data.get_check_for_updates());
    assert!(pages::settings::check_for_updates());

    data.invoke_toggled(SettingKey::CheckForUpdates, false);
    assert!(!data.get_check_for_updates());
    assert!(!pages::settings::check_for_updates());
}

#[test]
fn install_updates_is_a_setting_on_by_default() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("settings").expect("settings");
    scene.settle();
    let data = scene.ui.global::<SettingsData>();
    assert!(data.get_install_updates());
    assert!(pages::settings::install_updates());

    data.invoke_toggled(SettingKey::InstallUpdates, false);
    assert!(!data.get_install_updates());
    assert!(!pages::settings::install_updates());
}

#[test]
fn a_downloaded_update_offers_restart_to_update() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("about").expect("about");
    scene.settle();
    let about = scene.ui.global::<AboutData>();
    let toast = scene.ui.global::<Toast>();

    updates::ready_for_snapshot(&scene.ctx);
    app::on_event(UiEvent::Release(release()));
    scene.settle();

    assert!(toast.get_open());
    assert_eq!(
        toast.get_text().as_str(),
        "featherplay 0.2.0 is ready to install"
    );
    assert_eq!(toast.get_action().as_str(), "Restart to update");
    assert_eq!(about.get_update_version().as_str(), "0.2.0");
    assert!(
        about.get_update_ready(),
        "the banner offers Restart to update"
    );
}

#[test]
fn a_newer_release_than_the_downloaded_one_is_not_offered_as_ready() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("about").expect("about");
    scene.settle();
    let about = scene.ui.global::<AboutData>();

    updates::ready_for_snapshot(&scene.ctx);
    let mut newer = release();
    newer.version = "0.3.0".into();
    app::on_event(UiEvent::Release(newer));
    scene.settle();

    assert_eq!(about.get_update_version().as_str(), "0.3.0");
    assert!(
        !about.get_update_ready(),
        "0.2.0's setup doesn't install 0.3.0"
    );
}
