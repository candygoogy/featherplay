//! A page's first frames, at 4K 150 %, playing, art on every cover, Up next open: the user's
//! conditions (`support::playing_4k`). The ceilings are the targets set on
//! 2026-11-01: a page's first show in the session at most 16 ms a frame,
//! a page shown again at most 10.
//!
//! The frames hold in optimised builds; CPU time per frame is noisy while other work loads
//! the machine, so each check gets up to three tries (a first show, one).

use crate::support::{Frame1, idle, playing_4k, show, step, three_tries};
use featherplay::snapshot::Scene;
use slint::ComponentHandle;

/// The most a frame of a page shown again may cost, ms of CPU (optimised).
const RESHOW_MAX_MS: f64 = 10.0;
/// Settings' own two: the most a frame of its first show in the session may cost, and of it
/// shown again. It is the longest page (115 rows; 67 when it was held to 16 and 10 ms, which
/// it then met at ~12 and ~8): half of its first frame is painting the rows a 4K view shows,
/// half making and laying them out, the sections below the fold left for the frames after.
/// Measured 2026-11-15: 14 to 15.5 ms the first time, 8.3 to 9.5 shown again.
const SETTINGS_FIRST_MAX_MS: f64 = 18.0;
const SETTINGS_RESHOW_MAX_MS: f64 = 12.0;
/// The window, physical pixels.
const WINDOW: f64 = 3840.0 * 2160.0;

/// From Home to `page`, the frames of its entry (the rise, the cards' fades).
fn enter(scene: &Scene, page: &str) -> Vec<Frame1> {
    scene.open(page).expect("page");
    let frames = (0..100).map(|_| step(scene)).collect();
    show(scene, "home");
    frames
}

/// The worst frame of `frames` against `ceiling`.
fn worst_within(frames: &[Frame1], page: &str, ceiling: f64) -> Result<(), String> {
    let first = frames[0].cpu;
    let worst = frames.iter().map(|f| f.cpu).fold(0.0, f64::max);
    println!("{page} shown again at 4K 150 %: first frame {first:.2} ms, worst {worst:.2} ms");
    if worst <= ceiling {
        Ok(())
    } else {
        Err(format!(
            "a frame of {page} shown again cost {worst:.2} ms, over {ceiling}"
        ))
    }
}

/// Explore's New releases, shown again: its shelves are known and their art is in memory, so
/// every card's cover is there in the first frame, and no frame is long. The change repaints
/// the page and the sidebar's marked entry, not the whole window: Up next and the bar stay as
/// they are. Before (2026-11-01) every page change marked whatever Now playing moves aside
/// (the sidebar, Up next, the bar's clear) dirty, though no page but Now playing moves them,
/// and the new page's first two frames repainted all 4K of it.
#[test]
fn explore_shown_again_has_no_long_frame_and_leaves_up_next_and_the_bar_alone() {
    let scene = playing_4k();
    show(&scene, "explore");
    show(&scene, "home");
    three_tries(|attempt| {
        let frames = enter(&scene, "explore");
        if attempt == 0 {
            for (i, f) in frames[..4].iter().enumerate() {
                let share = f.damaged as f64 / WINDOW * 100.0;
                println!("Explore shown again, frame {i}: {share:.1} % of the window repainted");
                assert!(
                    share < 95.0,
                    "Explore's frame {i} repainted {share:.1} % of the window"
                );
            }
        }
        worst_within(&frames, "explore", RESHOW_MAX_MS)
    });
}

/// Up next sliding in or out on a wide window repaints the panel's strip, not the window:
/// only the frame the page changes width in (the first closing, the last opening) is whole.
/// Before (2026-11-01) the narrow windows' scrim, kept at a wide one, faded from 0 to 0 and
/// marked the whole window dirty in each frame of its fade: ~35 full 4K repaints closing.
#[test]
fn up_next_slides_repaint_only_its_strip() {
    let scene = playing_4k();
    show(&scene, "home");
    let frame = scene.ui.global::<featherplay::ui::Frame>();
    for open in [false, true] {
        frame.set_queue_open(open);
        let frames: Vec<Frame1> = (0..100).map(|_| step(&scene)).collect();
        let whole = frames
            .iter()
            .filter(|f| f.damaged as f64 >= 0.9 * WINDOW)
            .count();
        let what = if open { "opening" } else { "closing" };
        println!("Up next {what}: {whole} frames repainted the whole window");
        assert!(
            whole <= 1,
            "Up next {what} repainted the whole window in {whole} frames"
        );
        idle(&scene, 100);
    }
}

/// Settings: its first show in the session (the fonts and glyphs only it uses, then the page),
/// then shown again. Nothing but the ceilings, which a debug build doesn't hold.
#[test]
#[cfg_attr(debug_assertions, ignore = "a release-build ceiling")]
fn settings_shows_without_a_long_frame() {
    let scene = playing_4k();
    let frames = enter(&scene, "settings");
    let worst = frames.iter().map(|f| f.cpu).fold(0.0, f64::max);
    println!(
        "settings' first show at 4K 150 %: first frame {:.2} ms, worst {worst:.2} ms",
        frames[0].cpu
    );
    assert!(
        cfg!(debug_assertions) || worst <= SETTINGS_FIRST_MAX_MS,
        "a frame of Settings' first show cost {worst:.2} ms, over {SETTINGS_FIRST_MAX_MS}"
    );
    three_tries(|_| {
        worst_within(
            &enter(&scene, "settings"),
            "settings",
            SETTINGS_RESHOW_MAX_MS,
        )
    });
}
