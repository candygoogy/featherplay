//! The sign-in page, offscreen (no window is shown): the ways in are rows of one list.
//! Its browser row names the browser it opens; with more than one found, "Change" lists them
//! and a pick is offered from then on. With a browser's connection turned off on this PC, it
//! says so calmly and "Why?" opens the policy detail; the details ease open and shut;
//! "Continue without signing in" goes Home. A sign-in window that never gets its handle (the
//! offscreen platform makes none) ends the sign-in in plain words, with Try again. No row
//! that would open a browser is pressed here, and no sign-in window gets far enough to load a
//! page.
//!
//! The page has no search bar. Its block, the error row in it, a toast and the account
//! picker all centre on the room the page has: right of the sidebar, left of Up next while
//! it's open, above the bar.

use std::cell::Cell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use featherplay::controller::{AccountAction, Action, Change};
use featherplay::offscreen;
use featherplay::router::Page;
use featherplay::snapshot::Scene;
use featherplay::ui::{Frame, SessionStore, SignInData, Theme, TopBarData};
use i_slint_core::accessibility::{AccessibilityAction, AccessibleStringProperty};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, SharedString};

use crate::support;
use crate::support::label::{self, ItemRc};

/// The first visible element whose label passes `test`.
fn find(scene: &Scene, test: impl Fn(&str) -> bool) -> Option<ItemRc> {
    label::shown_where(scene, |_, l| test(l)).into_iter().next()
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

/// The page as on a PC with these browsers installed (by id; `true`: a policy turns the
/// connection to it off), its browser row offering `offered`: the same page on every PC,
/// whatever browsers and policies it has.
fn sign_in_page_with(installed: &[(&str, bool)], offered: &str) -> Scene {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("sign-in").expect("sign in");
    scene.settle();
    featherplay::pages::sign_in::offer_for_snapshot(&scene.ctx, installed, offered);
    scene.settle();
    scene
}

/// The page on a PC with Microsoft Edge alone.
fn sign_in_page(edge_blocked: bool) -> Scene {
    sign_in_page_with(&[("edge", edge_blocked)], "edge")
}

#[test]
fn the_ways_in_are_rows_of_one_list() {
    let scene = sign_in_page(false);
    assert!(label::shows(&scene, "Sign in to featherplay"));
    for row in ["Use Microsoft Edge", "Paste from any browser"] {
        assert!(label::shows(&scene, row), "no {row:?} row");
        assert!(enabled(&scene, row), "{row:?} is off");
    }
    // The sign-in window is Windows' alone.
    let window = featherplay_platform::supports(featherplay_platform::Feature::WebViewSignIn);
    assert_eq!(label::shows(&scene, "Sign in with Google"), window);
    assert_ne!(
        line(&scene, "Use Microsoft Edge"),
        "Not available on this PC"
    );
    assert!(!label::shows(&scene, "Why?"));
    assert!(!label::shows(&scene, "Change"), "one browser is no choice");
}

const BROWSERS: [&str; 3] = ["Microsoft Edge", "Google Chrome", "Brave"];

/// Whether the list's entry for `browser` has the check.
fn checked(scene: &Scene, browser: &str) -> bool {
    find(scene, |l| l == browser)
        .unwrap_or_else(|| panic!("{browser:?} isn't listed"))
        .accessible_string_property(AccessibleStringProperty::Checked)
        .is_some_and(|c| c == "true")
}

#[test]
fn with_several_browsers_the_row_names_the_one_offered_and_change_lists_them() {
    let found = [("edge", false), ("chrome", false), ("brave", false)];
    let scene = sign_in_page_with(&found, "chrome");
    assert!(enabled(&scene, "Use Google Chrome"));
    assert!(!label::shows(&scene, "Use Microsoft Edge"));
    for browser in BROWSERS {
        assert!(
            !label::shows(&scene, browser),
            "{browser} shows before Change"
        );
    }

    label::press(&scene, "Change");
    for browser in BROWSERS {
        assert!(label::shows(&scene, browser), "{browser} isn't listed");
    }
    let checks: Vec<bool> = BROWSERS.iter().map(|b| checked(&scene, b)).collect();
    assert_eq!(checks, [false, true, false]);

    // Another way in shuts the list, so it doesn't unfold again when that way ends.
    if featherplay_platform::supports(featherplay_platform::Feature::WebViewSignIn) {
        label::press(&scene, "Sign in with Google");
        assert!(!scene.ui.global::<SignInData>().get_choosing());
    }
}

#[test]
fn picking_a_browser_from_the_list_offers_it_and_remembers_it() {
    let found = [("edge", false), ("chrome", false), ("brave", false)];
    let scene = sign_in_page_with(&found, "chrome");
    label::press(&scene, "Change");
    scene.take_actions();

    label::press(&scene, "Brave");
    assert!(enabled(&scene, "Use Brave"));
    assert!(
        !label::shows(&scene, "Google Chrome"),
        "the list stayed open"
    );
    let remembered = scene.take_actions().into_iter().any(|a| {
        matches!(a, Action::Account(AccountAction::Set(Change::SignInBrowser(id))) if id == "brave")
    });
    assert!(remembered, "the choice wasn't saved");
    // Opened again, the check has moved.
    label::press(&scene, "Change");
    assert!(checked(&scene, "Brave") && !checked(&scene, "Google Chrome"));
}

#[test]
fn a_blocked_browser_in_the_list_says_so_with_why_and_cant_be_picked() {
    let found = [("edge", true), ("chrome", false), ("brave", false)];
    let scene = sign_in_page_with(&found, "chrome");
    // The row offers one that works, so it isn't the one saying "Why?".
    assert!(enabled(&scene, "Use Google Chrome"));
    assert!(!label::shows(&scene, "Why?"));

    label::press(&scene, "Change");
    assert!(!enabled(&scene, "Microsoft Edge"));
    assert_eq!(line(&scene, "Microsoft Edge"), "Not available on this PC");
    let policy = |l: &str| l.starts_with("Edge on this PC");
    assert!(
        find(&scene, policy).is_none(),
        "the policy shows before Why?"
    );
    label::press(&scene, "Why?");
    assert!(find(&scene, policy).is_some(), "Why? didn't open the note");

    label::press(&scene, "Microsoft Edge");
    assert!(
        label::shows(&scene, "Use Google Chrome"),
        "a blocked one was taken"
    );
}

#[test]
fn a_blocked_browser_says_so_in_its_row_and_why_opens_the_detail() {
    let scene = sign_in_page(false);
    let data = scene.ui.global::<SignInData>();
    // (case, the browsers found, the one offered, its row, the detail's policy)
    type Case = (
        &'static str,
        &'static [(&'static str, bool)],
        &'static str,
        &'static str,
        fn(&str) -> bool,
    );
    let cases: [Case; 2] = [
        (
            "Edge alone, blocked",
            &[("edge", true)],
            "edge",
            "Use Microsoft Edge",
            |l| l.contains("RemoteDebuggingAllowed"),
        ),
        (
            "every browser blocked, as with one",
            &[("edge", true), ("chrome", true)],
            "chrome",
            "Use Google Chrome",
            |l| l.starts_with("Chrome on this PC"),
        ),
    ];
    for (case, installed, offered, row, policy) in cases {
        data.set_why_open(false);
        featherplay::pages::sign_in::offer_for_snapshot(&scene.ctx, installed, offered);
        scene.settle();
        assert!(!enabled(&scene, row), "{case}: {row:?} is on");
        assert_eq!(line(&scene, row), "Not available on this PC", "{case}");
        assert!(
            !label::shows(&scene, "Change"),
            "{case}: nothing to choose from"
        );
        assert!(
            find(&scene, policy).is_none(),
            "{case}: the policy shows before Why?"
        );
        label::press(&scene, "Why?");
        assert!(
            find(&scene, policy).is_some(),
            "{case}: Why? didn't open the detail"
        );
    }
}

/// The page as Linux shows it, on any OS: no sign-in window, so its row is gone and the
/// browser row leads; and with no keyring running, the card says the session lasts for this
/// run only.
#[test]
fn without_a_sign_in_window_the_browser_row_leads() {
    let scene = sign_in_page(false);
    let data = scene.ui.global::<SignInData>();
    data.set_window_way(false);
    data.set_session_store(SessionStore::None);
    scene.settle();
    assert!(!label::shows(&scene, "Sign in with Google"));
    assert!(enabled(&scene, "Use Microsoft Edge"));
    assert!(find(&scene, |l| l.contains("lasts until featherplay closes")).is_some());
}

/// A left click with the pointer in the middle of what says `label`.
fn click(scene: &Scene, name: &str) {
    label::click(scene, name);
    scene.settle();
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
    featherplay::pages::sign_in::offer_for_snapshot(&scene.ctx, &[("edge", true)], "edge");
    scene.settle();
    offscreen::freeze_clock();
    offscreen::advance(Duration::from_secs(1));
    // The detail's first line (the last is clipped first as it shuts).
    let policy = |l: &str| l.starts_with("Edge on this PC");
    let shut = top(&scene, "Paste from any browser");

    label::press(&scene, "Why?");
    offscreen::advance(Duration::from_millis(40));
    let opening = top(&scene, "Paste from any browser");
    assert!(find(&scene, policy).is_some(), "the detail isn't there");
    offscreen::advance(Duration::from_secs(1));
    let open = top(&scene, "Paste from any browser");
    assert!(
        shut < opening && opening < open,
        "the row under it jumped instead of following: {shut} {opening} {open}"
    );

    label::press(&scene, "Why?");
    offscreen::advance(Duration::from_millis(40));
    let closing = top(&scene, "Paste from any browser");
    assert!(
        shut < closing && closing < open,
        "it snapped shut: {shut} {closing} {open}"
    );
    assert!(
        find(&scene, policy).is_some(),
        "the detail went before it had shut"
    );
    offscreen::advance(Duration::from_secs(1));
    assert_eq!(top(&scene, "Paste from any browser"), shut);
    assert!(find(&scene, policy).is_none(), "the detail stayed");
}

/// A sign-in window that never gets its handle ends the sign-in in plain words, with Try
/// again. The log says how much memory the app holds as the window opens, and again a
/// minute after it closed: what a hand test reads to see that the window gave its memory
/// back.
#[test]
#[cfg_attr(not(windows), ignore = "the sign-in window is Windows only")]
fn a_sign_in_window_with_no_handle_says_so_plainly_with_try_again_and_the_log_follows_it() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("sign-in-memory-log-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    featherplay_controller::diagnostics::init(&dir).expect("log");
    let log = || {
        featherplay_controller::diagnostics::flush();
        std::fs::read_to_string(dir.join("featherplay.log")).unwrap_or_default()
    };
    let scene = sign_in_page(false);
    let data = scene.ui.global::<SignInData>();
    label::press(&scene, "Sign in with Google");
    assert!(data.get_busy(), "the sign-in didn't start");
    assert!(
        log().contains("sign-in window opening: private "),
        "no memory line as the window opened"
    );

    // The offscreen platform never makes the window: the page waits for its handle, then
    // gives up, and its frame closes.
    let end = Instant::now() + Duration::from_secs(5);
    while data.get_busy() && Instant::now() < end {
        slint::platform::update_timers_and_animations();
        std::thread::sleep(Duration::from_millis(10));
    }
    let after = "sign-in window closed a minute ago: private ";
    offscreen::freeze_clock();
    scene.settle();
    assert!(!data.get_busy(), "still waiting after 5 s");
    assert!(label::shows(&scene, "The sign-in window didn't open"));
    assert!(label::shows(&scene, "Try again, or use another way below."));
    assert!(
        find(&scene, |l| l.contains("handle")).is_none(),
        "the log's detail is on the page"
    );
    assert!(enabled(&scene, "Sign in with Google"));

    offscreen::advance(Duration::from_secs(59));
    assert!(!log().contains(after), "the memory line came early");
    offscreen::advance(Duration::from_secs(1));
    assert!(
        log().contains(after),
        "no memory line a minute after it closed"
    );

    // Try again takes the same way.
    let tried = Rc::new(Cell::new(false));
    let t = tried.clone();
    data.on_open_window(move || t.set(true));
    label::press(&scene, "Try again");
    assert!(tried.get(), "Try again didn't open the window");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The paste fold-out's "Sign in", not the sidebar's: the one under the field.
fn press_paste_sign_in(scene: &Scene) {
    let field = bounds(scene, "What you copied").3;
    let button = label::all(scene, "Sign in")
        .into_iter()
        .find(|b| label::rect(b).min_y() > field)
        .expect("no Sign in under the paste field");
    button.accessible_action(&AccessibilityAction::Default);
    scene.settle();
}

#[test]
fn a_click_on_the_paste_row_opens_the_steps_and_the_field_and_a_paste_that_cant_be_used_says_why() {
    let scene = sign_in_page(false);
    let data = scene.ui.global::<SignInData>();
    assert_eq!(
        line(&scene, "Paste from any browser"),
        "With its developer tools (F12)"
    );
    assert!(!label::shows(&scene, "What you copied"));
    click(&scene, "Paste from any browser");
    assert!(label::shows(&scene, "What you copied"), "no paste field");
    let steps = find(&scene, |l| l.starts_with("1. Open music.youtube.com"))
        .expect("no steps")
        .accessible_string_property(AccessibleStringProperty::Label)
        .unwrap_or_default();
    assert!(
        steps.contains("\n2. Press F12, open the Network tab, type browse in its filter")
            && steps.contains("\n3. Right-click the browse request")
    );
    assert!(label::shows(
        &scene,
        "Paste it here. A cookie header or cookies.txt works too."
    ));

    // An empty field keeps the button off; the check behind it still says why.
    data.invoke_submit("".into());
    scene.settle();
    assert!(label::shows(&scene, "Paste what you copied first."));
    let why = |l: &str| l.contains("Copy as cURL");
    data.set_cookies("hello world".into());
    data.set_paste_error("".into());
    scene.settle();
    press_paste_sign_in(&scene);
    assert!(find(&scene, why).is_some(), "no reason under the field");
    assert!(
        !label::shows(&scene, "Try again"),
        "a paste is fixed in the field"
    );

    // The request's address alone: the reason wraps under the field, in its column, down
    // to the narrowest window.
    let url = "https://music.youtube.com/youtubei/v1/browse?ctoken=4qmFsgKxAhIMRkVtdXNpY19ob21l";
    data.set_cookies(url.into());
    data.set_paste_error("".into());
    scene.settle();
    press_paste_sign_in(&scene);
    let reason =
        "That's only the request's address. Right-click the request and pick Copy as cURL.";
    for width in [1100, 480] {
        resize(&scene, width, 800);
        // The field's text sits 12 px inside its edge.
        let (left, _, right, bottom) = bounds(&scene, "What you copied");
        let (left, right) = (left - 12.0, right + 12.0);
        let (x0, y0, x1, y1) = bounds(&scene, reason);
        assert!(
            (x0 - left).abs() < 1.0,
            "at {width}: the reason starts at {x0}, the field at {left}"
        );
        assert!(
            x1 <= right + 0.5,
            "at {width}: it runs past the field ({x1} > {right})"
        );
        assert!(y0 >= bottom, "at {width}: it's over the field");
        if width == 480 {
            assert!(
                y1 - y0 > 30.0,
                "at 480 it isn't wrapped ({} px tall)",
                y1 - y0
            );
        }
    }
}

/// No browser the sign-in can open: the row says so calmly, dimmed, and Why? names the ones
/// it can. Without a sign-in window (Linux), the paste row is the way in, open already.
#[test]
fn with_no_browser_found_the_row_says_so_and_why_names_the_ones_it_can_open() {
    let scene = sign_in_page_with(&[], "");
    let data = scene.ui.global::<SignInData>();
    assert!(!label::shows(&scene, "Use Microsoft Edge"));
    assert!(!enabled(&scene, "Use your browser"));
    assert_eq!(
        line(&scene, "Use your browser"),
        "None found that featherplay can open"
    );
    let window = data.get_window_way();
    assert_eq!(label::shows(&scene, "What you copied"), !window);

    let install = "Install one of them and this page offers it.";
    assert!(!label::shows(&scene, install));
    label::press(&scene, "Why?");
    assert!(label::shows(&scene, install), "Why? didn't open the note");
    let families = |l: &str| {
        l.starts_with("featherplay can open Edge, Chrome, Brave or Vivaldi for the sign-in")
            && l.ends_with("Sign in with Google works without them.")
    };
    assert_eq!(find(&scene, families).is_some(), window);

    // As Linux shows it, on any OS.
    data.set_why_open(false);
    data.set_window_way(false);
    featherplay::pages::sign_in::offer_for_snapshot(&scene.ctx, &[], "");
    scene.settle();
    assert!(
        label::shows(&scene, "What you copied"),
        "the paste row isn't open"
    );
    label::press(&scene, "Why?");
    let families = |l: &str| {
        l.starts_with("featherplay can open Firefox, Chrome, Chromium")
            && l.ends_with("Pasting from any browser works now.")
    };
    assert!(find(&scene, families).is_some());
}

/// A default browser the sign-in can't open is said on the row's line until one is picked.
#[test]
fn the_row_says_the_default_cant_be_used_until_a_browser_is_picked() {
    let scene = sign_in_page_with(&[("edge", false), ("chrome", false)], "edge");
    let data = scene.ui.global::<SignInData>();
    let usual = "In your own browser, with a temporary profile";
    assert_eq!(line(&scene, "Use Microsoft Edge"), usual);
    data.set_default_unusable(true);
    scene.settle();
    assert_eq!(
        line(&scene, "Use Microsoft Edge"),
        "Your default browser can't be used to sign in"
    );

    label::press(&scene, "Change");
    label::press(&scene, "Google Chrome");
    assert_eq!(line(&scene, "Use Google Chrome"), usual);
}

#[test]
fn continue_without_signing_in_goes_home() {
    let scene = sign_in_page(false);
    label::press(&scene, "Continue without signing in");
    assert_eq!(scene.ctx.current_page(), Page::Home);
}

/// Ctrl and `key`, as the keyboard sends them.
fn with_ctrl(scene: &Scene, key: &str) {
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    tap(scene, key.into());
    w.dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
    scene.settle();
}

/// A pasted request runs to dozens of lines: the field keeps its height, scrolls to the end
/// of the paste, where the text cursor is, and the wheel over it scrolls back to the start.
/// With `FEATHERPLAY_TEST_SHOTS=<dir>`, the page as it shows the paste's end.
#[test]
fn a_long_paste_scrolls_in_the_field_to_its_end_and_the_wheel_scrolls_it_back() {
    let scene = sign_in_page(false);
    click(&scene, "Paste from any browser");
    let (left, top, right, bottom) = bounds(&scene, "What you copied");
    // The paste's own Sign in, under the field: it stays where it is.
    let button = |scene: &Scene| {
        label::all(scene, "Sign in")
            .iter()
            .map(|b| label::rect(b).min_y())
            .filter(|&y| y > bottom)
            .fold(f32::MAX, f32::min)
    };
    let under = button(&scene);
    let curl = std::iter::once("curl 'https://music.youtube.com/youtubei/v1/browse'".to_owned())
        .chain((0..40).map(|i| format!("  -H 'x-header-{i}: {}'", "v".repeat(40))))
        .collect::<Vec<_>>()
        .join(" \\\n");
    label::click(&scene, "What you copied");
    scene.settle();
    offscreen::set_clipboard(&curl);
    with_ctrl(&scene, "v");
    assert_eq!(
        scene.ui.global::<SignInData>().get_cookies().len(),
        curl.len(),
        "the paste didn't land"
    );
    let end = bounds(&scene, "What you copied");
    assert!(
        end.1 < top - 300.0,
        "the paste's end isn't in view: the text's top went from {top} to {}",
        end.1
    );
    assert!(
        end.3 - bottom < 20.0 && end.3 >= bottom - 20.0,
        "the text's end ({}) isn't at the field's foot ({bottom})",
        end.3
    );
    assert!(near(button(&scene), under), "the field grew");
    if let Some(dir) = std::env::var_os("FEATHERPLAY_TEST_SHOTS") {
        let out = std::path::Path::new(&dir).join("sign_in_page-long-paste.png");
        offscreen::snapshot(scene.ui.window(), 1280, 800, &out).expect("shot");
    }

    // The wheel's notches ease (`wheel.rs`): their animation's time, on the frozen clock.
    let over = slint::LogicalPosition::new((left + right) / 2.0, top + 30.0);
    offscreen::wheel(scene.ui.window(), over, 40.0);
    support::run_for(scene.ui.window(), 400);
    assert!(
        near(bounds(&scene, "What you copied").1, top),
        "the wheel didn't scroll it back: its top is at {}, was {top}",
        bounds(&scene, "What you copied").1
    );
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
    support::resize(scene.ui.window(), width, height);
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
    assert!(
        !label::shows(&scene, "Search"),
        "the search bar shows on sign in"
    );
    scene.open("home").expect("home");
    scene.settle();
    assert!(
        label::shows(&scene, "Search"),
        "the search bar is gone from Home"
    );

    // Ctrl+K on sign in: Home, its field taking the keys.
    scene.open("sign-in").expect("sign in");
    scene.settle();
    ctrl_k(&scene);
    tap(&scene, "a".into());
    assert_eq!(scene.ctx.current_page(), Page::Home);
    assert_eq!(scene.ui.global::<TopBarData>().get_query(), "a");
}

#[test]
#[cfg_attr(
    not(windows),
    ignore = "its error row is the sign-in window's, Windows only"
)]
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
