//! "Not for me" in the window (offscreen, no window is shown): the ⊘ shows the toast with
//! Undo, and Undo closes it. What the actions do to the player is `tests/not_for_me.rs`.

use featherplay::snapshot::Scene;
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
