//! With the colours following the cover, "Not for me" takes the cover's tint as the heart
//! beside it does: the user saw the heart tinted and the old circle-slash stay grey (it was
//! drawn in `text3`, which doesn't follow). Checked in the bar and in Now playing's title row,
//! through the renderer's own frames (`offscreen::install_presenting`); no window is shown.

use std::ops::ControlFlow;
use std::time::Duration;

use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::Theme;
use i_slint_core::accessibility::AccessibleStringProperty;
use i_slint_core::item_tree::ItemRc;
use i_slint_core::lengths::LogicalRect;
use i_slint_core::window::WindowInner;
use slint::{Color, ComponentHandle};

/// A tint no other part of the bar or the stage is drawn in.
const TINT: (u8, u8, u8) = (0x3c, 0xe8, 0x9a);

/// Where every shown element labelled `label` is, in window coordinates.
fn rects(scene: &Scene, label: &str) -> Vec<LogicalRect> {
    let root = ItemRc::new_root(WindowInner::from_pub(scene.ui.window()).component());
    let mut found = Vec::new();
    root.visit_descendants(|item| {
        if item.is_visible()
            && item
                .accessible_string_property(AccessibleStringProperty::Label)
                .is_some_and(|l| l == label)
        {
            let g = item.geometry();
            found.push(LogicalRect::new(item.map_to_window(g.origin), g.size));
        }
        ControlFlow::<()>::Continue(())
    });
    found
}

/// How close the glyph in `r` comes to the tint: the least distance of any of its pixels
/// (0 where the stroke is drawn solid in it).
fn nearest_to_tint(scene: &Scene, r: &LogicalRect) -> u32 {
    let shot = offscreen::shown(scene.ui.window()).expect("shown");
    let (w, h) = (shot.width() as i32, shot.height() as i32);
    let px = shot.as_slice();
    let mut best = u32::MAX;
    for y in r.origin.y as i32..(r.origin.y + r.size.height) as i32 {
        for x in r.origin.x as i32..(r.origin.x + r.size.width) as i32 {
            if x < 0 || y < 0 || x >= w || y >= h {
                continue;
            }
            let p = px[(y * w + x) as usize];
            let d = u32::from(p.r.abs_diff(TINT.0))
                + u32::from(p.g.abs_diff(TINT.1))
                + u32::from(p.b.abs_diff(TINT.2));
            best = best.min(d);
        }
    }
    best
}

fn settle(scene: &Scene) {
    for _ in 0..60 {
        offscreen::advance(Duration::from_millis(16));
        offscreen::present(scene.ui.window()).expect("present");
    }
}

/// The glyphs of the shown buttons labelled `label` (left to right) and how close each
/// comes to the tint.
fn tinted(scene: &Scene, label: &str) -> Vec<(f32, u32)> {
    let mut found: Vec<(f32, u32)> = rects(scene, label)
        .iter()
        .map(|r| (r.origin.x, nearest_to_tint(scene, r)))
        .collect();
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    found
}

/// One test: the platform can be installed once per process.
#[test]
fn not_for_me_takes_the_covers_tint_like_the_heart() {
    let scene = Scene::new(1280, 800, false, true).expect("scene");
    scene.play(false).expect("player");
    scene.open("now-playing").expect("now playing");
    scene.theme(true);
    scene.ui.show().expect("show");
    scene.settle();
    offscreen::freeze_clock();
    // Once the accent has settled on the fixture's cover (none here: the neutral colours),
    // so its fade doesn't paint over the tint.
    settle(&scene);
    let theme = scene.ui.global::<Theme>();
    theme.set_follow_text2(Color::from_rgb_u8(TINT.0, TINT.1, TINT.2));
    theme.set_follow(true);
    settle(&scene);

    // In the bar and in the title row: one of each, unrated.
    let hearts = tinted(&scene, "Like");
    let thumbs = tinted(&scene, "Not for me");
    assert_eq!(hearts.len(), 2, "hearts: {hearts:?}");
    assert_eq!(thumbs.len(), 2, "thumbs-downs: {thumbs:?}");
    for (heart, thumb) in hearts.iter().zip(&thumbs) {
        assert!(
            heart.1 <= 24,
            "the heart at x {} isn't drawn in the cover's tint (off by {})",
            heart.0,
            heart.1
        );
        assert!(
            thumb.1 <= 24,
            "\"Not for me\" at x {} isn't drawn in the cover's tint (off by {}) where the heart is",
            thumb.0,
            thumb.1
        );
    }
}
