//! The Up next panel in a narrow window, on the offscreen platform (no window is shown).
//!
//! At the window's least width (960 logical px: 1440 px at 150 %, beside the rail) the page
//! makes room for the panel: it ends at the panel's left edge, and Home's rows end at the
//! page's padding left of it rather than running on under the panel. Where making room would
//! leave the page narrower than `Theme.page-min-w` (1100 px, beside the full sidebar), the
//! panel floats over the page at its full width instead, and Escape closes it.

use crate::support::{frames, home_at};
use featherplay::offscreen;
use featherplay::ui::{Frame, Theme};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalSize, PhysicalSize};

const SCALE: f32 = 1.5;

#[test]
fn the_page_ends_at_the_panel_and_floats_it_below_the_least_width() {
    // The user's window: 1440 px at 150 %.
    let scene = home_at(1440, 1200, SCALE);
    let (ui, w) = (&scene.ui, scene.ui.window());
    let theme = ui.global::<Theme>();
    let width = 960.0;
    let panel_x = width - theme.get_panel_w();
    let pad = theme.get_page_pad();

    assert!(!ui.get_panel_floats(), "the panel floats at {width} px");
    assert_eq!(
        theme.get_rail_w() + ui.get_page_width(),
        panel_x,
        "the page doesn't end at the panel's left edge"
    );
    assert!(ui.get_page_width() >= theme.get_page_min_w());

    // Nothing of the page's rows in its right padding, just left of the panel: each row
    // of pixels there is the ground or the wash (horizontal bands), as in the left padding.
    let shot = offscreen::shown(w).expect("shown");
    let px = |x: f32, y: u32| {
        let p = shot.as_slice()[(y * shot.width() + (x * SCALE) as u32) as usize];
        [p.r, p.g, p.b]
    };
    let near = |a: [u8; 3], b: [u8; 3]| a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 6);
    let bar_top = ((w.size().height as f32 / SCALE - theme.get_bar_clear()) * SCALE) as u32;
    let left = theme.get_rail_w() + 2.0;
    let mut stray = Vec::new();
    for y in 0..bar_top {
        let ground = px(left, y);
        let mut x = panel_x - pad + 2.0;
        while x < panel_x - 2.0 {
            if !near(px(x, y), ground) {
                stray.push((x, y));
            }
            x += 1.0;
        }
    }
    assert!(
        stray.is_empty(),
        "{} pixels of the page's rows in its right padding, first at {:?}",
        stray.len(),
        stray.first()
    );

    // 1100 px beside the full sidebar: making room would leave the page 520 px.
    w.set_size(PhysicalSize::new(1650, 1200));
    w.dispatch_event(WindowEvent::Resized {
        size: LogicalSize::new(1100.0, 800.0),
    });
    let _ = frames(w, 60, |_| {});
    assert!(ui.get_panel_floats(), "the panel doesn't float at 1100 px");
    assert_eq!(
        ui.get_page_width(),
        1100.0 - theme.get_sidebar_w(),
        "the page doesn't keep its full width under the floating panel"
    );

    w.dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    w.dispatch_event(WindowEvent::KeyReleased {
        text: Key::Escape.into(),
    });
    assert!(
        !ui.global::<Frame>().get_queue_open(),
        "Escape didn't close the floating panel"
    );
}
