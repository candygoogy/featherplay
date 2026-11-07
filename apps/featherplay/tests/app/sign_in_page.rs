//! The sign-in page, offscreen (no window is shown): the ways in are rows of one list.
//! With Edge's connection turned off on this PC, its row says so calmly and "Why?" opens the
//! policy detail; the details ease open and shut; "Continue without signing in" goes Home. A
//! sign-in window that never gets its handle (the offscreen platform makes none) ends the
//! sign-in in plain words, with Try again. No row that would open Edge is pressed here, and
//! no sign-in window gets far enough to load a page.

use std::cell::Cell;
use std::ops::ControlFlow;
use std::rc::Rc;
use std::time::{Duration, Instant};

use featherplay::offscreen;
use featherplay::router::Page;
use featherplay::snapshot::Scene;
use featherplay::ui::{SignInData, Theme};
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

/// Where the top of what says `label` is in the window.
fn top(scene: &Scene, label: &str) -> f32 {
    let item = find(scene, |l| l == label).unwrap_or_else(|| panic!("nothing says {label:?}"));
    item.map_to_window(item.geometry().origin).y
}

#[test]
fn why_eases_open_and_shut_and_the_rows_below_follow() {
    let scene = Scene::new(1280, 800, false, false).expect("scene");
    scene.ui.global::<Theme>().set_motion(true);
    scene.open("sign-in").expect("sign in");
    scene.settle();
    scene.ui.global::<SignInData>().set_edge_blocked(true);
    scene.settle();
    offscreen::freeze_clock();
    offscreen::advance(Duration::from_secs(1));
    // The detail's first line (the last is clipped first as it shuts).
    let policy = |l: &str| l.starts_with("Edge on this PC");
    let shut = top(&scene, "Paste a cookie header");

    press(&scene, "Why?");
    offscreen::advance(Duration::from_millis(40));
    let opening = top(&scene, "Paste a cookie header");
    assert!(find(&scene, policy).is_some(), "the detail isn't there");
    offscreen::advance(Duration::from_secs(1));
    let open = top(&scene, "Paste a cookie header");
    assert!(
        shut < opening && opening < open,
        "the row under it jumped instead of following: {shut} {opening} {open}"
    );

    press(&scene, "Why?");
    offscreen::advance(Duration::from_millis(40));
    let closing = top(&scene, "Paste a cookie header");
    assert!(
        shut < closing && closing < open,
        "it snapped shut: {shut} {closing} {open}"
    );
    assert!(
        find(&scene, policy).is_some(),
        "the detail went before it had shut"
    );
    offscreen::advance(Duration::from_secs(1));
    assert_eq!(top(&scene, "Paste a cookie header"), shut);
    assert!(find(&scene, policy).is_none(), "the detail stayed");
}

#[test]
fn a_sign_in_window_with_no_handle_says_so_plainly_with_try_again() {
    let scene = sign_in_page(false);
    let data = scene.ui.global::<SignInData>();
    press(&scene, "Sign in with Google");
    assert!(data.get_busy(), "the sign-in didn't start");

    // The offscreen platform never makes the window: the page waits for its handle, then
    // gives up.
    let end = Instant::now() + Duration::from_secs(5);
    while data.get_busy() && Instant::now() < end {
        slint::platform::update_timers_and_animations();
        std::thread::sleep(Duration::from_millis(10));
    }
    scene.settle();
    assert!(!data.get_busy(), "still waiting after 5 s");
    assert!(shown(&scene, "The sign-in window didn't open"));
    assert!(shown(&scene, "Try again, or use another way below."));
    assert!(
        find(&scene, |l| l.contains("handle")).is_none(),
        "the log's detail is on the page"
    );
    assert!(enabled(&scene, "Sign in with Google"));

    // Try again takes the same way.
    let tried = Rc::new(Cell::new(false));
    let t = tried.clone();
    data.on_open_window(move || t.set(true));
    press(&scene, "Try again");
    assert!(tried.get(), "Try again didn't open the window");
}

#[test]
fn a_paste_that_cant_be_used_says_why_under_the_field() {
    let scene = sign_in_page(false);
    click(&scene, "Paste a cookie header");
    let why = |l: &str| l.contains("name=value");
    assert!(find(&scene, why).is_none(), "a reason before any paste");
    scene
        .ui
        .global::<SignInData>()
        .set_cookies("hello world".into());
    scene.settle();
    press(&scene, "Sign in with this cookie");
    assert!(find(&scene, why).is_some(), "no reason under the field");
    assert!(!shown(&scene, "Try again"), "a paste is fixed in the field");
}

#[test]
fn continue_without_signing_in_goes_home() {
    let scene = sign_in_page(false);
    press(&scene, "Continue without signing in");
    assert_eq!(scene.ctx.current_page(), Page::Home);
}
