//! What draws the window (Settings › Window › Performance): the chips pick the graphics card
//! or the processor; with the graphics card unusable a row says the processor draws, with
//! "Why?"; a hidden window keeps what a quick restore draws with (its GPU surface, the shown
//! page's covers) for `memory::KEEP_FOR_RESTORE`, then lets it go, and shown again before that,
//! it keeps it (in a `gpu` build: run these with `--features gpu` too).

use std::time::Duration;

use featherplay::memory::{self, KEEP_FOR_RESTORE};
use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::SettingsData;
use i_slint_core::accessibility::AccessibilityAction;
use i_slint_core::items::AccessibleRole;
use slint::ComponentHandle;

use crate::support::label;

const FALLBACK: &str = "Drawing with the processor for now: the graphics card couldn't be used.";
const PROCESSOR: &str = "Drawing with the processor";

/// The elements labelled `name`, below the fold or not (the page isn't scrolled).
fn made(scene: &Scene, name: &str) -> usize {
    label::any_where(scene, |_, l| l == name).len()
}

/// Presses the chip or button labelled `name`, as a screen reader does.
fn press(scene: &Scene, name: &str) {
    let pressable = |role| matches!(role, AccessibleRole::Tab | AccessibleRole::Button);
    let items = label::any_where(scene, |i, l| l == name && pressable(i.accessible_role()));
    assert_eq!(items.len(), 1, "{name}");
    items[0].accessible_action(&AccessibilityAction::Default);
    scene.settle();
}

#[test]
fn the_chips_pick_what_draws_and_a_fallback_says_why() {
    let scene = Scene::new(1100, 900, true, false).expect("scene");
    scene.open("settings").expect("settings");
    scene.settle();
    // The Window section's rows are made a tick each after the page's first frame.
    for _ in 0..8 {
        scene.ui.window().take_snapshot().expect("snapshot");
        std::thread::sleep(Duration::from_millis(2));
        scene.settle_once();
    }
    let data = scene.ui.global::<SettingsData>();
    if !cfg!(windows) {
        assert_eq!(
            made(&scene, "Draw with"),
            0,
            "Linux draws with the processor"
        );
        return;
    }
    assert!(!data.get_processor_draws(), "the graphics card by default");

    // The status line says what the window's last surface drew with, as the renderer tells
    // it (`app::drawn_with`; offscreen nothing does): the card's name, trademark signs out.
    featherplay::app::drawn_with(Some("Intel(R) UHD Graphics 770"));
    scene.settle();
    assert_eq!(
        made(
            &scene,
            "Drawing with the graphics card (Intel UHD Graphics 770)"
        ),
        1
    );
    featherplay::app::drawn_with(Some(""));
    scene.settle();
    assert_eq!(made(&scene, "Drawing with the graphics card"), 1);

    press(&scene, "Processor");
    assert!(data.get_processor_draws());
    featherplay::app::drawn_with(None);
    scene.settle();
    assert_eq!(made(&scene, PROCESSOR), 1);
    press(&scene, "Graphics card");
    assert!(!data.get_processor_draws());

    // The graphics card picked but unusable: the pick stays, the one line says the processor
    // draws for now, with "Why?".
    assert_eq!(made(&scene, FALLBACK), 0);
    data.set_drawing_fallback(true);
    scene.settle();
    assert_eq!(made(&scene, FALLBACK), 1);
    assert_eq!(made(&scene, PROCESSOR), 0, "one line, not two");
    press(&scene, "Why?");
    assert!(data.get_drawing_why());
    assert!(!label::any_where(&scene, |_, l| l.contains("Remote Desktop")).is_empty());
    // The processor picked: it draws by choice, and the line says so.
    press(&scene, "Processor");
    assert_eq!(made(&scene, FALLBACK), 0);
    assert_eq!(made(&scene, PROCESSOR), 1);
    assert!(label::any_where(&scene, |_, l| l.contains("Remote Desktop")).is_empty());
}

/// The window minimised, or shown again, as the app's WM_SIZE hook reports it.
fn hide(scene: &Scene, hidden: bool) {
    scene.ui.window().set_minimized(hidden);
    memory::hidden(&scene.ctx, hidden);
}

fn wait(by: Duration) {
    offscreen::advance(by);
}

#[test]
fn a_hidden_window_keeps_what_a_restore_draws_with_for_thirty_seconds() {
    let scene = Scene::new(1100, 800, true, false).expect("scene");
    scene.ui.show().expect("show");
    scene.settle();
    offscreen::freeze_clock();
    let second = Duration::from_secs(1);

    // In either renderer: the shown page's covers are kept too.
    hide(&scene, true);
    assert!(memory::keeps_for_restore(), "hidden: kept");
    wait(KEEP_FOR_RESTORE - second);
    assert!(
        memory::keeps_for_restore(),
        "just under the time: still kept"
    );
    wait(2 * second);
    assert!(!memory::keeps_for_restore(), "after the time: let go");

    // Shown again before the time: kept, and a later hide counts from its own start.
    hide(&scene, false);
    hide(&scene, true);
    wait(KEEP_FOR_RESTORE / 2);
    hide(&scene, false);
    assert!(
        !memory::keeps_for_restore(),
        "shown: nothing waits to let go"
    );
    hide(&scene, true);
    wait(KEEP_FOR_RESTORE - second);
    assert!(
        memory::keeps_for_restore(),
        "hidden again: the time starts over"
    );
    wait(2 * second);
    assert!(!memory::keeps_for_restore());
    hide(&scene, false);
}
