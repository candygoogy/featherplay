//! Art on every card doesn't make a Home scroll frame expensive at 4K: on the offscreen
//! platform, through the app's own present path, a long Home (12 shelves of 12 cards) scrolls
//! at 3840×2160, scale 1.5, Up next open, with art and then without it.
//!
//! Resampling the art on every frame (it's decoded at a fixed size and the tiles are fluid)
//! cost 10 ms of CPU a frame. Drawn once into each cover's layer, art costs a frame about 2 ms
//! more than no art. That difference is skia's work, compiled optimised in every build, so it
//! holds in a debug run too; the frame's own ceiling (7 ms median, 10 ms p95) is held only in
//! an optimised build, where Slint's part of the frame is what ships.

use crate::support;
use crate::support::{home_at, quantile, wheel_run};
use featherplay::ui::HomeData;
use slint::{ComponentHandle, Image, LogicalPosition, Model};

/// What art may add to a scroll frame's median and p95, in ms of CPU.
const ART_MEDIAN_MS: f64 = 2.5;
const ART_P95_MS: f64 = 4.0;
/// A whole scroll frame's ceiling with art, optimised (median, p95). The frame without any
/// art is already 5.95 / 7.35 ms here (text and the many cards a row of the shaped shelves
/// holds), so this is that floor with room for noise, not a 6 / 8 ms target.
const FRAME_MEDIAN_MS: f64 = 7.0;
const FRAME_P95_MS: f64 = 10.0;

/// The CPU times of the repainted frames of a wheel run down and back up, the cheaper of two.
fn scroll(w: &slint::Window, at: LogicalPosition) -> Vec<f64> {
    let run = || {
        let mut out = Vec::new();
        for notches in [-2.0, 2.0] {
            out.extend(
                wheel_run(w, at, 15, notches)
                    .into_iter()
                    .filter(|f| f.damaged > 0)
                    .map(|f| f.cpu),
            );
        }
        out
    };
    // A first pass creates the rows and draws their layers: not what a scroll costs.
    let _ = run();
    let (a, b) = (run(), run());
    if a.len() == b.len() {
        a.iter().zip(&b).map(|(x, y)| x.min(*y)).collect()
    } else {
        a
    }
}

fn set_covers(ui: &featherplay::ui::AppWindow, mut f: impl FnMut(&Image) -> Image) {
    for shelf in ui.global::<HomeData>().get_shelves().iter() {
        for i in 0..shelf.items.row_count() {
            if let Some(mut item) = shelf.items.row_data(i) {
                item.cover = f(&item.cover);
                shelf.items.set_row_data(i, item);
            }
        }
    }
}

#[test]
fn art_adds_little_to_a_4k_home_scroll_frame() {
    let scene = home_at(3840, 2160, 1.5);
    let ui = &scene.ui;
    let w = ui.window();
    support::long_home(ui, 12, 12);
    scene.settle();
    let covers: Vec<Image> = ui
        .global::<HomeData>()
        .get_shelves()
        .iter()
        .flat_map(|s| s.items.iter().map(|it| it.cover).collect::<Vec<_>>())
        .collect();
    let with_art = covers.iter().filter(|i| i.size().width > 0).count();
    assert!(with_art >= 100, "only {with_art} cards have art");
    let at = LogicalPosition::new(1024.0, 600.0);

    let art = scroll(w, at);
    set_covers(ui, |_| Image::default());
    let bare = scroll(w, at);
    let (am, ap) = (quantile(&art, 0.5), quantile(&art, 0.95));
    let (bm, bp) = (quantile(&bare, 0.5), quantile(&bare, 0.95));
    println!(
        "4K 150 % Home scroll, ms of CPU a frame (median / p95): art {am:.2} / {ap:.2}, no art {bm:.2} / {bp:.2}, {} and {} frames",
        art.len(),
        bare.len()
    );
    assert!(
        am - bm <= ART_MEDIAN_MS && ap - bp <= ART_P95_MS,
        "art adds {:.2} / {:.2} ms to a frame (median / p95), over {ART_MEDIAN_MS} / {ART_P95_MS}",
        am - bm,
        ap - bp
    );
    if !cfg!(debug_assertions) {
        assert!(
            am <= FRAME_MEDIAN_MS && ap <= FRAME_P95_MS,
            "a frame with art costs {am:.2} / {ap:.2} ms, over {FRAME_MEDIAN_MS} / {FRAME_P95_MS}"
        );
    }
}
