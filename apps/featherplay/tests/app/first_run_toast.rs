//! A first run in the window (offscreen, no window is shown): while the helpers download,
//! the controller's toast says why the first song takes a moment. The download itself, and
//! resolves waiting for it, are tested in `featherplay-streams` (`ytdlp::tests::first_run`).

use featherplay::app;
use featherplay::controller::UiEvent;
use featherplay::snapshot::Scene;
use featherplay::ui::Toast;
use featherplay_core::failure::{Notice, getting_ready};
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

#[test]
fn a_failed_helpers_download_says_so_in_plain_words() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("home").expect("home");
    scene.settle();

    app::on_event(UiEvent::Stopped {
        text: Notice::HelpersFailed.text(),
        help: None,
    });

    let toast = scene.ui.global::<Toast>();
    assert!(toast.get_open());
    assert_eq!(
        toast.get_text().as_str(),
        "Couldn't download helpers. Check your connection."
    );
}
