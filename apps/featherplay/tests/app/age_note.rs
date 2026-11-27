//! A song marked for adults skipped (`UiEvent::AgeRestricted`), on Home offscreen: the first
//! time in a run a dialog explains it, signed out or with an account that isn't
//! age-verified, with its own button; a skip while it's open adds nothing; after it, or with
//! the window off screen, a toast says it. Each test is its own run (nextest).

use featherplay::app;
use featherplay::controller::UiEvent;
use featherplay::router::Page;
use featherplay::snapshot::Scene;
use featherplay::ui::{AgeNote, Frame, Toast};
use featherplay_core::failure::age_restricted;
use slint::ComponentHandle;
use slint::platform::{Key, WindowEvent};

use crate::support::{click, label};

const TITLE: &str = "This song is for adults";
const SIGN_IN_TEXT: &str = "Sign in to play songs that YouTube marks for adults.";
const VERIFY_TEXT: &str =
    "YouTube needs your Google account to confirm your age before it plays songs for adults.";

fn home() -> Scene {
    let scene = Scene::new(1100, 760, true, false).expect("scene");
    scene.ui.show().expect("show");
    scene.open("home").expect("home");
    scene.settle();
    scene
}

fn skipped(scene: &Scene, signed_in: bool) {
    app::on_event(UiEvent::AgeRestricted { signed_in });
    scene.settle();
}

/// The dialog's button labelled `label`: the dialog is drawn over everything, last.
fn press_in_dialog(scene: &Scene, name: &str) {
    let r = *label::rects(scene, name).last().expect(name);
    let c = r.center();
    click(scene, slint::LogicalPosition::new(c.x, c.y));
    scene.settle();
}

fn toast_says(scene: &Scene) -> Option<String> {
    let toast = scene.ui.global::<Toast>();
    toast.get_open().then(|| toast.get_text().to_string())
}

#[test]
fn signed_out_the_first_skip_asks_to_sign_in_then_a_toast_says_it() {
    let scene = home();
    let note = scene.ui.global::<AgeNote>();
    skipped(&scene, false);
    assert!(note.get_open(), "the first skip didn't open the dialog");
    for words in [TITLE, SIGN_IN_TEXT, "Sign in", "Not now"] {
        assert!(label::shows(&scene, words), "the dialog lacks {words:?}");
    }
    assert!(!label::shows(&scene, "Verify on YouTube"));
    assert_eq!(toast_says(&scene), None, "a toast came with the dialog");

    // Another skip while it's open: no second dialog, no toast, the same words.
    skipped(&scene, true);
    assert!(note.get_open() && !note.get_signed_in());
    assert_eq!(toast_says(&scene), None);

    // Escape closes it; the next skip, of either kind, is a toast.
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    w.dispatch_event(WindowEvent::KeyReleased {
        text: Key::Escape.into(),
    });
    scene.settle();
    assert!(!note.get_open(), "Escape didn't close the dialog");
    skipped(&scene, true);
    assert!(!note.get_open(), "the dialog came back in the same run");
    assert_eq!(toast_says(&scene).as_deref(), Some(age_restricted(true)));
}

#[test]
fn not_age_verified_the_first_skip_offers_youtubes_check_and_not_now_closes() {
    let scene = home();
    let note = scene.ui.global::<AgeNote>();
    skipped(&scene, true);
    assert!(note.get_open());
    for words in [TITLE, VERIFY_TEXT, "Verify on YouTube", "Not now"] {
        assert!(label::shows(&scene, words), "the dialog lacks {words:?}");
    }
    assert!(!label::shows(&scene, SIGN_IN_TEXT));
    press_in_dialog(&scene, "Not now");
    assert!(!note.get_open(), "Not now didn't close the dialog");
    assert_eq!(scene.ctx.current_page(), Page::Home);
    skipped(&scene, false);
    assert!(!note.get_open());
    assert_eq!(toast_says(&scene).as_deref(), Some(age_restricted(false)));
}

#[test]
fn off_screen_a_toast_says_it_and_the_dialog_waits_for_the_window() {
    let scene = home();
    let note = scene.ui.global::<AgeNote>();
    // In the tray: a toast, and the window isn't brought up.
    scene.ui.global::<Frame>().set_hidden(true);
    scene.ui.hide().expect("hide");
    skipped(&scene, false);
    assert!(
        !note.get_open(),
        "the dialog opened with the window in the tray"
    );
    assert!(
        !scene.ui.window().is_visible(),
        "the skip showed the window"
    );
    assert_eq!(toast_says(&scene).as_deref(), Some(age_restricted(false)));

    // Back on screen, the next skip explains it, and Sign in opens the sign-in page.
    scene.ui.global::<Frame>().set_hidden(false);
    scene.ui.show().expect("show");
    scene.ui.global::<Toast>().set_open(false);
    skipped(&scene, false);
    assert!(note.get_open());
    press_in_dialog(&scene, "Sign in");
    assert!(!note.get_open());
    assert_eq!(scene.ctx.current_page(), Page::SignIn);
}
