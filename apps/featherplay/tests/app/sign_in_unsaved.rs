//! A new sign-in that Windows wouldn't store (Credential Manager refused the cookie header):
//! the session works for this run, and a toast says it isn't saved for the next start. On
//! the offscreen platform (no window is shown), with the fixtures' identities and no
//! controller: the test plays its part, as in `account_picker`. That the controller reports
//! a refused save is tested beside it (`controller::account`).

use std::ops::ControlFlow;

use featherplay::app;
use featherplay::controller::{AccountAction, Action, UiEvent};
use featherplay::router::Page;
use featherplay::snapshot::Scene;
use featherplay::ui::{AccountPicker, Toast, ToastTone};
use i_slint_core::accessibility::AccessibleStringProperty;
use i_slint_core::item_tree::ItemRc;
use i_slint_core::items::AccessibleRole;
use i_slint_core::window::WindowInner;
use slint::ComponentHandle;
use slint::platform::{Key, WindowEvent};

/// The width of the toast's pill, which is as wide as its line up to 480 px.
fn pill_width(scene: &Scene) -> f32 {
    let line = scene.ui.global::<Toast>().get_text();
    let root = ItemRc::new_root(WindowInner::from_pub(scene.ui.window()).component());
    let mut width = None;
    root.visit_descendants(|item| {
        let label = item.accessible_string_property(AccessibleStringProperty::Label);
        if item.accessible_role() == AccessibleRole::Text && label.is_some_and(|l| l == line) {
            width = Some(item.geometry().size.width);
            return ControlFlow::Break(());
        }
        ControlFlow::Continue(())
    });
    width.expect("the toast")
}

#[test]
fn a_sign_in_windows_wouldnt_save_says_so_once_it_shows() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.ui.show().expect("show");
    // Signed out on the sign-in page, then a new sign-in as the controller reports one it
    // couldn't store.
    app::on_event(UiEvent::Account { signed_in: false });
    scene.ctx.navigate(Page::SignIn);
    scene.settle();
    app::on_event(UiEvent::AskIdentity);
    app::on_event(UiEvent::SignInNotSaved);
    app::on_event(UiEvent::Account { signed_in: true });
    scene.settle();

    // The fixtures' session has several identities: the sign-in waits under the picker,
    // and the toast with it, or it would come and go behind the picker's scrim.
    let toast = scene.ui.global::<Toast>();
    assert!(scene.ui.global::<AccountPicker>().get_open());
    assert!(!toast.get_open(), "the toast showed under the picker");

    // Escape takes the default identity; the controller acts as it, and the sign-in goes
    // through.
    scene.take_actions();
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    w.dispatch_event(WindowEvent::KeyReleased {
        text: Key::Escape.into(),
    });
    let chosen = scene
        .take_actions()
        .into_iter()
        .find_map(|a| match a {
            Action::Account(AccountAction::SwitchIdentity(id)) => Some(id),
            _ => None,
        })
        .expect("the picker's choice");
    app::on_event(UiEvent::Identity(chosen));
    scene.settle();

    assert!(
        toast.get_open(),
        "no toast for the sign-in that wasn't saved"
    );
    assert_eq!(toast.get_tone(), ToastTone::Error);
    // On Linux, the keyring refused it, or there's none in this session.
    let said = if cfg!(windows) {
        "Signed in, but Windows couldn't save it for next time."
    } else if featherplay_platform::supports(featherplay_platform::Feature::SecretStorage) {
        "Signed in, but the keyring didn't save it for next time."
    } else {
        "Signed in for now: no keyring is running to save it."
    };
    assert_eq!(toast.get_text().as_str(), said);
    assert_eq!(toast.get_action().as_str(), "", "nothing to click");
    let width = pill_width(&scene);
    assert!(width < 480.0, "the line is cut short in a {width} px pill");
    // The session works for this run.
    assert!(scene.ctx.signed_in());
    assert_eq!(scene.ctx.current_page(), Page::Home);

    // Once per sign-in: the session's next news (a reconnect) doesn't say it again.
    let shown = toast.get_shown();
    app::on_event(UiEvent::Account { signed_in: true });
    assert_eq!(toast.get_shown(), shown, "said again without a new sign-in");
}
