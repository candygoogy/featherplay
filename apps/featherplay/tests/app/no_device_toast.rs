//! Play with no output device (the speakers switched off): the controller's toast says what
//! to do instead of the app quitting. That the engine starts and reports it is tested in
//! `featherplay-audio` (`stream_tests::engine_play_with_no_output_device_says_so`).

use featherplay::app;
use featherplay::controller::{UiEvent, no_sound_output};
use featherplay::snapshot::Scene;
use featherplay::toast;
use featherplay::ui::{Toast, ToastTone};
use slint::ComponentHandle;

#[test]
fn play_with_no_output_device_says_how_to_fix_it() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("home").expect("home");
    scene.settle();

    app::on_event(UiEvent::ToastError(no_sound_output()));

    let toast = scene.ui.global::<Toast>();
    assert!(toast.get_open());
    assert_eq!(
        toast.get_text().as_str(),
        "No sound output found. Connect speakers or headphones."
    );
    assert_eq!(toast.get_action().as_str(), "", "nothing to click");
}

/// Something went wrong, so the toast wears the error tone (the alert ring before the line);
/// the next confirmation is plain again.
#[test]
fn no_output_device_is_an_error_toast_and_the_next_toast_is_plain() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("home").expect("home");
    scene.settle();

    app::on_event(UiEvent::ToastError(no_sound_output()));
    let shown = scene.ui.global::<Toast>();
    assert_eq!(shown.get_tone(), ToastTone::Error);

    toast::show(&scene.ctx, "Added to queue: Da Funk");
    assert_eq!(
        shown.get_tone(),
        ToastTone::Neutral,
        "the error tone stayed"
    );
}
