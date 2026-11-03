//! A page's cards fade their art in without long frames, at 4K 150 %, playing: Explore with
//! its shelves known but no art decoded (after the window hid and let its covers go; on a
//! first visit the shelves' own arrival comes first).
//!
//! Each card's art fades in over its empty tile (`Cover`, `t-fast`). The fade was inside the
//! cover's layer, so the layer was drawn again in every frame of it, the art resampled into
//! it: with a page of cards fading at once, ~15 ms a 4K frame for ~40 frames
//! (measured 2026-11-01). Now each layer is drawn once and blended as it fades. The
//! frames where the shelves and then the art arrive (laying out the page, drawing each new
//! layer once) are one-offs, apart from the fade: the ceiling is on the p95 of the frames that
//! follow the shelves' arrival. It holds in optimised builds.

use crate::animations::{scene, show, step};
use crate::support::quantile;
use featherplay::ui::ExploreData;
use slint::{ComponentHandle, Model};

/// The p95 of the fade's frames, ms of CPU (optimised).
const FADE_P95_MS: f64 = 8.0;

/// From Home to Explore with no art in memory; the CPU time of the frame the shelves show in,
/// and of each of the ~120 frames after it.
fn enter(scene: &featherplay::snapshot::Scene) -> (f64, Vec<f64>) {
    let data = scene.ui.global::<ExploreData>();
    // Nothing decoded in memory, as on a first visit: the art comes from the disk cache.
    let covers = &scene.ctx.covers;
    covers.show(None);
    covers.show(Some(scene.ctx.current_page().kind()));
    scene.open("explore").expect("explore");
    let mut arrived = None;
    let mut after = Vec::new();
    for _ in 0..200 {
        let f = step(scene);
        if std::env::var_os("FADE_FRAMES").is_some() && f.cpu > 3.0 {
            println!("  frame: {:.2} ms, {} px", f.cpu, f.damaged);
        }
        match arrived {
            None if data.get_shelves().row_count() > 0 => arrived = Some(f.cpu),
            Some(_) if after.len() < 120 => after.push(f.cpu),
            _ => {}
        }
    }
    (arrived.expect("Explore's shelves never came"), after)
}

#[test]
fn explores_cards_fade_their_art_in_without_long_frames() {
    let scene = scene();
    // The first visit, so Explore has its shelves.
    enter(&scene);
    show(&scene, "home");
    let mut last = String::new();
    for _ in 0..3 {
        let (first, after) = enter(&scene);
        let (p95, max) = (quantile(&after, 0.95), quantile(&after, 1.0));
        println!(
            "Explore's cards at 4K 150 %: the shelves' frame {first:.2} ms, then p95 {p95:.2}, worst {max:.2} ms"
        );
        show(&scene, "home");
        if cfg!(debug_assertions) || p95 <= FADE_P95_MS {
            return;
        }
        last = format!("Explore's fade frames had a p95 of {p95:.2} ms, over {FADE_P95_MS}");
    }
    panic!("{last}");
}
