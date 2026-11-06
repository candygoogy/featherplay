//! The sign-in page, offscreen (no window is shown): the ways in are rows of one list.
//! With Edge's connection turned off on this PC, its row says so calmly and "Why?" opens the
//! policy detail; "Continue without signing in" goes Home. No row that would open a sign-in
//! window or Edge is pressed here.

use std::ops::ControlFlow;

use featherplay::router::Page;
use featherplay::snapshot::Scene;
use featherplay::ui::SignInData;
use i_slint_core::accessibility::{AccessibilityAction, AccessibleStringProperty};
use i_slint_core::item_tree::ItemRc;
use i_slint_core::window::WindowInner;
use slint::ComponentHandle;
use slint::platform::{PointerEventButton, WindowEvent};

/// The first visible element whose label passes `test`.
fn find(scene: &Scene, test: impl Fn(&str) -> bool) -> Option<ItemRc> {
    let root = ItemRc::new_root(WindowInner::from_pub(scene.ui.window()).component());
    root.visit_descendants(|item| {
        let says = item
            .accessible_string_property(AccessibleStringProperty::Label)
            .is_some_and(|l| test(&l));
        if says && item.is_visible() {
            ControlFlow::Break(item.clone())
        } else {
            ControlFlow::Continue(())
        }
    })
}

fn shown(scene: &Scene, label: &str) -> bool {
    find(scene, |l| l == label).is_some()
}

fn press(scene: &Scene, label: &str) {
    find(scene, |l| l == label)
        .unwrap_or_else(|| panic!("nothing visible says {label:?}"))
        .accessible_action(&AccessibilityAction::Default);
    scene.settle();
}

fn enabled(scene: &Scene, label: &str) -> bool {
    find(scene, |l| l == label)
        .unwrap_or_else(|| panic!("nothing visible says {label:?}"))
        .accessible_string_property(AccessibleStringProperty::Enabled)
        .is_none_or(|e| e == "true")
}

/// The line under a row's title, as screen readers hear it.
fn line(scene: &Scene, row: &str) -> String {
    find(scene, |l| l == row)
        .unwrap_or_else(|| panic!("no {row:?} row"))
        .accessible_string_property(AccessibleStringProperty::Description)
        .unwrap_or_default()
        .to_string()
}

fn sign_in_page(edge_blocked: bool) -> Scene {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("sign-in").expect("sign in");
    scene.settle();
    // The same page on every PC, whatever its Edge policy.
    scene
        .ui
        .global::<SignInData>()
        .set_edge_blocked(edge_blocked);
    scene.settle();
    scene
}

#[test]
fn the_ways_in_are_rows_of_one_list() {
    let scene = sign_in_page(false);
    assert!(shown(&scene, "Sign in to featherplay"));
    for row in [
        "Sign in with Google",
        "Use Microsoft Edge",
        "Paste a cookie header",
    ] {
        assert!(shown(&scene, row), "no {row:?} row");
        assert!(enabled(&scene, row), "{row:?} is off");
    }
    assert_ne!(
        line(&scene, "Use Microsoft Edge"),
        "Not available on this PC"
    );
    assert!(!shown(&scene, "Why?"));
}

#[test]
fn edge_blocked_says_so_in_its_row_and_why_opens_the_detail() {
    let scene = sign_in_page(true);
    assert!(!enabled(&scene, "Use Microsoft Edge"));
    assert_eq!(
        line(&scene, "Use Microsoft Edge"),
        "Not available on this PC"
    );
    let policy = |l: &str| l.contains("RemoteDebuggingAllowed");
    assert!(
        find(&scene, policy).is_none(),
        "the policy shows before Why?"
    );

    press(&scene, "Why?");
    assert!(
        find(&scene, policy).is_some(),
        "Why? didn't open the detail"
    );
}

/// A left click with the pointer in the middle of what says `label`.
fn click(scene: &Scene, label: &str) {
    let item = find(scene, |l| l == label).unwrap_or_else(|| panic!("nothing says {label:?}"));
    let g = item.geometry();
    let at = item.map_to_window(g.center());
    let position = slint::LogicalPosition::new(at.x, at.y);
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::PointerMoved { position });
    w.dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    w.dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
    scene.settle();
}

#[test]
fn a_click_on_the_cookie_row_opens_the_paste_field() {
    let scene = sign_in_page(false);
    assert!(!shown(&scene, "Cookie header"));
    click(&scene, "Paste a cookie header");
    assert!(shown(&scene, "Cookie header"), "no paste field");
}

#[test]
fn continue_without_signing_in_goes_home() {
    let scene = sign_in_page(false);
    press(&scene, "Continue without signing in");
    assert_eq!(scene.ctx.current_page(), Page::Home);
}
