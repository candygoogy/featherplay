//! Going to the tray on the offscreen platform (no window is shown), with a stand-in for the
//! Win32 window: the close minimises the window and hides it only once Windows' minimise
//! animation has had its time; with animation effects off it hides at once. A show or a quit
//! during the wait keeps the window. Hidden, the window is still minimised, so a show takes
//! the animated restore out of the taskbar button.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use featherplay::app::{self, NativeWindow};
use featherplay::close;
use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{Frame, SettingsData};
use featherplay_store::CloseAction;
use slint::ComponentHandle;
use slint::platform::WindowEvent;

/// The main window's Win32 side, as far as the tray uses it.
struct FakeWindow {
    animations: bool,
    minimised: Cell<bool>,
    /// How many times it was restored and brought to the front.
    forwarded: Cell<u32>,
}

impl NativeWindow for FakeWindow {
    fn minimise(&self) {
        self.minimised.set(true);
    }

    fn is_minimised(&self) -> bool {
        self.minimised.get()
    }

    fn bring_forward(&self) {
        self.minimised.set(false);
        self.forwarded.set(self.forwarded.get() + 1);
    }

    fn animations_enabled(&self) -> bool {
        self.animations
    }
}

/// The window on screen with the close action set to Tray, the clock frozen.
fn scene(animations: bool) -> (Scene, Rc<FakeWindow>) {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("home").expect("home");
    scene.settle();
    let window = Rc::new(FakeWindow {
        animations,
        minimised: Cell::new(false),
        forwarded: Cell::new(0),
    });
    scene.ctx.set_native(window.clone());
    close::install(&scene.ctx, &scene.ui, || true);
    scene
        .ui
        .global::<SettingsData>()
        .invoke_pick_close_action(2);
    assert_eq!(
        featherplay::pages::settings::close_action(),
        CloseAction::Tray
    );
    scene.ui.show().expect("show");
    offscreen::freeze_clock();
    (scene, window)
}

fn request_close(scene: &Scene) {
    scene
        .ui
        .window()
        .dispatch_event(WindowEvent::CloseRequested);
}

fn hidden(scene: &Scene) -> bool {
    !scene.ui.window().is_visible() && scene.ui.global::<Frame>().get_hidden()
}

fn shown(scene: &Scene) -> bool {
    scene.ui.window().is_visible() && !scene.ui.global::<Frame>().get_hidden()
}

const MS: Duration = Duration::from_millis(1);

#[test]
fn with_animations_the_hide_waits_for_the_minimise() {
    let (scene, window) = scene(true);
    request_close(&scene);
    assert!(window.is_minimised(), "the close didn't minimise");
    assert!(shown(&scene), "hidden before the minimise animation ran");

    offscreen::advance(close::MINIMISE_WAIT - MS);
    assert!(shown(&scene), "hidden before the wait was over");

    offscreen::advance(2 * MS);
    assert!(hidden(&scene), "not hidden after the wait");
    assert!(window.is_minimised(), "hidden but no longer minimised");
}

#[test]
fn closed_while_minimised_it_hides_at_once() {
    let (scene, window) = scene(true);
    // Minimised already, then closed from its taskbar button: no animation to wait for.
    window.minimise();
    request_close(&scene);
    assert!(hidden(&scene), "a minimised window waited to hide");
    assert!(window.is_minimised());
}

#[test]
fn without_animations_the_hide_is_immediate() {
    let (scene, window) = scene(false);
    request_close(&scene);
    assert!(hidden(&scene), "animations off, the hide waited");
    assert!(window.is_minimised(), "hidden without being minimised");
}

#[test]
fn a_show_during_the_wait_keeps_the_window() {
    let (scene, window) = scene(true);
    request_close(&scene);
    offscreen::advance(close::MINIMISE_WAIT / 2);
    app::show_window(&scene.ctx);
    assert_eq!(window.forwarded.get(), 1, "the show didn't restore it");

    offscreen::advance(close::MINIMISE_WAIT * 2);
    assert!(shown(&scene), "the hide went ahead after the show");
}

#[test]
fn a_restore_from_the_taskbar_during_the_wait_keeps_the_window() {
    let (scene, window) = scene(true);
    request_close(&scene);
    // The user clicks the taskbar button while it's still there.
    window.minimised.set(false);

    offscreen::advance(close::MINIMISE_WAIT * 2);
    assert!(
        shown(&scene),
        "a window restored from the taskbar was hidden"
    );
}

#[test]
fn a_quit_during_the_wait_doesnt_hide() {
    let (scene, _window) = scene(true);
    request_close(&scene);
    scene.ctx.quit();

    offscreen::advance(close::MINIMISE_WAIT * 2);
    assert!(shown(&scene), "the window hid to the tray while quitting");
}

#[test]
fn back_from_the_tray_the_show_takes_the_animated_restore() {
    let (scene, window) = scene(true);
    request_close(&scene);
    offscreen::advance(close::MINIMISE_WAIT * 2);
    assert!(hidden(&scene));

    app::show_window(&scene.ctx);
    assert!(shown(&scene), "the show didn't show the window");
    // Animated: it waits on the taskbar for its button before restoring.
    assert_eq!(
        window.forwarded.get(),
        0,
        "restored before its button was there"
    );
    offscreen::advance(app::BUTTON_WAIT + MS);
    assert_eq!(window.forwarded.get(), 1, "not restored after the wait");
    assert!(!window.is_minimised());
}

#[test]
fn the_dialogs_keep_playing_waits_for_the_minimise_too() {
    let (scene, window) = scene(true);
    scene
        .ui
        .global::<SettingsData>()
        .invoke_pick_close_action(0);
    request_close(&scene);
    let choice = scene.ui.global::<featherplay::ui::CloseChoice>();
    assert!(choice.get_open(), "Ask didn't open the dialog");
    choice.invoke_keep_playing();
    assert!(window.is_minimised(), "Keep playing didn't minimise");
    assert!(shown(&scene), "hidden before the minimise animation ran");

    offscreen::advance(close::MINIMISE_WAIT * 2);
    assert!(hidden(&scene), "not hidden after the wait");
}
