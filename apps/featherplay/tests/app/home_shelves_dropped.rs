//! Shelves scrolled far off Home are dropped, with their cards' art copies and their covers'
//! layers (each 1–2 MB at 4K), so scrolling a long Home down and back doesn't keep growing
//! the memory. On the offscreen platform at 3840×2160, scale 1.5, with art on every card.
//!
//! It needs both Home's `ListView` and the vendored renderer's patch that frees a dropped
//! row's layers (`vendor/i-slint-renderer-skia/VENDORED.md`, change 3): without the patch,
//! every row ever shown kept its layers and this grew past 140 MB.

use crate::support;
use crate::support::{home_at, private_mb, sweep};
use slint::{ComponentHandle, LogicalPosition};

/// Shelves on the long Home (576 cards): keeping what every card drew (a ~1.1 MB layer
/// each at 4K, five or more drawn per shelf) would pass the ceiling several times over.
const SHELVES: usize = 48;
/// What scrolling the whole Home may add to private memory, MB.
const GROWTH_MB: f64 = 40.0;
/// What a second time down and back may add on top, MB.
const AGAIN_MB: f64 = 8.0;

#[test]
fn a_long_home_scrolled_down_and_back_stays_bounded() {
    let scene = home_at(3840, 2160, 1.5);
    let w = scene.ui.window();
    support::long_home(&scene.ui, SHELVES, 12);
    scene.settle();
    let at = LogicalPosition::new(1024.0, 600.0);
    let _ = support::frames(w, 30, |_| {});
    let before = private_mb();
    sweep(w, at);
    let once = private_mb() - before;
    sweep(w, at);
    let twice = private_mb() - before;
    println!(
        "4K 150 %, {SHELVES} shelves: +{once:.1} MB after one sweep, +{twice:.1} MB after two"
    );
    assert!(
        once <= GROWTH_MB,
        "scrolling Home down and back grew private memory {once:.1} MB, over {GROWTH_MB}"
    );
    assert!(
        twice - once <= AGAIN_MB,
        "a second sweep grew it {:.1} MB more, over {AGAIN_MB}",
        twice - once
    );
}
