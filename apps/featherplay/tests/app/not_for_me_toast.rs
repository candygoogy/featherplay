//! "Not for me" in the window (offscreen, no window is shown): the thumbs-down shows the
//! toast with Undo, and Undo closes it. What the actions do to the player is
//! `tests/not_for_me.rs`. Also any toast's button: it runs its action, then closes.

use std::cell::Cell;
use std::rc::Rc;

use featherplay::snapshot::Scene;
use featherplay::toast;
use featherplay::ui::{Player, Toast};
use slint::ComponentHandle;

#[test]
fn not_for_me_shows_a_toast_with_undo() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.play(false).expect("player");
    scene.open("home").expect("home");
    scene.settle();

    let player = scene.ui.global::<Player>();
    let toast = scene.ui.global::<Toast>();
    assert!(player.get_has_track());
    player.invoke_not_for_me();
    assert!(toast.get_open());
    assert_eq!(toast.get_text().as_str(), "Marked as not for me");
    assert_eq!(toast.get_action().as_str(), "Undo");
    assert!(!player.get_liked(), "the heart shows no like after Dislike");

    toast.invoke_action_clicked();
    assert!(!toast.get_open());
}

/// "Restart to update", "Undo", "View", "Sign in": the button runs what it says, once, and
/// the toast closes.
#[test]
fn a_toasts_button_runs_its_action() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("home").expect("home");
    scene.settle();

    let ran = Rc::new(Cell::new(0));
    let r = ran.clone();
    toast::show_with(
        &scene.ctx,
        "featherplay 9.9.9 is ready to install",
        "Restart to update",
        move |_| {
            r.set(r.get() + 1);
        },
    );
    let shown = scene.ui.global::<Toast>();
    assert!(shown.get_open());
    shown.invoke_action_clicked();
    assert_eq!(ran.get(), 1, "the button didn't run its action");
    assert!(!shown.get_open(), "the toast stayed open");
    shown.invoke_action_clicked();
    assert_eq!(ran.get(), 1, "a second click ran it again");
}
