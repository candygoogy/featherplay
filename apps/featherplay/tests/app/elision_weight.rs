//! An elided title's "…" is drawn in the title's own weight, on the offscreen platform with
//! fixture data (no window is shown): Slint 1.19 draws the ellipsis without its run's
//! variations, and Manrope's default instance is its thinnest (weight 200), so a playlist
//! title at 800 ended in three faint dots (vendored skia, change 28). Measured against the
//! same title font drawing "…" as text: the three dots cover about as many pixels.

use featherplay::snapshot::Scene;
use featherplay::ui::PlaylistData;
use slint::ComponentHandle;

use crate::support::label;

/// The window's pixels inside `label`'s element whose brightness is over the dark header's,
/// as dots: each connected group of bright pixels, as (left, right, bottom, pixel count).
fn dots(scene: &Scene, text: &str) -> Vec<(u32, u32, u32, u32)> {
    let shot = scene.ui.window().take_snapshot().expect("snapshot");
    let w = shot.width();
    let rect = label::rects(scene, text)
        .into_iter()
        .next()
        .unwrap_or_else(|| panic!("no element says {text:?}"));
    let scale = scene.ui.window().scale_factor();
    let x0 = (rect.origin.x * scale) as u32;
    let y0 = (rect.origin.y * scale) as u32;
    let x1 = ((rect.origin.x + rect.size.width) * scale).ceil() as u32;
    let y1 = ((rect.origin.y + rect.size.height) * scale).ceil() as u32;
    let px = shot.as_slice();
    let bright = |x: u32, y: u32| {
        let p = px[(y * w + x) as usize];
        u32::from(p.r) + u32::from(p.g) + u32::from(p.b) > 3 * 160
    };
    let mut seen = std::collections::HashSet::new();
    let mut found = Vec::new();
    for y in y0..y1 {
        for x in x0..x1 {
            if !bright(x, y) || !seen.insert((x, y)) {
                continue;
            }
            let (mut left, mut right, mut bottom, mut count) = (x, x, y, 0);
            let mut stack = vec![(x, y)];
            while let Some((cx, cy)) = stack.pop() {
                count += 1;
                left = left.min(cx);
                right = right.max(cx);
                bottom = bottom.max(cy);
                for (nx, ny) in [
                    (cx + 1, cy),
                    (cx.wrapping_sub(1), cy),
                    (cx, cy + 1),
                    (cx, cy.wrapping_sub(1)),
                ] {
                    if (x0..x1).contains(&nx)
                        && (y0..y1).contains(&ny)
                        && bright(nx, ny)
                        && seen.insert((nx, ny))
                    {
                        stack.push((nx, ny));
                    }
                }
            }
            found.push((left, right, bottom, count));
        }
    }
    found
}

#[test]
fn an_elided_title_ends_in_dots_of_its_own_weight() {
    let scene = Scene::new(1100, 720, true, false).expect("scene");
    scene.play(false).expect("player");
    scene.open("playlist").expect("playlist");
    scene.settle();
    let data = scene.ui.global::<PlaylistData>();

    // "…" as the title's text: drawn as any glyph of the title is.
    data.set_title("…".into());
    scene.settle();
    let typed: u32 = dots(&scene, "…").iter().map(|d| d.3).sum();
    assert!(typed > 0, "no ellipsis drawn");

    // A title too long for its room: its three rightmost marks are the elision's.
    let long = "Daft punk /Moby / Fatboy Slim / The Chemical Brothers / Underworld / Orbital / Leftfield / Basement Jaxx";
    data.set_title(long.into());
    scene.settle();
    let mut marks = dots(&scene, long);
    marks.sort_by_key(|d| d.1);
    let elided: u32 = marks.iter().rev().take(3).map(|d| d.3).sum();
    let share = f64::from(elided) / f64::from(typed);
    assert!(
        (0.75..1.33).contains(&share),
        "the elision's dots cover {elided} px, the typed \"…\" {typed} ({share:.2})"
    );
}
