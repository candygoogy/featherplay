//! The Up next panel in a narrow window, on the offscreen platform (no window is shown).
//!
//! At 960 logical px (1440 px at 150 %, beside the rail), near the narrowest window that keeps
//! the panel beside the page (under 952 px it's a sheet), the page makes room for the panel:
//! it ends at the panel's left edge, and Home's band ends at the page's padding left of it
//! rather than running on under the panel. Where making room beside the full sidebar would
//! leave the page narrower than `Theme.page-min-w` (540 px), the sidebar folds to the rail
//! while the panel is open; nothing floats over a darkened page.

use crate::support::{frames, home_at};
use featherplay::offscreen;
use featherplay::ui::{Frame, Theme};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalSize, PhysicalSize};

const SCALE: f32 = 1.5;

#[test]
fn the_page_ends_at_the_panel_and_the_sidebar_folds_to_make_room() {
    // The user's window: 1440 px at 150 %.
    let scene = home_at(1440, 1200, SCALE);
    let (ui, w) = (&scene.ui, scene.ui.window());
    let theme = ui.global::<Theme>();
    let width = 960.0;
    let panel_x = width - theme.get_panel_w();
    let pad = theme.get_page_pad();

    assert_eq!(
        theme.get_rail_w() + ui.get_page_width(),
        panel_x,
        "the page doesn't end at the panel's left edge"
    );
    assert!(ui.get_page_width() >= theme.get_page_min_w());

    // Nothing of the band in the page's right padding, just left of the panel: each row of
    // pixels there down to the band's foot (the 64 px top bar, 32 px, 224 px) is the ground,
    // as in the left padding. (The rails under it run on to the page's edge on purpose.)
    let shot = offscreen::shown(w).expect("shown");
    let px = |x: f32, y: u32| {
        let p = shot.as_slice()[(y * shot.width() + (x * SCALE) as u32) as usize];
        [p.r, p.g, p.b]
    };
    let near = |a: [u8; 3], b: [u8; 3]| a.iter().zip(b).all(|(a, b)| a.abs_diff(b) <= 6);
    let band_foot = ((64.0 + 32.0 + 224.0) * SCALE) as u32;
    let left = theme.get_rail_w() + 2.0;
    let mut stray = Vec::new();
    for y in 0..band_foot {
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

    // 1100 px: beside the full sidebar, making room would leave the page 520 px, so the
    // sidebar folds to the rail while the panel is open, and the page makes room.
    w.set_size(PhysicalSize::new(1650, 1200));
    w.dispatch_event(WindowEvent::Resized {
        size: LogicalSize::new(1100.0, 800.0),
    });
    let _ = frames(w, 120, |_| {});
    let beside = 1100.0 - theme.get_rail_w() - theme.get_panel_w();
    assert_eq!(
        ui.get_page_width(),
        beside,
        "the page doesn't make room beside the rail at 1100 px"
    );
    assert!(ui.get_page_width() >= theme.get_page_min_w());

    // Closed, the sidebar is back at once, under the panel sliding away; opened again, it
    // folds once the panel is in, with the page's one relayout.
    let full = 1100.0 - theme.get_sidebar_w();
    let frame = ui.global::<Frame>();
    frame.set_queue_open(false);
    let _ = frames(w, 1, |_| {});
    assert_eq!(
        ui.get_page_width(),
        full,
        "the sidebar didn't come back at once"
    );
    let _ = frames(w, 120, |_| {});
    frame.set_queue_open(true);
    let mut widths = Vec::new();
    let _ = frames(w, 120, |_| widths.push(ui.get_page_width()));
    widths.dedup();
    assert_eq!(
        widths,
        [full, beside],
        "the page didn't keep its width through the slide, then make room once"
    );
}
