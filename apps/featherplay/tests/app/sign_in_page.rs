//! The sign-in page, offscreen (no window is shown): the ways in are rows of one list.
//! With Edge's connection turned off on this PC, its row says so calmly and "Why?" opens the
//! policy detail; the details ease open and shut; "Continue without signing in" goes Home. A
//! sign-in window that never gets its handle (the offscreen platform makes none) ends the
//! sign-in in plain words, with Try again. No row that would open Edge is pressed here, and
//! no sign-in window gets far enough to load a page.
//!
//! The page has no search bar. Its block, the error row in it, a toast and the account
//! picker all centre on the room the page has: right of the sidebar, left of Up next while
//! it's open, above the bar.

use std::cell::Cell;
use std::ops::ControlFlow;
use std::rc::Rc;
use std::time::{Duration, Instant};

use featherplay::offscreen;
use featherplay::router::Page;
use featherplay::snapshot::Scene;
use featherplay::ui::{Frame, SignInData, Theme, TopBarData};
use i_slint_core::accessibility::{AccessibilityAction, AccessibleStringProperty};
use i_slint_core::item_tree::ItemRc;
use i_slint_core::window::WindowInner;
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalSize, PhysicalSize, SharedString};

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

/// Where what says `label` is in the window: left, top, right, bottom.
fn bounds(scene: &Scene, label: &str) -> (f32, f32, f32, f32) {
    let item = find(scene, |l| l == label).unwrap_or_else(|| panic!("nothing says {label:?}"));
    let g = item.geometry();
    let o = item.map_to_window(g.origin);
    (o.x, o.y, o.x + g.width(), o.y + g.height())
}

/// The window at `width`×`height` logical pixels (scale 1).
fn resize(scene: &Scene, width: u32, height: u32) {
    let w = scene.ui.window();
    w.set_size(PhysicalSize::new(width, height));
    w.dispatch_event(WindowEvent::Resized {
        size: LogicalSize::new(width as f32, height as f32),
    });
    scene.settle();
}

/// Up next in or out, and the page laid out beside it.
fn panel(scene: &Scene, open: bool) {
    scene.ui.global::<Frame>().set_queue_open(open);
    for _ in 0..4 {
        scene.settle();
    }
}

/// The room the page has: from the sidebar to Up next (or the window's edge), from the
/// window's top to the bar's. Its left, right and bottom.
fn room(scene: &Scene) -> (f32, f32, f32) {
    let ui = &scene.ui;
    let left = ui.get_page_x();
    let bottom = ui.window().size().height as f32 - ui.global::<Theme>().get_bar_clear();
    (left, left + ui.get_page_width(), bottom)
}

/// The block's top and bottom: from the logo (56 px, `Theme.s4` above the title) down to
/// "Continue without signing in".
fn block(scene: &Scene) -> (f32, f32) {
    let logo = 56.0 + scene.ui.global::<Theme>().get_s4();
    let top = bounds(scene, "Sign in to featherplay").1 - logo;
    (top, bounds(scene, "Continue without signing in").3)
}

fn near(a: f32, b: f32) -> bool {
    (a - b).abs() <= 1.0
}

fn centre((l, _, r, _): (f32, f32, f32, f32)) -> f32 {
    (l + r) / 2.0
}

fn tap(scene: &Scene, text: SharedString) {
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    w.dispatch_event(WindowEvent::KeyReleased { text });
    scene.settle();
}

fn ctrl_k(scene: &Scene) {
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    tap(scene, "k".into());
    w.dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
    scene.settle();
}

#[test]
fn the_page_has_no_search_bar_and_ctrl_k_searches_from_home() {
    let scene = sign_in_page(false);
    assert!(!shown(&scene, "Search"), "the search bar shows on sign in");
    scene.open("home").expect("home");
    scene.settle();
    assert!(shown(&scene, "Search"), "the search bar is gone from Home");

    // Ctrl+K on sign in: Home, its field taking the keys.
    scene.open("sign-in").expect("sign in");
    scene.settle();
    ctrl_k(&scene);
    tap(&scene, "a".into());
    assert_eq!(scene.ctx.current_page(), Page::Home);
    assert_eq!(scene.ui.global::<TopBarData>().get_query(), "a");
}

#[test]
fn the_block_centres_in_the_room_beside_up_next_and_the_error_row_stays_with_it() {
    let scene = sign_in_page(false);
    featherplay::pages::sign_in::didnt_open("for a test");
    scene.settle();
    for (width, height) in [(1280, 800), (1568, 1000)] {
        resize(&scene, width, height);
        let mut offsets = Vec::new();
        for open in [false, true] {
            panel(&scene, open);
            let at = format!(
                "{width}x{height}, Up next {}",
                if open { "open" } else { "shut" }
            );
            let (left, right, bottom) = room(&scene);
            let title = bounds(&scene, "Sign in to featherplay");
            assert!(
                near(centre(title), (left + right) / 2.0),
                "{at}: the block's centre {} isn't the room's {}",
                centre(title),
                (left + right) / 2.0
            );
            // In the full height above the bar, a touch above the middle.
            let (top, foot) = block(&scene);
            let above = top / (top + bottom - foot);
            assert!(
                (0.44..=0.46).contains(&above),
                "{at}: {above} of the free height is above the block ({top} above, {} below)",
                bottom - foot
            );
            // The error row: between the title and the card, at the card's left.
            let problem = bounds(&scene, "The sign-in window didn't open");
            let card = bounds(&scene, "Sign in with Google");
            assert!(
                problem.1 > title.3 && problem.3 < card.1,
                "{at}: the error row isn't between the title and the card"
            );
            offsets.push(problem.0 - card.0);
        }
        assert!(
            near(offsets[0], offsets[1]),
            "{width}x{height}: the error row moved apart from the card: {offsets:?}"
        );
    }
}

#[test]
fn a_toast_and_the_account_picker_centre_in_the_same_room() {
    let scene = sign_in_page(false);
    resize(&scene, 1568, 1000);
    for open in [false, true] {
        panel(&scene, open);
        let (left, right, bottom) = room(&scene);
        scene.ctx.toast("Added to queue: Da Funk");
        scene.settle();
        let toast = bounds(&scene, "Added to queue: Da Funk");
        assert!(
            near(centre(toast), (left + right) / 2.0),
            "Up next open {open}: the toast's centre {} isn't the room's {}",
            centre(toast),
            (left + right) / 2.0
        );
        assert!(toast.3 <= bottom, "the toast is over the bar");
    }

    featherplay::account_picker::show_for_snapshot(&scene.ctx);
    scene.settle();
    let (left, right, bottom) = room(&scene);
    let card = bounds(&scene, "Choose an account");
    assert!(
        near(centre(card), (left + right) / 2.0),
        "the picker's centre {} isn't the room's {}",
        centre(card),
        (left + right) / 2.0
    );
    assert!(
        card.1 > 0.0 && card.3 < bottom,
        "the picker isn't above the bar: {card:?}"
    );

    // Ctrl+K doesn't go Home from under it.
    ctrl_k(&scene);
    assert_eq!(scene.ctx.current_page(), Page::SignIn);
}
