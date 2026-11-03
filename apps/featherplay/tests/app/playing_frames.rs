//! Sound playing costs the window's frames nothing beyond the playing mark, through the
//! renderer's own frames as the app's window presents them (`offscreen::install_presenting`).
//! No window is shown.
//!
//! Slint keeps at most three dirty rectangles a frame and merges any more into bounding
//! boxes, so a few small changes far apart can turn one frame into a repaint of most of the
//! window, and anything that changes every frame while playing makes every frame repaint more.
//! While playing:
//! - a scroll repaints what the same scroll repaints paused, but for the frames where the mark
//!   steps;
//! - with nothing else moving, only the mark's steps repaint, and only small spots.
//!
//! The position stands still here (the fixture's sound isn't audible), so its once-a-second
//! tick doesn't land on frames at the whim of the test machine's speed.

use std::time::Duration;

use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{Frame, Theme};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition, Window};

/// One frame at 240 Hz.
const FRAME: Duration = Duration::from_millis(4);

/// What playing may add to an idle frame, as a share of the window (the marks are 20 px
/// squares).
const SPOTS: f64 = 0.02;
/// What playing may add to a scroll, as a share of what it repaints paused. Only the mark's
/// steps add anything; what they add depends on how the frame's three dirty rectangles merge.
/// While Home's header wash is in view it is one more rectangle, so a step merges into a
/// taller box: 2.07 % of this scroll (1.80 % without the wash, 2026-10-31).
const SCROLL_SPOTS: f64 = 0.025;

/// Frames between two steps of the mark (`Mark.step`, 33 ms, at 240 Hz), rounded down.
const STEP_FRAMES: usize = 8;

/// Physical pixels each of `n` frames copied to the window; `each` runs first in each one.
fn damage(w: &Window, n: u32, mut each: impl FnMut(u32)) -> Vec<u64> {
    (0..n)
        .map(|i| {
            each(i);
            offscreen::advance(FRAME);
            offscreen::present(w).expect("present").damaged
        })
        .collect()
}

/// A wheel run down Home's feed (a notch every 8 frames), damage per frame, then back to the
/// top so the next run starts from the same place.
fn scroll(w: &Window, at: LogicalPosition) -> Vec<u64> {
    let run = damage(w, 160, |i| {
        if i % 8 == 0 {
            offscreen::wheel(w, at, -1.0);
        }
    });
    damage(w, 160, |i| {
        if i % 8 == 0 {
            offscreen::wheel(w, at, 1.0);
        }
    });
    damage(w, 120, |_| {});
    run
}

/// How many times the mark steps over `frames` frames, at most.
fn steps(frames: usize) -> usize {
    frames / STEP_FRAMES + 1
}

/// One test: the platform can be installed once per process.
#[test]
fn playing_repaints_only_the_marks_steps() {
    let scene = Scene::new(1280, 800, false, true).expect("scene");
    scene.ui.global::<Theme>().set_motion(true);
    scene.sound(false).expect("player");
    scene.open("home").expect("home");
    scene.theme(true);
    scene.ui.show().expect("show");
    scene.settle();
    offscreen::freeze_clock();
    let w = scene.ui.window();
    // Up next open: its current row has the playing mark.
    scene.ui.global::<Frame>().set_queue_open(true);
    damage(w, 120, |_| {});
    let at = LogicalPosition::new(500.0, 500.0);
    w.dispatch_event(WindowEvent::PointerMoved { position: at });
    scene.ui.global::<Frame>().set_scrolling(true);
    // A first pass fills the caches.
    scroll(w, at);
    let size = w.size();
    let window = f64::from(size.width) * f64::from(size.height);

    let paused = scroll(w, at);
    scene.play(false).expect("player");
    // The play button's morph to pause settles first.
    damage(w, 250, |_| {});
    let playing = scroll(w, at);
    assert!(
        paused.iter().filter(|&&d| d > 0).count() > 100,
        "the scroll didn't repaint: the test lost its point"
    );
    // A step of the mark adds its few pixels, or, when their rectangles are too many for
    // the frame's three, what merging them into the scroll's own adds (5 % of the window,
    // once, in this run).
    let extra: Vec<f64> = playing
        .iter()
        .zip(&paused)
        .map(|(p, s)| (*p as f64 - *s as f64) / window)
        .collect();
    let added = extra.iter().filter(|&&e| e > 0.001).count();
    assert!(
        added <= steps(playing.len()),
        "{added} scroll frames repainted more while playing than paused; the mark steps {} times",
        steps(playing.len())
    );
    let more: f64 = extra.iter().sum();
    let total: f64 = paused.iter().map(|&d| d as f64 / window).sum();
    assert!(
        more <= total * SCROLL_SPOTS,
        "playing repainted {:.2} % more of the scroll than paused",
        more / total * 100.0
    );

    scene.ui.global::<Frame>().set_scrolling(false);
    damage(w, 20, |_| {});
    // A second and a half: 45 steps of the mark.
    let idle = damage(w, 375, |_| {});
    let repainted = idle.iter().filter(|&&d| d > 0).count();
    assert!(
        (30..=steps(idle.len())).contains(&repainted),
        "{repainted} frames repainted while playing; the mark steps {} times",
        steps(idle.len())
    );
    for (i, d) in idle.iter().enumerate() {
        let share = *d as f64 / window;
        assert!(
            share <= SPOTS,
            "idle frame {i} repainted {:.1} % of the window while playing",
            share * 100.0
        );
    }
}
