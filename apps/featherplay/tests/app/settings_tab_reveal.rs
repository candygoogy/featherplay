//! Tab through Settings in a short window, on the offscreen platform with fixture data (no
//! window is shown): each control on the page that Tab gives the keys to is scrolled into
//! view, below the top bar and above the floating player bar, and Shift+Tab brings them back
//! the same way. A control already in view scrolls nothing.

use featherplay::snapshot::Scene;
use featherplay::ui::Theme;
use i_slint_core::item_tree::{ItemRc, ParentItemTraversalMode};
use i_slint_core::items::Flickable;
use i_slint_core::window::WindowInner;
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, SharedString};

use crate::support::{self, label};

const W: u32 = 1280;
const H: u32 = 600;

fn tap(scene: &Scene, text: SharedString) {
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    w.dispatch_event(WindowEvent::KeyReleased { text });
    scene.settle();
}

/// The element with the keys and how far its page is scrolled, while it's on the page (in
/// its Flickable); `None` on the sidebar, the top bar or the player bar.
fn focused(scene: &Scene) -> Option<(ItemRc, f32)> {
    let item = WindowInner::from_pub(scene.ui.window())
        .focus_item
        .borrow()
        .upgrade()?;
    let mut up = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(p) = up {
        if let Some(flick) = p.downcast::<Flickable>() {
            return Some((item, -flick.as_pin_ref().content_y().get()));
        }
        up = p.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    None
}

/// Tabs (or Shift+Tabs) onto the page and through it until the keys leave it, checking each
/// control with them shows between the top bar and the player bar. Returns how far the page
/// was scrolled at each stop.
fn walk(scene: &Scene, key: Key, top: f32, bottom: f32) -> Vec<f32> {
    let mut stops = Vec::new();
    for _ in 0..400 {
        tap(scene, key.into());
        match focused(scene) {
            Some((item, scrolled)) => {
                let r = label::rect(&item);
                assert!(
                    r.min_y() >= top && r.max_y() <= bottom,
                    "stop {} at y {}..{} with the page scrolled {scrolled}: outside {top}..{bottom}",
                    stops.len(),
                    r.min_y(),
                    r.max_y(),
                );
                stops.push(scrolled);
            }
            None if !stops.is_empty() => return stops,
            None => {}
        }
    }
    panic!("the keys never left the page after {} stops", stops.len());
}

#[test]
fn tab_scrolls_each_setting_into_view() {
    // Still: the page scrolls at once rather than over a spring.
    let scene = Scene::new(W, H, true, false).expect("scene");
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 1.0 });
    support::resize(w, W, H);
    scene.open("settings").expect("settings");
    scene.settle();
    let theme = scene.ui.global::<Theme>();
    let top = theme.get_top_bar_h();
    let bottom = H as f32 - theme.get_bar_clear();

    let down = walk(&scene, Key::Tab, top, bottom);
    assert!(down.len() > 20, "only {} tab stops on Settings", down.len());
    // The first ones show from the start: nothing moves for them.
    assert_eq!(
        down[0], 0.0,
        "the first switch, already in view, scrolled the page"
    );
    // The page went down with the keys, a step at a time, to its end.
    assert!(
        down.windows(2).all(|p| p[1] >= p[0]),
        "Tab scrolled up: {down:?}"
    );
    let end = *down.last().unwrap();
    assert!(end > 2000.0, "Tab took the page only {end} px down");

    // Shift+Tab from past the page's end comes back in at its last control and goes up.
    let up = walk(&scene, Key::Backtab, top, bottom);
    assert_eq!(up.len(), down.len(), "Shift+Tab stopped elsewhere than Tab");
    assert!(
        up.windows(2).all(|p| p[1] <= p[0]),
        "Shift+Tab scrolled down: {up:?}"
    );
    // Back at the first control, only as far up as it needs: near the top.
    let first = *up.last().unwrap();
    assert!(
        first < end / 10.0,
        "Shift+Tab left the page {first} px down"
    );
}

#[test]
fn with_motion_the_page_glides_to_the_control() {
    let scene = Scene::new(W, H, false, false).expect("scene");
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 1.0 });
    support::resize(w, W, H);
    scene.open("settings").expect("settings");
    scene.settle();
    featherplay::offscreen::freeze_clock();
    let theme = scene.ui.global::<Theme>();
    let (top, bottom) = (theme.get_top_bar_h(), H as f32 - theme.get_bar_clear());

    // Tab until a control below the fold has the keys: the page doesn't jump to it, it gets
    // there over the next frames.
    for _ in 0..100 {
        tap(&scene, Key::Tab.into());
        let Some((item, scrolled)) = focused(&scene) else {
            continue;
        };
        if label::rect(&item).max_y() <= bottom {
            continue;
        }
        assert_eq!(scrolled, 0.0, "the page jumped");
        support::run_for(w, 1000);
        let (_, scrolled) = focused(&scene).expect("the keys left the page");
        let r = label::rect(&item);
        assert!(scrolled > 0.0, "the page didn't move");
        assert!(
            r.min_y() >= top && r.max_y() <= bottom,
            "y {}..{} outside {top}..{bottom}",
            r.min_y(),
            r.max_y()
        );
        return;
    }
    panic!("no control below the fold took the keys");
}
