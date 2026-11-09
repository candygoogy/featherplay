//! A first run in the window (offscreen, no window is shown): while the helpers download,
//! the controller's toast says why the first song takes a moment. The download itself, and
//! resolves waiting for it, are tested in `featherplay-streams` (`ytdlp::tests::first_run`).

use featherplay::app;
use featherplay::controller::UiEvent;
use featherplay::snapshot::Scene;
use featherplay::ui::{Toast, ToastTone};
use featherplay_core::failure::{HelperFault, Notice, getting_ready};
use slint::ComponentHandle;

#[test]
fn the_helpers_download_shows_why_the_song_waits() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("home").expect("home");
    scene.settle();

    app::on_event(UiEvent::Toast(getting_ready()));

    let toast = scene.ui.global::<Toast>();
    assert!(toast.get_open());
    assert_eq!(
        toast.get_text().as_str(),
        "Getting ready to play: downloading helpers (one time)…"
    );
    assert_eq!(toast.get_action().as_str(), "", "nothing to click");
}

/// The toast says why, in plain words, and offers what a bug report needs.
#[test]
fn a_failed_helpers_download_says_why_with_copy_diagnostics() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("home").expect("home");
    scene.settle();

    app::on_event(UiEvent::HelpersFailed(
        Notice::HelpersFailed(HelperFault::Blocked).text(),
    ));

    let toast = scene.ui.global::<Toast>();
    assert!(toast.get_open());
    assert_eq!(
        toast.get_text().as_str(),
        "Your antivirus may have blocked the helpers."
    );
    assert_eq!(toast.get_tone(), ToastTone::Error, "a stop is an error");
    assert_eq!(toast.get_action().as_str(), "Copy diagnostics");
}
