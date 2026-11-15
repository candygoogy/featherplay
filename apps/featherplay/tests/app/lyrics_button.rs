//! The bar's microphone opens and closes the lyrics view without a long frame, at 4K 150 %,
//! playing, art on every cover, the way the user clicks it: the pointer rests on it, its
//! tooltip shows, the click turns the view on and the tooltip fades out under it.
//!
//! The tooltip's fade is an `opacity` over a few children, so skia drew it through a layer;
//! upstream makes that layer as large as the repainted region, and the toggle repaints ~84 %
//! of the window, so each of the fade's frames cleared and blended a second 4K buffer: the
//! toggle's first ~25 frames cost ~20 ms instead of ~8 (measured 2026-11-01).
//! The ceilings hold in optimised builds; the repainted share in any.

use crate::support::{
    away, click, cpu, idle, lyrics_button, playing_4k, point, quantile, show, step, three_tries,
};
use featherplay::snapshot::Scene;
use featherplay::ui::LyricsData;
use slint::{ComponentHandle, LogicalPosition};

/// The most the first frame of the lyrics toggle may cost, ms of CPU (optimised).
const TOGGLE_FIRST_MAX_MS: f64 = 10.0;
/// The most one frame of the toggle may cost, ms of CPU (optimised).
const TOGGLE_MAX_MS: f64 = 12.0;
/// The window, physical pixels.
const WINDOW: f64 = 3840.0 * 2160.0;

/// The toggle's frames, clicked with the tooltip showing; the worst one's CPU time.
fn clicked_with_tooltip(scene: &Scene, mic: LogicalPosition) -> f64 {
    point(scene, mic);
    // The tooltip shows after 500 ms of rest (125 frames).
    idle(scene, 150);
    let mut worst = 0.0f64;
    for i in 0..100 {
        let c = cpu::cycles();
        if i == 0 {
            click(scene, mic);
        }
        step(scene);
        worst = worst.max(cpu::ms(cpu::cycles() - c));
    }
    worst
}

/// The first frame of the lyrics view opening and of it closing, the first toggle of the
/// session too, at most `TOGGLE_FIRST_MAX_MS`: the cover holds its full size, unscaled, over
/// the toggle's first frames. Then clicked with the tooltip showing, no frame of the toggle
/// over `TOGGLE_MAX_MS`.
#[test]
fn the_microphone_toggles_the_lyrics_without_a_long_frame() {
    let scene = playing_4k();
    show(&scene, "now-playing");
    let mic = lyrics_button(&scene);
    let data = scene.ui.global::<LyricsData>();
    three_tries(|_| {
        let [first_open, first_close] = [(); 2].map(|()| {
            click(&scene, mic);
            let first = step(&scene).cpu;
            idle(&scene, 150);
            first
        });
        println!(
            "the lyrics toggle's first frame at 4K 150 %: open {first_open:.2}, close {first_close:.2} ms"
        );
        let open = clicked_with_tooltip(&scene, mic);
        assert!(data.get_full(), "the click didn't open the lyrics");
        // Off the button and back, so the tooltip shows again for the closing click.
        point(&scene, away(&scene));
        idle(&scene, 20);
        let close = clicked_with_tooltip(&scene, mic);
        assert!(!data.get_full(), "the click didn't close the lyrics");
        point(&scene, away(&scene));
        idle(&scene, 20);
        println!(
            "the microphone's toggle at 4K 150 %: worst frame open {open:.2}, close {close:.2} ms"
        );
        let (first, worst) = (first_open.max(first_close), open.max(close));
        if first > TOGGLE_FIRST_MAX_MS {
            Err(format!(
                "a first frame of the lyrics toggle cost {first:.2} ms, over {TOGGLE_FIRST_MAX_MS}"
            ))
        } else if worst > TOGGLE_MAX_MS {
            Err(format!(
                "a frame of the toggle cost {worst:.2} ms, over {TOGGLE_MAX_MS}"
            ))
        } else {
            Ok(())
        }
    });
}

/// Paused, so no playing mark ticks and merges in. The toggle's first frame repaints the
/// stage and the side panel's tabs (the Lyrics tab steps aside), not the panel's list: ~93 %
/// of the window, the stage's 84 % and a strip of the panel that a small change there, merged
/// into the stage's dirty rectangle (Slint keeps three), adds. Before (2026-11-01) every tab's
/// `visible` was marked dirty by the toggle, and the frame repainted ~99 %.
///
/// While the lines fade (the second half of opening, the first of closing) the stage's ground
/// holds still, all the panel's colour, so only what moves repaints: the cover, the title, the
/// lines. Before (2026-11-01) every frame repainted the whole stage, ~84 % of the window:
/// everything mixed by the ground's tint was marked dirty by the eased value the tint was read
/// from, though the tint itself stayed at 1.
#[test]
fn the_toggle_repaints_only_what_moves() {
    let scene = playing_4k();
    scene.sound(false).expect("player");
    show(&scene, "now-playing");
    let data = scene.ui.global::<LyricsData>();
    // 300 ms is 75 frames at 240 Hz; the half way is frame ~38, kept clear of by a few frames.
    for (open, fading) in [(true, 44..72), (false, 4..32)] {
        let what = if open { "opening" } else { "closing" };
        data.set_full(open);
        let frames: Vec<_> = (0..90).map(|_| step(&scene)).collect();
        let first = frames[0].damaged as f64 / WINDOW * 100.0;
        println!("the lyrics toggle {what}: its first frame repainted {first:.1} % of the window");
        assert!(
            first < 95.0,
            "the lyrics toggle's first frame {what} repainted {first:.1} % of the window"
        );
        // The median: art that a worker thread finishes meanwhile repaints a frame of its own.
        let fade: Vec<f64> = frames[fading]
            .iter()
            .map(|f| f.damaged as f64 / WINDOW * 100.0)
            .collect();
        let typical = quantile(&fade, 0.5);
        println!("{what}: {typical:.1} % of the window repainted a frame while the lines fade");
        assert!(
            typical < 50.0,
            "{what} repainted {typical:.1} % of the window a frame while the lines faded"
        );
        idle(&scene, 60);
    }
}
