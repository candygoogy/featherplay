//! The bar's microphone opens and closes the lyrics view without a long frame, at 4K 150 %,
//! playing, art on every cover, the way the user clicks it: the pointer rests on it, its
//! tooltip shows, the click turns the view on and the tooltip fades out under it.
//!
//! The tooltip's fade is an `opacity` over a few children, so skia drew it through a layer;
//! upstream makes that layer as large as the repainted region, and the toggle repaints ~84 %
//! of the window, so each of the fade's frames cleared and blended a second 4K buffer: the
//! toggle's first ~25 frames cost ~20 ms instead of ~8 (measured 2026-11-01).
//! The ceiling holds in optimised builds.

use crate::animations::{click, idle, lyrics_button, point, scene, show, step};
use featherplay::ui::LyricsData;
use slint::ComponentHandle;

/// The most one frame of the toggle may cost, ms of CPU (optimised).
const TOGGLE_MAX_MS: f64 = 12.0;

/// The toggle's frames, clicked with the tooltip showing; the worst one's CPU time.
fn clicked_with_tooltip(scene: &featherplay::snapshot::Scene, mic: slint::LogicalPosition) -> f64 {
    point(scene, mic);
    // The tooltip shows after 500 ms of rest (125 frames).
    idle(scene, 150);
    let mut worst = 0.0f64;
    for i in 0..100 {
        let c = crate::support::cpu::cycles();
        if i == 0 {
            click(scene, mic);
        }
        step(scene);
        worst = worst.max(crate::support::cpu::ms(crate::support::cpu::cycles() - c));
    }
    worst
}

#[test]
fn the_microphone_toggles_the_lyrics_without_a_long_frame() {
    let scene = scene();
    show(&scene, "now-playing");
    let mic = lyrics_button(&scene);
    let data = scene.ui.global::<LyricsData>();
    let mut last = String::new();
    for _ in 0..3 {
        let open = clicked_with_tooltip(&scene, mic);
        assert!(data.get_full(), "the click didn't open the lyrics");
        // Off the button and back, so the tooltip shows again for the closing click.
        point(&scene, crate::animations::away());
        idle(&scene, 20);
        let close = clicked_with_tooltip(&scene, mic);
        assert!(!data.get_full(), "the click didn't close the lyrics");
        point(&scene, crate::animations::away());
        idle(&scene, 20);
        println!(
            "the microphone's toggle at 4K 150 %: worst frame open {open:.2}, close {close:.2} ms"
        );
        let worst = open.max(close);
        if cfg!(debug_assertions) || worst <= TOGGLE_MAX_MS {
            return;
        }
        last = format!("a frame of the toggle cost {worst:.2} ms, over {TOGGLE_MAX_MS}");
    }
    panic!("{last}");
}

/// While the lines fade (the second half of opening, the first of closing) the stage's ground
/// holds still, all the panel's colour, so only what moves repaints: the cover, the title, the
/// lines. Before (2026-11-01) every frame repainted the whole stage, ~84 % of the window:
/// everything mixed by the ground's tint was marked dirty by the eased value the tint was read
/// from, though the tint itself stayed at 1.
#[test]
fn the_toggle_repaints_only_what_moves_while_the_lines_fade() {
    let scene = scene();
    show(&scene, "now-playing");
    let data = scene.ui.global::<LyricsData>();
    let window = 3840.0 * 2160.0;
    // 300 ms is 75 frames at 240 Hz; the half way is frame ~38, kept clear of by a few frames.
    for (open, fading) in [(true, 44..72), (false, 4..32)] {
        data.set_full(open);
        let frames: Vec<_> = (0..90).map(|_| step(&scene)).collect();
        // The median: art that a worker thread finishes meanwhile repaints a frame of its own.
        let fade: Vec<f64> = frames[fading]
            .iter()
            .map(|f| f.damaged as f64 / window * 100.0)
            .collect();
        let typical = crate::support::quantile(&fade, 0.5);
        let what = if open { "opening" } else { "closing" };
        println!("{what}: {typical:.1} % of the window repainted a frame while the lines fade");
        assert!(
            typical < 50.0,
            "{what} repainted {typical:.1} % of the window a frame while the lines faded"
        );
    }
}
