//! The sleep timer in the player bar, on the offscreen platform (no window is shown): off by
//! default there's no button; turned on, the button opens the popup, a chip there starts the
//! timer and closes it; running, the button reads as the time left.

use std::ops::ControlFlow;
use std::time::Duration;

use featherplay::app;
use featherplay::controller::{Action, SleepAction, SleepKind, SleepState, UiEvent};
use featherplay::snapshot::Scene;
use featherplay::ui::{SettingsData, Sleep};
use i_slint_core::accessibility::AccessibleStringProperty;
use i_slint_core::item_tree::ItemRc;
use i_slint_core::window::WindowInner;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition};

/// The centre of the visible element named `label`, the lowest one if several are.
fn centre(scene: &Scene, label: &str) -> Option<LogicalPosition> {
    let root = ItemRc::new_root(WindowInner::from_pub(scene.ui.window()).component());
    let mut lowest: Option<LogicalPosition> = None;
    root.visit_descendants(|item| {
        let named = item
            .accessible_string_property(AccessibleStringProperty::Label)
            .is_some_and(|l| l == label);
        if named && item.is_visible() {
            let g = item.geometry();
            let at = item.map_to_window(g.origin);
            let c = LogicalPosition::new(at.x + g.width() / 2.0, at.y + g.height() / 2.0);
            if lowest.is_none_or(|l| c.y > l.y) {
                lowest = Some(c);
            }
        }
        ControlFlow::<()>::Continue(())
    });
    lowest
}

fn click(scene: &Scene, at: LogicalPosition) {
    let w = scene.ui.window();
    let button = PointerEventButton::Left;
    w.dispatch_event(WindowEvent::PointerMoved { position: at });
    w.dispatch_event(WindowEvent::PointerPressed {
        position: at,
        button,
    });
    w.dispatch_event(WindowEvent::PointerReleased {
        position: at,
        button,
    });
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
fn running_the_button_reads_as_the_time_left() {
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
    assert!(centre(&scene, "24 min left").is_some());
    app::on_event(state(true));
    scene.settle();
    assert!(centre(&scene, "Fading out").is_some());
    app::on_event(UiEvent::Sleep(None));
    scene.settle();
    assert!(centre(&scene, "Fading out").is_none());
    assert!(
        centre(&scene, "Sleep timer").is_some(),
        "the button went with the timer"
    );
}
