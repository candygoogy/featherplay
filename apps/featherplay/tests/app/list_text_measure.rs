//! A song list's texts are measured when they change, not on every frame drawn, on the
//! offscreen platform with fixture data (no window is shown, nothing goes out).
//!
//! Slint 1.19's compiler inlines a property read in one place only into its reader; when that
//! reader is an item's geometry, which the renderer asks for on every frame, a
//! `preferred-width` in it measures its text each frame, and the text's drawn line is broken
//! and its "…" shaped again after each measure. `SongListRow`'s album link did this
//! (`ui/pages/content.slint`): while a playlist scrolled at 1600 px with Up next open, about 33
//! text layouts were made a frame instead of about 1.

use featherplay::ui::{Frame, PlaylistData, TrackRow};
use i_slint_core::window::WindowInner;
use slint::{ComponentHandle, LogicalPosition, Model};

use crate::support::{frames, home_at, long_rows, wheel_run};

/// Text layouts made so far by the window's renderer (the cache's misses).
fn layouts_made(w: &slint::Window) -> u64 {
    let adapter = WindowInner::from_pub(w).window_adapter();
    let cache = adapter
        .renderer()
        .text_layout_cache()
        .expect("a text layout cache");
    cache.layout_miss_count()
}

#[test]
fn a_scrolling_song_list_lays_its_texts_out_once_not_every_frame() {
    // 1600x1000 at 150 %, Up next open: the rows are wide enough for their album column.
    let scene = home_at(2400, 1500, 1.5);
    let w = scene.ui.window();
    scene.open("playlist").expect("playlist");
    scene.settle();
    frames(w, 60, |_| {});
    // The fixture's songs again and again, 92 rows.
    let data = scene.ui.global::<PlaylistData>();
    let rows: Vec<TrackRow> = data.get_tracks().iter().collect();
    data.set_tracks(long_rows(&rows, 92));
    scene.settle();
    frames(w, 60, |_| {});
    scene.ui.global::<Frame>().set_scrolling(true);
    let at = LogicalPosition::new(640.0, 600.0);
    // Down and back once, so every row is made and its texts shaped.
    let _ = wheel_run(w, at, 10, -2.0);
    let _ = wheel_run(w, at, 10, 2.0);

    let before = layouts_made(w);
    let run = wheel_run(w, at, 20, -2.0);
    let drawn = run.iter().filter(|f| f.damaged > 0).count();
    let made = layouts_made(w) - before;
    assert!(drawn > 50, "the list didn't scroll: {drawn} frames drawn");
    // About 1.3 a frame now (rows coming into view), about 33 when every frame measured.
    let per_frame = made as f64 / drawn as f64;
    assert!(
        per_frame < 4.0,
        "{made} text layouts in {drawn} frames drawn ({per_frame:.1} a frame): a text is measured on every frame"
    );
}
