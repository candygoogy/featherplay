//! A page's first frames, at 4K 150 %, playing, art on every cover, Up next open: the user's
//! conditions (`animations::scene`). The ceilings are the targets set on
//! 2026-11-01: a page's first show in the session at most 16 ms a frame,
//! a page shown again at most 10.
//!
//! The frames hold in optimised builds; CPU time per frame is noisy while other work loads
//! the machine, so each check gets up to three tries (a first show, one).

use crate::animations::{F, scene, show, step};
use featherplay::snapshot::Scene;
use slint::ComponentHandle;

/// The most a frame of a page's first show may cost, ms of CPU (optimised).
const FIRST_MAX_MS: f64 = 16.0;
/// The most a frame of a page shown again may cost, ms of CPU (optimised).
const RESHOW_MAX_MS: f64 = 10.0;
/// The most the first frame of the lyrics toggle may cost, ms of CPU (optimised).
const TOGGLE_FIRST_MAX_MS: f64 = 10.0;

/// From Home to `page`, the frames of its entry (the rise, the cards' fades): their CPU times.
fn enter(scene: &Scene, page: &str) -> Vec<F> {
    scene.open(page).expect("page");
    let frames = (0..100).map(|_| step(scene)).collect();
    show(scene, "home");
    frames
}

/// The worst frame of entering `page`, three tries at most, against `ceiling`.
fn worst_of_entering(scene: &Scene, page: &str, ceiling: f64) {
    let mut last = String::new();
    for _ in 0..3 {
        let frames = enter(scene, page);
        let first = frames[0].cpu;
        let worst = frames.iter().map(|f| f.cpu).fold(0.0, f64::max);
        println!("{page} shown again at 4K 150 %: first frame {first:.2} ms, worst {worst:.2} ms");
        if cfg!(debug_assertions) || worst <= ceiling {
            return;
        }
        last = format!("a frame of {page} shown again cost {worst:.2} ms, over {ceiling}");
    }
    panic!("{last}");
}

/// Explore's New releases, shown again: its shelves are known and their art is in memory, so
/// every card's cover is there in the first frame.
#[test]
fn explore_shown_again_has_no_long_frame() {
    let scene = scene();
    show(&scene, "explore");
    show(&scene, "home");
    worst_of_entering(&scene, "explore", RESHOW_MAX_MS);
}

/// A page change repaints the page and the sidebar's marked entry, not the whole window: Up
/// next and the bar stay as they are. Before (2026-11-01) every page change marked whatever
/// Now playing moves aside (the sidebar, Up next, the bar's clear) dirty, though no page but
/// Now playing moves them, and the new page's first two frames repainted all 4K of it.
#[test]
fn a_page_change_leaves_up_next_and_the_bar_alone() {
    let scene = scene();
    show(&scene, "explore");
    show(&scene, "home");
    let window = 3840.0 * 2160.0;
    scene.open("explore").expect("explore");
    for i in 0..4 {
        let share = step(&scene).damaged as f64 / window * 100.0;
        println!("Explore shown again, frame {i}: {share:.1} % of the window repainted");
        assert!(
            share < 95.0,
            "Explore's frame {i} repainted {share:.1} % of the window"
        );
    }
}

/// Up next sliding in or out on a wide window repaints the panel's strip, not the window:
/// only the frame the page changes width in (the first closing, the last opening) is whole.
/// Before (2026-11-01) the narrow windows' scrim, kept at a wide one, faded from 0 to 0 and
/// marked the whole window dirty in each frame of its fade: ~35 full 4K repaints closing.
#[test]
fn up_next_slides_repaint_only_its_strip() {
    let scene = scene();
    show(&scene, "home");
    let frame = scene.ui.global::<featherplay::ui::Frame>();
    let window = 3840.0 * 2160.0;
    for open in [false, true] {
        frame.set_queue_open(open);
        let frames: Vec<F> = (0..100).map(|_| step(&scene)).collect();
        let whole = frames
            .iter()
            .filter(|f| f.damaged as f64 >= 0.9 * window)
            .count();
        let what = if open { "opening" } else { "closing" };
        println!("Up next {what}: {whole} frames repainted the whole window");
        assert!(
            whole <= 1,
            "Up next {what} repainted the whole window in {whole} frames"
        );
        crate::animations::idle(&scene, 100);
    }
}

/// The lyrics toggle's first frame repaints the stage and the side panel's tabs (the Lyrics
/// tab steps aside), not the panel's list: ~93 % of the window, the stage's 84 % and a strip
/// of the panel that a small change there, merged into the stage's dirty rectangle (Slint
/// keeps three), adds. Before (2026-11-01) every tab's `visible` was marked dirty by the
/// toggle, and the frame repainted ~99 %. Paused, so no playing mark ticks and merges in.
#[test]
fn the_lyrics_toggle_leaves_the_side_panels_list_alone() {
    let scene = scene();
    scene.sound(false).expect("player");
    show(&scene, "now-playing");
    let data = scene.ui.global::<featherplay::ui::LyricsData>();
    let window = 3840.0 * 2160.0;
    for open in [true, false] {
        data.set_full(open);
        let first = step(&scene);
        let share = first.damaged as f64 / window * 100.0;
        let what = if open { "opening" } else { "closing" };
        println!("the lyrics toggle {what}: its first frame repainted {share:.1} % of the window");
        assert!(
            share < 95.0,
            "the lyrics toggle's first frame {what} repainted {share:.1} % of the window"
        );
        crate::animations::idle(&scene, 150);
    }
}

/// The bar's microphone, clicked on Now playing: the first frame of the lyrics view opening
/// and of it closing, the first toggle of the session too, at most `TOGGLE_FIRST_MAX_MS`.
/// The cover holds its full size, unscaled, over the toggle's first frames.
#[test]
fn the_lyrics_toggles_first_frame_is_short() {
    let scene = scene();
    show(&scene, "now-playing");
    let mic = crate::animations::lyrics_button(&scene);
    let mut last = String::new();
    for _ in 0..3 {
        let [open, close] = [(); 2].map(|()| {
            crate::animations::click(&scene, mic);
            let first = step(&scene).cpu;
            crate::animations::idle(&scene, 150);
            first
        });
        let worst = open.max(close);
        println!(
            "the lyrics toggle's first frame at 4K 150 %: open {open:.2}, close {close:.2} ms"
        );
        if cfg!(debug_assertions) || worst <= TOGGLE_FIRST_MAX_MS {
            return;
        }
        last = format!(
            "a first frame of the lyrics toggle cost {worst:.2} ms, over {TOGGLE_FIRST_MAX_MS}"
        );
    }
    panic!("{last}");
}

/// Settings: its first show in the session (the fonts and glyphs only it uses, then the page),
/// then shown again.
#[test]
fn settings_shows_without_a_long_frame() {
    let scene = scene();
    let frames = enter(&scene, "settings");
    let worst = frames.iter().map(|f| f.cpu).fold(0.0, f64::max);
    println!(
        "settings' first show at 4K 150 %: first frame {:.2} ms, worst {worst:.2} ms",
        frames[0].cpu
    );
    assert!(
        cfg!(debug_assertions) || worst <= FIRST_MAX_MS,
        "a frame of Settings' first show cost {worst:.2} ms, over {FIRST_MAX_MS}"
    );
    worst_of_entering(&scene, "settings", RESHOW_MAX_MS);
}
