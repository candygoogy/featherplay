//! The first sign-in's account picker, on the offscreen platform (no window is shown), with
//! the fixtures' placeholder identities and no controller: the test plays its part
//! (`UiEvent::AskIdentity`, then `Account`, then `Identity` once a switch was asked for).
//! With several identities the sign-in is held under the picker until one is chosen, and the
//! choice asks the controller to act as it; with one, or at a start with a stored session,
//! nothing is asked.

use featherplay::app;
use featherplay::controller::{AccountAction, Action, UiEvent};
use featherplay::router::Page;
use featherplay::snapshot::Scene;
use featherplay::ui::AccountPicker;
use featherplay_ytm::Identity;
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, Model, SharedString};

fn key(w: &slint::Window, k: impl Into<SharedString>) {
    let text = k.into();
    w.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    w.dispatch_event(WindowEvent::KeyReleased { text });
}

/// Signed out on the sign-in page, then a new sign-in as the controller reports it.
fn sign_in(scene: &Scene) {
    app::on_event(UiEvent::Account { signed_in: false });
    scene.settle();
    assert_eq!(scene.ctx.current_page(), Page::SignIn);
    app::on_event(UiEvent::AskIdentity);
    app::on_event(UiEvent::Account { signed_in: true });
    scene.settle();
}

fn switches(actions: Vec<Action>) -> Vec<String> {
    actions
        .into_iter()
        .filter_map(|a| match a {
            Action::Account(AccountAction::SwitchIdentity(id)) => Some(id.key()),
            _ => None,
        })
        .collect()
}

#[test]
fn a_sign_in_with_several_identities_asks_which_before_loading() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.ui.show().expect("show");
    scene.open("home").expect("home");
    scene.settle();
    sign_in(&scene);

    let picker = scene.ui.global::<AccountPicker>();
    assert!(picker.get_open(), "several identities, and no picker");
    assert_eq!(
        scene.ctx.current_page(),
        Page::SignIn,
        "the sign-in went through before an identity was chosen"
    );
    let rows = picker.get_accounts();
    assert!(rows.row_count() > 1);
    let default = picker.get_default_row() as usize;
    assert!(rows.row_data(default).expect("row").active);
    assert_eq!(picker.get_focused() as usize, default);

    // Down, then Enter: the next row's identity is asked for, and the picker closes.
    let w = scene.ui.window();
    key(w, Key::DownArrow);
    let next = (default + 1).min(rows.row_count() - 1);
    assert_eq!(picker.get_focused() as usize, next);
    scene.take_actions();
    key(w, Key::Return);
    let key_wanted = rows.row_data(next).expect("row").key.to_string();
    assert_eq!(
        switches(scene.take_actions()),
        std::slice::from_ref(&key_wanted)
    );
    assert!(
        !picker.get_open(),
        "the picker stayed open after the choice"
    );
    assert_eq!(scene.ctx.current_page(), Page::SignIn);

    // The controller acts as it: the held sign-in goes through, to Home.
    let identity = Identity::from_key(&key_wanted).expect("identity");
    app::on_event(UiEvent::Identity(identity));
    scene.settle();
    assert_eq!(scene.ctx.current_page(), Page::Home);
    assert!(!picker.get_open());
}

#[test]
fn escape_takes_the_default_identity() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.ui.show().expect("show");
    sign_in(&scene);
    let picker = scene.ui.global::<AccountPicker>();
    assert!(picker.get_open());
    let default = picker.get_default_row() as usize;
    let wanted = picker
        .get_accounts()
        .row_data(default)
        .expect("row")
        .key
        .to_string();
    scene.take_actions();
    key(scene.ui.window(), Key::Escape);
    assert_eq!(switches(scene.take_actions()), [wanted]);
    assert!(!picker.get_open());
}

#[test]
fn one_identity_or_a_stored_session_asks_nothing() {
    // The demo set's session has one identity.
    // SAFETY: set before anything reads it, on the test's only thread so far (nextest runs
    // each test in its own process).
    unsafe { std::env::set_var("FEATHERPLAY_FIXTURES", "demo") };
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.ui.show().expect("show");
    let picker = scene.ui.global::<AccountPicker>();
    // A start with a stored session (`Scene::new` reports it): no question.
    scene.open("home").expect("home");
    scene.settle();
    assert!(!picker.get_open(), "a stored session asked for an identity");

    // A new sign-in with one identity goes straight through, asking nothing.
    sign_in(&scene);
    assert!(!picker.get_open(), "one identity, and a picker");
    assert_eq!(scene.ctx.current_page(), Page::Home);
    assert!(switches(scene.take_actions()).is_empty());
}
