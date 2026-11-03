//! Settings uses a wide window's room, on the offscreen platform with fixture data (no
//! window is shown): the content column runs to 920 px, and at wide windows a section index
//! sits beside it, the two starting at the page's padding like every page's content. The index's
//! first entry is level with the first section's heading. A click on an entry scrolls to its
//! section, and the entry of the section in view is marked in the accent. At narrow windows
//! there's no index and the column starts at the page's padding.

use featherplay::snapshot::Scene;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Rgba8Pixel, SharedPixelBuffer};

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
    w.set_size(slint::PhysicalSize::new(width, height));
    w.dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(width as f32, height as f32),
    });
}

fn click(scene: &Scene, x: f32, y: f32) {
    let w = scene.ui.window();
    let p = LogicalPosition::new(x, y);
    let button = PointerEventButton::Left;
    w.dispatch_event(WindowEvent::PointerMoved { position: p });
    w.dispatch_event(WindowEvent::PointerPressed {
        position: p,
        button,
    });
    w.dispatch_event(WindowEvent::PointerReleased {
        position: p,
        button,
    });
    // Away again, so no hover remains on the entry.
    w.dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(1200.0, 500.0),
    });
}

/// One test: the platform can be installed once per process.
#[test]
fn settings_widen_and_index_their_sections_at_wide_windows() {
    let scene = Scene::new(1920, 1080, true, false).expect("scene");
    resize(&scene, 1920, 1080);
    scene.open("settings").expect("settings");
    scene.theme(true);
    scene.settle();

    // Wide: six entries, the first one (Playback) marked.
    let s = shot(&scene);
    let entries = runs(&busy_rows(&s, 64, 700));
    assert_eq!(entries.len(), 6, "index entries: {entries:?}");
    let (first, last) = (entries[0], entries[5]);
    let marked = |s: &SharedPixelBuffer<Rgba8Pixel>, (top, last): (usize, usize)| {
        // The entry's fill, left of its text, isn't the ground.
        let y = (top + last) / 2;
        px(s, INDEX_LEFT + 6, y) != px(s, INDEX_RIGHT + 8, y)
    };
    assert!(marked(&s, first), "Playback isn't marked at the top");
    assert!(!marked(&s, last), "Account is marked at the top");
    // Marked in the accent's tint, not a grey.
    let fill = px(&s, INDEX_LEFT + 6, (first.0 + first.1) / 2);
    let spread = [fill.r, fill.g, fill.b].iter().max().unwrap()
        - [fill.r, fill.g, fill.b].iter().min().unwrap();
    assert!(spread >= 12, "the marked entry is grey: {fill:?}");

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
    // ink between the page title and the first panel).
    let heading: Vec<usize> = (panel.saturating_sub(48)..panel)
        .filter(|&y| {
            (content_left..content_left + 200).any(|x| px(&s, x, y) != px(&s, content_left - 8, y))
        })
        .collect();
    let heading_mid = (heading[0] + heading[heading.len() - 1]) / 2;
    let label: Vec<usize> = (first.0..=first.1)
        .filter(|&y| (INDEX_LEFT + 12..INDEX_LEFT + 100).any(|x| px(&s, x, y) != fill))
        .collect();
    let label_mid = (label[0] + label[label.len() - 1]) / 2;
    assert!(
        heading_mid.abs_diff(label_mid) <= 2,
        "the index's first label sits at {label_mid}, the first heading at {heading_mid}"
    );

    // A click on Account scrolls there and marks it.
    let y = (last.0 + last.1) as f32 / 2.0;
    click(&scene, INDEX_LEFT as f32 + 40.0, y);
    let after = shot(&scene);
    let entries = runs(&busy_rows(&after, 64, 700));
    assert_eq!(entries.len(), 6, "after the click: {entries:?}");
    assert!(
        marked(&after, entries[5]),
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
