//! The toasts the controller's events raise (`app::on_event`), one after another on Home
//! (offscreen, no window is shown): each one's words, its button and its tone, on one line
//! that fits the pill. The words themselves are pinned beside the functions that make them
//! (`featherplay-core`'s `failure`, the controller's `notice`); that the helpers' download
//! and a missing output device are reported is tested in `featherplay-streams` and
//! `featherplay-audio`.

use featherplay::controller::{UiEvent, no_sound_output};
use featherplay::snapshot::Scene;
use featherplay::ui::{SettingsData, Toast, ToastTone};
use featherplay::updates::Help;
use featherplay::{app, pages, toast};
use featherplay_core::failure::{HelperFault, Notice, getting_ready, songs_refused_by_network};
use i_slint_core::items::AccessibleRole;
use slint::ComponentHandle;

use crate::support::label;

/// The width of the toast, which is as wide as its line and its button up to 480 px.
fn toast_width(scene: &Scene) -> f32 {
    let line = scene.ui.global::<Toast>().get_text();
    let text = label::any_where(scene, |item, l| {
        l == line.as_str() && item.accessible_role() == AccessibleRole::Text
    });
    text.first().expect("the toast").geometry().size.width
}

#[test]
fn each_event_toasts_its_words_with_its_button_and_tone_on_one_line() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("home").expect("home");
    scene.settle();
    let toast = scene.ui.global::<Toast>();
    let blocked = Notice::HelpersFailed(HelperFault::Blocked).text();

    // (case, the event, its line, its button, its tone)
    type Case<'a> = (&'a str, fn() -> UiEvent, &'a str, &'a str, ToastTone);
    let cases: [Case; 5] = [
        (
            "the helpers' first download says why the song waits",
            || UiEvent::Toast(getting_ready()),
            getting_ready(),
            "",
            ToastTone::Neutral,
        ),
        (
            "a failed helpers' download says why, with what a bug report needs",
            || UiEvent::HelpersFailed(Notice::HelpersFailed(HelperFault::Blocked).text()),
            blocked,
            "Copy diagnostics",
            ToastTone::Error,
        ),
        (
            "play with no output device says how to fix it",
            || UiEvent::ToastError(no_sound_output()),
            "No sound output found. Connect speakers or headphones.",
            "",
            ToastTone::Error,
        ),
        (
            "a stop shows its words and the link found",
            || UiEvent::Stopped {
                text: "Songs can't play. featherplay may need an update.",
                help: Some(Help {
                    label: "Details",
                    url: "https://github.com/candygoogy/featherplay/issues/42".into(),
                    restart: false,
                }),
            },
            "Songs can't play. featherplay may need an update.",
            "Details",
            ToastTone::Error,
        ),
        (
            "a stop with nothing to link has no button",
            || UiEvent::Stopped {
                text: "This song isn't available.",
                help: None,
            },
            "This song isn't available.",
            "",
            ToastTone::Error,
        ),
    ];
    for (case, event, line, action, tone) in cases {
        app::on_event(event());
        scene.settle();
        assert!(toast.get_open(), "{case}: no toast");
        assert_eq!(toast.get_text().as_str(), line, "{case}");
        assert_eq!(toast.get_action().as_str(), action, "{case}: the button");
        assert_eq!(toast.get_tone(), tone, "{case}: the tone");
        // The line and the button share the pill's 480 px, the button keeping its own.
        let width = toast_width(&scene);
        assert!(
            width < 480.0,
            "{case}: the line is cut short in a {width} px pill"
        );
    }

    // After an error, the next confirmation is plain again.
    app::on_event(UiEvent::ToastError(no_sound_output()));
    toast::show(&scene.ctx, "Added to queue: Da Funk");
    assert_eq!(
        toast.get_tone(),
        ToastTone::Neutral,
        "the error tone stayed"
    );

    // Songs refused while the DNS sends YouTube's hosts away from Google: the button turns
    // on "Look up YouTube directly" (Settings > Playback).
    assert!(!pages::settings::dns_over_https(), "off by default");
    app::on_event(UiEvent::RefusedByDns(songs_refused_by_network()));
    scene.settle();
    assert!(toast.get_open(), "the DNS reason: no toast");
    assert_eq!(toast.get_text().as_str(), songs_refused_by_network());
    assert_eq!(toast.get_tone(), ToastTone::Error);
    assert_eq!(toast.get_action().as_str(), "Look up directly");
    let width = toast_width(&scene);
    assert!(
        width < 480.0,
        "the DNS reason is cut short in a {width} px pill"
    );
    toast.invoke_action_clicked();
    scene.settle();
    assert!(!toast.get_open());
    assert!(pages::settings::dns_over_https());
    assert!(scene.ui.global::<SettingsData>().get_dns_over_https());
}
