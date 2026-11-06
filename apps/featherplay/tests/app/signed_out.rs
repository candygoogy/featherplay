//! Signed out (never signed in, or signed out), offscreen: featherplay works as a visitor.
//! The window stays where it is (only a session that ended goes to Sign in), what belongs to
//! the account asks to sign in in place of an error, and a like asks to sign in instead of
//! writing.

use std::ops::ControlFlow;

use featherplay::app;
use featherplay::controller::{AccountEvent, Action, UiEvent};
use featherplay::router::Page;
use featherplay::snapshot::Scene;
use featherplay::ui::{Player, PlaylistData, Toast};
use i_slint_core::accessibility::AccessibleStringProperty;
use i_slint_core::item_tree::ItemRc;
use i_slint_core::window::WindowInner;
use slint::{ComponentHandle, Model};

/// Whether a visible element says `label`.
fn shown(scene: &Scene, label: &str) -> bool {
    let root = ItemRc::new_root(WindowInner::from_pub(scene.ui.window()).component());
    let found = root.visit_descendants(|item| {
        let says = item
            .accessible_string_property(AccessibleStringProperty::Label)
            .is_some_and(|l| l == label);
        if says && item.is_visible() {
            ControlFlow::Break(())
        } else {
            ControlFlow::Continue(())
        }
    });
    found.is_some()
}

fn signed_out_on_home() -> Scene {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.play(false).expect("play");
    scene.open("home").expect("home");
    scene.settle();
    app::on_event(UiEvent::Account { signed_in: false });
    scene.settle();
    scene
}

#[test]
fn signed_out_the_window_stays_and_the_library_asks_to_sign_in() {
    let scene = signed_out_on_home();
    assert_eq!(
        scene.ctx.current_page(),
        Page::Home,
        "signed out, the window went to Sign in"
    );

    scene.open("library").expect("library");
    scene.settle();
    assert!(shown(&scene, "Sign in to see your library"));
    assert!(!shown(&scene, "This didn\u{2019}t load"));

    // Liked songs keeps its header, and asks under it rather than listing.
    scene.open("liked").expect("liked songs");
    scene.settle();
    assert!(shown(&scene, "Sign in to see your liked songs"));
    assert_eq!(
        scene.ui.global::<PlaylistData>().get_tracks().row_count(),
        0
    );
}

#[test]
fn signed_out_a_like_asks_to_sign_in_and_writes_nothing() {
    let scene = signed_out_on_home();
    scene.take_actions();

    scene.ui.global::<Player>().invoke_toggle_like();

    let toast = scene.ui.global::<Toast>();
    assert!(toast.get_open());
    assert_eq!(toast.get_text().as_str(), "Sign in to like songs");
    assert_eq!(toast.get_action().as_str(), "Sign in");
    assert!(!scene.ui.global::<Player>().get_liked(), "the heart filled");
    assert!(
        !scene
            .take_actions()
            .iter()
            .any(|a| matches!(a, Action::Rate(..))),
        "a rating was sent"
    );
}

#[test]
fn a_session_that_ended_goes_to_sign_in() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("home").expect("home");
    scene.settle();

    app::on_event(UiEvent::AccountNotice(AccountEvent::SessionExpired));
    app::on_event(UiEvent::Account { signed_in: false });
    scene.settle();

    assert_eq!(scene.ctx.current_page(), Page::SignIn);
}
