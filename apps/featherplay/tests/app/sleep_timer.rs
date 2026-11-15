//! The sleep timer in the player bar, on the offscreen platform (no window is shown): off by
//! default there's no button; turned on, the button opens the popup, a chip there starts the
//! timer and closes it; running, the button reads as the time left, and Turn off in the
//! popup asks the controller to turn it off.

use std::time::Duration;

use featherplay::app;
use featherplay::controller::{Action, SleepAction, SleepKind, SleepState, UiEvent};
use featherplay::snapshot::Scene;
use featherplay::ui::{SettingsData, Sleep};
use slint::{ComponentHandle, LogicalPosition};

use crate::support::{self, label};

/// The centre of the visible element named `name`, the lowest one if several are.
fn centre(scene: &Scene, name: &str) -> Option<LogicalPosition> {
    label::rects(scene, name)
        .iter()
        .map(|r| LogicalPosition::new(r.origin.x + r.width() / 2.0, r.origin.y + r.height() / 2.0))
        .reduce(|low, c| if c.y > low.y { c } else { low })
}

fn click(scene: &Scene, at: LogicalPosition) {
    support::click(scene, at);
    scene.settle();
}

fn playing() -> Scene {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.play(false).expect("player");
    scene.settle();
    scene
}

#[test]
fn the_button_shows_only_with_its_setting_and_a_chip_starts_the_timer() {
    let scene = playing();
    assert!(centre(&scene, "Sleep timer").is_none(), "there by default");

    scene.ui.global::<SettingsData>().set_sleep_timer(true);
    scene.settle();
    let button = centre(&scene, "Sleep timer").expect("no button with the setting on");
    click(&scene, button);
    assert!(
        scene.ui.global::<Sleep>().get_open(),
        "the popup didn't open"
    );

    scene.take_actions();
    let chip = centre(&scene, "30 min").expect("no 30 min chip");
    click(&scene, chip);
    let sent = scene.take_actions();
    assert!(
        sent.iter()
            .any(|a| matches!(a, Action::Sleep(SleepAction::Minutes(30)))),
        "{sent:?}"
    );
    assert!(
        !scene.ui.global::<Sleep>().get_open(),
        "the pick left it open"
    );
}

#[test]
fn running_the_button_reads_as_the_time_left_and_turn_off_sends_off() {
    let scene = playing();
    scene.ui.global::<SettingsData>().set_sleep_timer(true);
    let state = |fading| {
        UiEvent::Sleep(Some(SleepState {
            kind: SleepKind::Time,
            minutes: 24,
            fading,
            left: Duration::from_secs(24 * 60),
        }))
    };
    app::on_event(state(false));
    scene.settle();
    assert!(
        centre(&scene, "24 min left").is_some(),
        "running, the button doesn't read as the time left"
    );

    // Turn off in the popup asks the controller to turn it off, and closes the popup.
    let button = centre(&scene, "Sleep timer").expect("no button");
    click(&scene, button);
    assert!(
        scene.ui.global::<Sleep>().get_open(),
        "the popup didn't open"
    );
    scene.take_actions();
    let off = centre(&scene, "Turn off").expect("no Turn off");
    click(&scene, off);
    let sent = scene.take_actions();
    assert!(
        sent.iter()
            .any(|a| matches!(a, Action::Sleep(SleepAction::Off))),
        "Turn off didn't send off: {sent:?}"
    );
    assert!(
        !scene.ui.global::<Sleep>().get_open(),
        "Turn off left it open"
    );

    // Fading, the button says so; once the controller says it's off, the plain button is back.
    app::on_event(state(true));
    scene.settle();
    assert!(
        centre(&scene, "Fading out").is_some(),
        "fading, the button doesn't say so"
    );
    app::on_event(UiEvent::Sleep(None));
    scene.settle();
    assert!(
        centre(&scene, "Fading out").is_none(),
        "off, the button still says it's fading"
    );
    assert!(
        centre(&scene, "Sleep timer").is_some(),
        "the button went with the timer"
    );
}
