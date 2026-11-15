//! Signed out (never signed in, or signed out), offscreen: featherplay works as a visitor.
//! The window stays where it is (only a session that ended goes to Sign in), what belongs to
//! the account asks to sign in in place of an error, and a like asks to sign in instead of
//! writing.

use featherplay::app;
use featherplay::controller::{AccountEvent, Action, UiEvent};
use featherplay::router::Page;
use featherplay::snapshot::Scene;
use featherplay::ui::{Player, PlaylistData, Theme, Toast};
use slint::{ComponentHandle, Model};

use crate::support::label;

/// The top of the topmost shown element saying `name`.
fn top(scene: &Scene, name: &str) -> f32 {
    label::rects(scene, name)
        .iter()
        .map(|r| r.origin.y)
        .min_by(f32::total_cmp)
        .unwrap_or_else(|| panic!("nothing shown says {name:?}"))
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
fn signed_out_the_window_stays_and_the_library_history_and_liked_songs_ask_to_sign_in() {
    let scene = signed_out_on_home();
    assert_eq!(
        scene.ctx.current_page(),
        Page::Home,
        "signed out, the window went to Sign in"
    );

    scene.open("library").expect("library");
    scene.settle();
    assert!(label::shows(&scene, "Sign in to see your library"));
    assert!(!label::shows(&scene, "This didn\u{2019}t load"));
    let library = top(&scene, "Sign in to see your library");

    scene.open("history").expect("history");
    scene.settle();
    let history = top(&scene, "Sign in to see your history");
    assert_eq!(library, history, "the gates sit at different heights");

    // Liked songs keeps its header (its title right of the sidebar's entry), and asks under
    // it rather than listing.
    scene.open("liked").expect("liked songs");
    scene.settle();
    assert!(label::shows(&scene, "Sign in to see your liked songs"));
    assert_eq!(
        scene.ui.global::<PlaylistData>().get_tracks().row_count(),
        0
    );
    let sidebar = scene.ui.global::<Theme>().get_sidebar_w();
    assert!(
        label::rects(&scene, "Liked songs")
            .iter()
            .any(|r| r.origin.x > sidebar),
        "no header"
    );
    // The bar at the bottom has a Shuffle of its own; the header's would be above it.
    let bar = top(&scene, "Previous");
    for name in ["Play", "Shuffle"] {
        assert!(
            label::rects(&scene, name)
                .iter()
                .all(|r| r.origin.y >= bar - 24.0),
            "the header shows {name}"
        );
    }
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
