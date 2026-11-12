//! Scrolling down a long 4K Home has no bump: no frame is far over a scroll frame's cost, not
//! when a shelf enters and not when the next page starts loading or lands. On the offscreen
//! platform, through the app's own present path, at 3840×2160, scale 1.5, Up next open, art on
//! every card (12 shelves of 12), with answers landing between frames as the event loop runs
//! them.
//!
//! What it caught:
//! - Home's `ListView` took its rows from an `int`. Slint makes a new model of an `int` each
//!   time it changes, and a repeater given a new model drops every row and builds them again,
//!   so the next page's row appearing, and the page landing, each rebuilt every shelf on screen
//!   with its cards and drew their covers' layers again: 41 and 45 ms frames back to back.
//! - A shelf entering drew a layer for each of its covers, the ones on its other pages too
//!   (Slint draws a layer even where it's clipped away): 12 layers, 8.5 ms, in one frame. Now
//!   only the page's covers take one, a card more every 8 ms after the row comes into sight.
//! - A card's hover veil sat in its cover's layer, so each frame of the veil's fade drew the
//!   layer again while the pointer crossed a shelf (1–2 layers, about 1.5 ms, most frames).
//!
//! Every pass down makes the rows again (the `ListView` drops the ones it scrolled past), so
//! each frame keeps its cheapest of three passes, as the smoothness benchmark does; the next
//! page loads in the first.

use crate::support;
use crate::support::{cpu, home_at, quantile};
use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{HomeData, MoreState};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition};

/// The worst frame's ceiling in an optimised build, ms of CPU: a 120 Hz refresh, less room
/// for the present.
const WORST_MS: f64 = 12.0;
/// A frame where the next page starts loading or lands may cost at most this many median
/// frames of the same pass, in any build.
const MORE_TIMES_MEDIAN: f64 = 2.0;
/// Passes down Home.
const PASSES: usize = 3;
/// Wheel events a pass: 2 notches every 32 ms, past the end of the long Home.
const EVENTS: u32 = 60;

struct Timed {
    cpu: f64,
    damaged: u64,
    more: MoreState,
}

/// A slow scroll down, answers run as they arrive.
fn scroll_down(scene: &Scene, at: LogicalPosition) -> Vec<Timed> {
    let w = scene.ui.window();
    let data = scene.ui.global::<HomeData>();
    w.dispatch_event(WindowEvent::PointerMoved { position: at });
    (0..EVENTS * 8 + 60)
        .map(|i| {
            let c = cpu::cycles();
            scene.run_queued();
            if i < EVENTS * 8 && i % 8 == 0 {
                offscreen::wheel(w, at, -2.0);
            }
            offscreen::advance(support::FRAME_240);
            let presented = offscreen::present(w).expect("present");
            Timed {
                cpu: cpu::ms(cpu::cycles() - c),
                damaged: presented.damaged,
                more: data.get_more(),
            }
        })
        .collect()
}

#[test]
#[cfg_attr(not(windows), ignore = "Windows only: it counts the thread's cycles")]
fn scrolling_down_a_4k_home_has_no_bump() {
    let scene = home_at(3840, 2160, 1.5);
    let w = scene.ui.window();
    support::long_home(&scene.ui, 12, 12);
    scene.settle();
    let _ = support::frames(w, 30, |_| {});
    let at = LogicalPosition::new(1024.0, 600.0);

    let first = scroll_down(&scene, at);
    // The frames where the next page's row came or went.
    let more: Vec<f64> = first
        .windows(2)
        .filter(|p| p[0].more != p[1].more)
        .map(|p| p[1].cpu)
        .collect();
    let first_painted: Vec<f64> = first
        .iter()
        .filter(|f| f.damaged > 0)
        .map(|f| f.cpu)
        .collect();
    let first_median = quantile(&first_painted, 0.5);
    let mut cheapest: Vec<f64> = first.iter().map(|f| f.cpu).collect();
    let painted: Vec<bool> = first.iter().map(|f| f.damaged > 0).collect();
    for _ in 1..PASSES {
        // Back to the top, then down again.
        let _ = support::wheel_run(w, at, 20, 16.0);
        scene.settle();
        for (best, f) in cheapest.iter_mut().zip(scroll_down(&scene, at)) {
            *best = best.min(f.cpu);
        }
    }
    let frames: Vec<f64> = cheapest
        .iter()
        .zip(&painted)
        .filter(|(_, p)| **p)
        .map(|(c, _)| *c)
        .collect();
    let median = quantile(&frames, 0.5);
    let worst = frames.iter().copied().fold(0.0, f64::max);
    let mut heaviest: Vec<(usize, f64)> = cheapest
        .iter()
        .copied()
        .enumerate()
        .filter(|(i, _)| painted[*i])
        .collect();
    heaviest.sort_by(|a, b| b.1.total_cmp(&a.1));
    heaviest.truncate(6);
    println!(
        "4K 150 % scroll down Home, ms of CPU a frame (cheapest of {PASSES} passes): median {median:.2}, p95 {:.2}, worst {worst:.2}, {} frames; first pass: median {first_median:.2}, next page {more:.2?}; heaviest (frame, ms) {heaviest:.2?}",
        quantile(&frames, 0.95),
        frames.len()
    );
    assert!(!more.is_empty(), "the scroll never asked for the next page");
    for ms in &more {
        assert!(
            *ms <= MORE_TIMES_MEDIAN * first_median,
            "a frame where the next page loads or lands costs {ms:.2} ms, over {MORE_TIMES_MEDIAN} × the pass's median {first_median:.2}"
        );
    }
    if !cfg!(debug_assertions) {
        assert!(
            worst <= WORST_MS,
            "the worst frame costs {worst:.2} ms, over {WORST_MS}"
        );
    }
}
