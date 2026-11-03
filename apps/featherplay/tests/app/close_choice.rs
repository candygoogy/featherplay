//! The window's close button on the offscreen platform (no window is shown): set to Ask, a
//! close request opens the close dialog and keeps the window; Escape and a click on the scrim
//! cancel; "Keep playing in the tray" with "Remember my choice" checked saves Tray and hides
//! the window; unchecked, it hides without saving; Enter picks Close featherplay. Once set,
//! a close does what was saved without asking.

use featherplay::close;
use featherplay::pages::settings;
use featherplay::snapshot::Scene;
use featherplay::ui::{CloseChoice, Frame, SettingsData};
use featherplay_store::CloseAction;
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, SharedString};

fn key(w: &slint::Window, k: impl Into<SharedString>) {
    let text = k.into();
    w.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    w.dispatch_event(WindowEvent::KeyReleased { text });
}

fn click(w: &slint::Window, x: f32, y: f32) {
    let position = LogicalPosition::new(x, y);
    w.dispatch_event(WindowEvent::PointerMoved { position });
    for pressed in [true, false] {
        let button = PointerEventButton::Left;
        w.dispatch_event(if pressed {
            WindowEvent::PointerPressed { position, button }
        } else {
            WindowEvent::PointerReleased { position, button }
        });
    }
}

/// Sets the close action back to Ask through the Settings page, then shows the window again
/// as the tray's Show would.
fn ask_again(scene: &Scene) {
    scene
        .ui
        .global::<SettingsData>()
        .invoke_pick_close_action(0);
    assert_eq!(settings::close_action(), CloseAction::Ask);
    scene.ui.global::<Frame>().set_hidden(false);
    scene.ui.show().expect("show");
}

fn request_close(scene: &Scene) {
    scene
        .ui
        .window()
        .dispatch_event(WindowEvent::CloseRequested);
}

/// One test: the platform can be installed once per process.
#[test]
fn closing_asks_until_a_choice_is_remembered() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("home").expect("home");
    scene.settle();
    close::install(&scene.ctx, &scene.ui, || true);
    let w = scene.ui.window();
    let choice = scene.ui.global::<CloseChoice>();
    // Whatever the dev home's settings say, start from the default.
    ask_again(&scene);

    // Ask: the dialog opens and the window stays.
    request_close(&scene);
    assert!(choice.get_open(), "a close with Ask didn't open the dialog");
    assert!(w.is_visible(), "the window hid while asking");
    assert!(choice.get_remember(), "Remember my choice starts checked");
    assert_eq!(choice.get_focused(), 0, "Close featherplay is the default");

    // Escape cancels.
    key(w, Key::Escape);
    assert!(!choice.get_open(), "Escape didn't close the dialog");
    assert!(w.is_visible() && !scene.ui.global::<Frame>().get_hidden());
    assert_eq!(settings::close_action(), CloseAction::Ask);

    // A click on the scrim cancels too.
    request_close(&scene);
    assert!(choice.get_open());
    click(w, 20.0, 20.0);
    assert!(!choice.get_open(), "a click on the scrim didn't cancel");
    assert!(w.is_visible());
    assert_eq!(settings::close_action(), CloseAction::Ask);

    // "Keep playing in the tray" with Remember checked: saved, and the window hides.
    request_close(&scene);
    key(w, Key::Tab);
    assert_eq!(choice.get_focused(), 1);
    key(w, Key::Return);
    assert!(!choice.get_open());
    assert_eq!(settings::close_action(), CloseAction::Tray);
    assert_eq!(scene.ui.global::<SettingsData>().get_close_action(), 2);
    assert!(
        scene.ui.global::<Frame>().get_hidden(),
        "not hidden to the tray"
    );
    assert!(!w.is_visible());

    // Set, a close does it without asking.
    scene.ui.global::<Frame>().set_hidden(false);
    scene.ui.show().expect("show");
    request_close(&scene);
    assert!(
        !choice.get_open(),
        "asked again after the choice was remembered"
    );
    assert!(scene.ui.global::<Frame>().get_hidden());

    // Unchecked: done this once, and the next close asks again.
    ask_again(&scene);
    request_close(&scene);
    key(w, Key::Tab);
    key(w, Key::Tab);
    assert_eq!(choice.get_focused(), 2);
    key(w, " ");
    assert!(
        !choice.get_remember(),
        "Space didn't uncheck Remember my choice"
    );
    key(w, Key::Backtab);
    assert_eq!(choice.get_focused(), 1);
    key(w, Key::Return);
    assert!(scene.ui.global::<Frame>().get_hidden());
    assert_eq!(settings::close_action(), CloseAction::Ask);
    scene.ui.global::<Frame>().set_hidden(false);
    scene.ui.show().expect("show");
    request_close(&scene);
    assert!(choice.get_open(), "unchecked, the next close didn't ask");
    assert!(choice.get_remember(), "Remember my choice is checked again");

    // Enter picks Close featherplay, the default. No controller runs here, so the event
    // loop is asked to end directly (a no-op offscreen).
    key(w, Key::Return);
    assert!(!choice.get_open());
    assert_eq!(settings::close_action(), CloseAction::Quit);
    assert!(!w.is_visible());
}
