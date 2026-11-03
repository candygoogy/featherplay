//! A frame copies each pixel it repainted to the window once. Slint hands over up to three
//! dirty rectangles that may overlap, and softbuffer's Win32 backend BitBlts each one, so an
//! overlap was copied twice: the real-window log (`live-2026-11-01_0612.log`) had frames
//! copying 15.3 Mpx of an 8.3 Mpx window, and opening Up next copied 187 % of it offscreen.
//! `coalesce_damage` (vendored renderer, change 9) merges overlapping rectangles whenever their
//! bounding box costs no more than the two did; `offscreen::Screen` copies through it too.

use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{Frame, Theme};
use i_slint_renderer_skia::software_surface::coalesce_damage;
use slint::ComponentHandle;

fn area(r: &[[u32; 4]]) -> u64 {
    r.iter()
        .map(|[x0, y0, x1, y1]| u64::from(x1 - x0) * u64::from(y1 - y0))
        .sum()
}

#[test]
fn overlapping_rectangles_become_their_bounding_box_when_it_costs_no_more() {
    let mut r = vec![[0, 0, 100, 100], [10, 0, 110, 100]];
    coalesce_damage(&mut r);
    assert_eq!(r, vec![[0, 0, 110, 100]]);
}

#[test]
fn a_rectangle_inside_another_goes() {
    let mut r = vec![[20, 20, 40, 40], [0, 0, 100, 100], [200, 0, 210, 10]];
    coalesce_damage(&mut r);
    r.sort_unstable();
    assert_eq!(r, vec![[0, 0, 100, 100], [200, 0, 210, 10]]);
}

#[test]
fn rectangles_that_dont_touch_stay_apart() {
    let mut r = vec![[0, 0, 10, 10], [50, 50, 60, 60]];
    coalesce_damage(&mut r);
    assert_eq!(r.len(), 2);
}

#[test]
fn an_overlap_whose_bounding_box_costs_more_is_never_copied_beyond_its_pixels() {
    // An L: the box would add the empty corner (100 × 90 px) and cost more than the overlap
    // (10 × 10) saves.
    let mut r = vec![[0, 0, 100, 10], [90, 0, 100, 100]];
    let before = area(&r);
    coalesce_damage(&mut r);
    assert!(area(&r) <= before, "{r:?} copies more than before");
}

/// Up next opening at 4K 150 % (its panel slides over the page, which narrows once at the
/// end): no frame copies more than the window.
#[test]
fn no_frame_copies_more_than_the_window() {
    let scene = Scene::new(1280, 800, false, true).expect("scene");
    scene.ui.global::<Theme>().set_motion(true);
    scene.play(false).expect("player");
    scene.open("home").expect("home");
    scene.ui.show().expect("show");
    scene.settle();
    offscreen::freeze_clock();
    let w = scene.ui.window();
    let window = u64::from(w.size().width) * u64::from(w.size().height);
    let frame = scene.ui.global::<Frame>();
    let mut worst = 0;
    for open in [true, false, true] {
        frame.set_queue_open(open);
        for _ in 0..100 {
            offscreen::advance(std::time::Duration::from_millis(4));
            let p = offscreen::present(w).expect("present");
            worst = worst.max(p.damaged);
        }
    }
    assert!(
        worst <= window,
        "a frame copied {worst} px of a {window} px window"
    );
}

/// Minimised, the renderer has let go of its buffer and skips frames: such a frame copies
/// nothing, so it reports no damage, whatever the frame before it copied.
#[test]
fn a_skipped_frame_copies_nothing() {
    let scene = Scene::new(1280, 800, false, true).expect("scene");
    scene.open("home").expect("home");
    scene.ui.show().expect("show");
    scene.settle();
    offscreen::freeze_clock();
    let w = scene.ui.window();
    let first = offscreen::present(w).expect("present");
    assert!(first.damaged > 0, "the first frame copied nothing");

    w.set_minimized(true);
    featherplay::memory::hidden(&scene.ctx, true);
    offscreen::advance(std::time::Duration::from_millis(4));
    let skipped = offscreen::present(w).expect("present");
    assert_eq!(
        (skipped.damaged, skipped.bounds),
        (0, None),
        "a skipped frame reported the frame before it"
    );
}
