//! Texts measure what they show after the window was minimised or hidden in the tray, on the
//! offscreen platform with fixture data (no window is shown). Hiding it lets the renderer's
//! caches go; on Slint 1.19 dropping the text layouts with them left every measured size on
//! the text it had then, so the bar's next artist elided at the last one's width ("40mP, K…")
//! and its hover area kept the last title's (vendored skia, change 27).

use featherplay::snapshot::Scene;
use slint::ComponentHandle;

use crate::support;
use crate::support::label::{self, LogicalRect};

const ARTIST: &str = "Isabel LaRosa";
const TITLE: &str = "瞬間、シンフォニー。 - A Symphony of Moments";

/// The widths of the elements: the bar's artist and Home's card's.
fn widths(rects: Vec<LogicalRect>) -> Vec<f32> {
    rects.into_iter().map(|r| r.size.width).collect()
}

/// The artist's and the song area's widths with `ARTIST` and `TITLE` playing.
fn measured(scene: &Scene) -> (Vec<f32>, Vec<f32>) {
    scene.play_credited(0, TITLE, &[ARTIST]).expect("player");
    scene.settle();
    support::frame(scene);
    (
        widths(label::rects(scene, ARTIST)),
        widths(label::rects(scene, &format!("Now playing: {TITLE}"))),
    )
}

#[test]
fn texts_measure_a_new_song_after_the_window_was_hidden() {
    let scene = Scene::new(1100, 720, true, true).expect("scene");
    support::resize(scene.ui.window(), 1100, 720);
    let (artist, area) = measured(&scene);
    assert!(
        !artist.is_empty() && artist.iter().all(|&w| w > 60.0),
        "{ARTIST:?} measured {artist:?}"
    );

    // Another, shorter song, then the window hidden and shown again.
    scene
        .play_credited(1, "Short", &["Daft Punk"])
        .expect("player");
    scene.settle();
    support::frame(&scene);
    featherplay::memory::hidden(&scene.ctx, true);
    featherplay::memory::hidden(&scene.ctx, false);
    support::frame(&scene);

    let (artist_after, area_after) = measured(&scene);
    assert_eq!(
        artist_after, artist,
        "{ARTIST:?} after the window was hidden (it was {artist:?})"
    );
    assert_eq!(
        area_after, area,
        "the song area under the long title after the window was hidden"
    );
}
