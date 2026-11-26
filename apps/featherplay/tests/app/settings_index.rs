//! Settings uses a wide window's room, on the offscreen platform with fixture data (no
//! window is shown): the content column runs to 920 px, and at wide windows a section index
//! sits beside it, the two starting at the page's padding like every page's content. The index's
//! first entry is level with the first section's heading. A click on an entry scrolls to its
//! section, and the entry of the section in view is marked, neutral. At narrow windows
//! there's no index and the column starts at the page's padding.

use featherplay::snapshot::Scene;
use i_slint_core::items::AccessibleRole;
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition, Rgba8Pixel, SharedPixelBuffer};

use crate::support::{self, label, pixels};

/// At 1920 wide the page is 1680 px past the 240 px sidebar; the index (184), its gap (40)
/// and the column (920) start at the page's 40 px padding.
const PAGE_LEFT: usize = 240;
const BLOCK: usize = 184 + 40 + 920;
const INDEX_LEFT: usize = PAGE_LEFT + 40;
const INDEX_RIGHT: usize = INDEX_LEFT + 184;
/// The narrow layout's column starts at the page's 40 px padding.
const NARROW_LEFT: usize = PAGE_LEFT + 40;

fn shot(scene: &Scene) -> SharedPixelBuffer<Rgba8Pixel> {
    scene.ui.window().take_snapshot().expect("snapshot")
}

fn px(s: &SharedPixelBuffer<Rgba8Pixel>, x: usize, y: usize) -> Rgba8Pixel {
    s.as_slice()[y * s.width() as usize + x]
}

/// Rows of the index column that hold anything but the ground, between `top` and `bottom`.
fn busy_rows(s: &SharedPixelBuffer<Rgba8Pixel>, top: usize, bottom: usize) -> Vec<usize> {
    let ground = px(s, INDEX_RIGHT + 8, top);
    (top..bottom)
        .filter(|&y| (INDEX_LEFT..INDEX_RIGHT).any(|x| px(s, x, y) != ground))
        .collect()
}

/// Runs of consecutive rows: the index's entries, as (first, last) rows.
fn runs(rows: &[usize]) -> Vec<(usize, usize)> {
    let mut out: Vec<(usize, usize)> = Vec::new();
    for &y in rows {
        match out.last_mut() {
            Some((_, last)) if *last + 1 == y => *last = y,
            _ => out.push((y, y)),
        }
    }
    out
}

/// The window at this size, at 100 %, as the offscreen snapshots size it.
fn resize(scene: &Scene, width: u32, height: u32) {
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 1.0 });
    support::resize(w, width, height);
}

fn click(scene: &Scene, x: f32, y: f32) {
    support::click(scene, LogicalPosition::new(x, y));
    // Away again, so no hover remains on the entry.
    support::point(scene, LogicalPosition::new(1200.0, 500.0));
}

/// One test: the platform can be installed once per process.
#[test]
fn settings_widen_and_index_their_sections_at_wide_windows() {
    let scene = Scene::new(1920, 1080, true, false).expect("scene");
    scene.open("settings").expect("settings");
    scene.theme(true);
    scene.settle();

    // Wide: eight entries, the first one (Playback) marked.
    let s = shot(&scene);
    pixels::save("wide", &s);
    // The rows and the ink measured below are the body font's (`pixels`): the two layouts
    // only saved.
    if !pixels::checked() {
        resize(&scene, 1280, 800);
        scene.settle();
        pixels::save("narrow", &shot(&scene));
        return;
    }
    let entries = runs(&busy_rows(&s, 64, 700));
    assert_eq!(entries.len(), 9, "index entries: {entries:?}");
    let (first, last) = (entries[0], entries[8]);
    let marked = |s: &SharedPixelBuffer<Rgba8Pixel>, (top, last): (usize, usize)| {
        // The entry's fill, left of its text, isn't the ground.
        let y = (top + last) / 2;
        px(s, INDEX_LEFT + 6, y) != px(s, INDEX_RIGHT + 8, y)
    };
    assert!(marked(&s, first), "Playback isn't marked at the top");
    assert!(!marked(&s, last), "Account is marked at the top");
    // Marked with a neutral fill, never the accent (the accent rule of 2026-11-22).
    let fill = px(&s, INDEX_LEFT + 6, (first.0 + first.1) / 2);
    let spread = [fill.r, fill.g, fill.b].iter().max().unwrap()
        - [fill.r, fill.g, fill.b].iter().min().unwrap();
    assert!(spread <= 4, "the marked entry is tinted: {fill:?}");

    // The column beside it is the wide one: a group's panel runs 900 px and more.
    let content_left = INDEX_RIGHT + 40;
    let panel = (200..1000)
        .find(|&y| px(&s, content_left + 600, y) != px(&s, content_left - 8, y))
        .expect("a group panel");
    let panel_colour = px(&s, content_left + 4, panel + 20);
    assert_eq!(
        px(&s, content_left + 900, panel + 20),
        panel_colour,
        "the content column is narrower than 900 px"
    );
    // Lined up with the page's padding, not centred: the column ends 920 px past its start,
    // the rest of the page left empty on the right.
    let ground = px(&s, INDEX_LEFT - 8, panel + 20);
    let col_right = (content_left + 900..1920)
        .find(|&x| px(&s, x + 1, panel + 20) != panel_colour)
        .expect("the panel's right edge");
    assert!(
        col_right.abs_diff(INDEX_LEFT + BLOCK - 1) <= 2,
        "the column ends at {col_right}, not {}",
        INDEX_LEFT + BLOCK - 1
    );
    assert_ne!(ground, panel_colour);

    // The first entry's label is level with the first section's heading ("Playback", the
    // first run of ink between the page title and the first card; its first topic's heading,
    // "What plays", follows it).
    let ink: Vec<usize> = (panel.saturating_sub(90)..panel)
        .filter(|&y| {
            (content_left..content_left + 200).any(|x| px(&s, x, y) != px(&s, content_left - 8, y))
        })
        .collect();
    let heading = runs(&ink)[0];
    let heading_mid = (heading.0 + heading.1) / 2;
    let label: Vec<usize> = (first.0..=first.1)
        .filter(|&y| (INDEX_LEFT + 12..INDEX_LEFT + 100).any(|x| px(&s, x, y) != fill))
        .collect();
    let label_mid = (label[0] + label[label.len() - 1]) / 2;
    assert!(
        heading_mid.abs_diff(label_mid) <= 2,
        "the index's first label sits at {label_mid}, the first heading at {heading_mid}"
    );

    // A click on Account scrolls there and marks it.
    let account = label::shown_where(&scene, |item, l| {
        l == "Account" && item.accessible_role() == AccessibleRole::Button
    });
    let at = label::rect(account.first().expect("the index's Account")).center();
    click(&scene, at.x, at.y);
    let after = shot(&scene);
    let entries = runs(&busy_rows(&after, 64, 700));
    assert_eq!(entries.len(), 9, "after the click: {entries:?}");
    assert!(
        marked(&after, entries[8]),
        "Account isn't marked after its click"
    );
    assert!(!marked(&after, entries[0]), "Playback is still marked");

    // Narrow (1280 x 800 with the sidebar): no index, the content starts at the padding.
    resize(&scene, 1280, 800);
    scene.settle();
    let narrow = shot(&scene);
    // A group's panel starts right at the padding: no index beside it.
    let ground = px(&narrow, NARROW_LEFT - 8, 300);
    let panel_at_padding = (64..700).any(|y| {
        px(&narrow, NARROW_LEFT + 4, y) == panel_colour && px(&narrow, NARROW_LEFT - 8, y) == ground
    });
    assert!(panel_at_padding, "the content doesn't start at the padding");
}
