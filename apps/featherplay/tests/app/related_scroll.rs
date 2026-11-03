//! Scrolling Related in the side panel stays within a 240 Hz frame at 4K: on the offscreen
//! platform, through the app's own present path, 3840×2160 at scale 1.5 with Up next open on
//! Related, 300 rows with art on every one (`support::fixture_covers`), over Home.
//!
//! Related's rows are `SongRow`s in a `ListView` (only the rows on screen exist) and their art
//! is decoded at its displayed size (`ROW_COVER` × the scale), so it's drawn 1:1, with no
//! resampling and no layer. Measured 2026-10-30 (release): 2.3 / 2.9 ms a frame, the same as
//! Up next's 2.1 / 2.7 ms, and art adds nothing. Art's share holds in any build (skia is
//! optimised in debug too); the whole frame's ceiling only in an optimised one.

use crate::support::{
    TABS_H, TABS_Y, click_tab, frames, home_at, long_rows, panel, quantile, wheel_run,
};
use featherplay::ui::{NowPlayingData, TrackRow};
use slint::{ComponentHandle, Image, LogicalPosition, Model};

const W: u32 = 3840;
const H: u32 = 2160;
const SCALE: f32 = 1.5;
const WIDTH: f32 = W as f32 / SCALE;
/// What art may add to a frame's median and p95, ms of CPU.
const ART_MEDIAN_MS: f64 = 1.0;
const ART_P95_MS: f64 = 1.5;
/// A whole frame's ceiling, optimised (median, p95): the median within 240 Hz (4.17 ms).
const FRAME_MEDIAN_MS: f64 = 4.0;
const FRAME_P95_MS: f64 = 6.0;

/// CPU per repainted frame of a wheel run down and back up the list, the cheaper of two runs
/// after a first one (which creates the rows).
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
    let _ = run();
    let (a, b) = (run(), run());
    if a.len() == b.len() {
        a.iter().zip(&b).map(|(x, y)| x.min(*y)).collect()
    } else {
        a
    }
}

#[test]
fn a_4k_related_scroll_frame_fits_240_hz() {
    let scene = home_at(W, H, SCALE);
    let ui = &scene.ui;
    let w = ui.window();
    click_tab(ui, WIDTH, 1);
    scene.settle();
    let d = ui.global::<NowPlayingData>();
    let rows: Vec<TrackRow> = d.get_related().iter().collect();
    let with_art = rows.iter().filter(|r| r.cover.size().width > 0).count();
    assert!(
        !rows.is_empty() && with_art == rows.len(),
        "{with_art} of {} related rows have art",
        rows.len()
    );
    d.set_related(long_rows(&rows, 300));
    scene.settle();
    frames(w, 60, |_| {});
    let (x0, side) = panel(ui, WIDTH);
    let at = LogicalPosition::new(x0 + side / 2.0, TABS_Y + TABS_H + 500.0);

    // CPU time per frame is noisy when other builds load the machine: up to 3 tries, and
    // the test fails only if every one is over.
    let mut last = String::new();
    for _ in 0..3 {
        d.set_related(long_rows(&rows, 300));
        scene.settle();
        frames(w, 60, |_| {});
        let art = scroll(w, at);
        let bare: Vec<TrackRow> = rows
            .iter()
            .cloned()
            .map(|mut r| {
                r.cover = Image::default();
                r
            })
            .collect();
        d.set_related(long_rows(&bare, 300));
        scene.settle();
        frames(w, 60, |_| {});
        let none = scroll(w, at);
        let (am, ap) = (quantile(&art, 0.5), quantile(&art, 0.95));
        let (bm, bp) = (quantile(&none, 0.5), quantile(&none, 0.95));
        println!(
            "4K 150 % Related scroll, ms of CPU a frame (median / p95): art {am:.2} / {ap:.2}, no art {bm:.2} / {bp:.2}, {} and {} frames",
            art.len(),
            none.len()
        );
        assert!(art.len() > 100, "only {} frames repainted", art.len());
        last = if am - bm > ART_MEDIAN_MS || ap - bp > ART_P95_MS {
            format!(
                "art adds {:.2} / {:.2} ms to a frame (median / p95), over {ART_MEDIAN_MS} / {ART_P95_MS}",
                am - bm,
                ap - bp
            )
        } else if !cfg!(debug_assertions) && (am > FRAME_MEDIAN_MS || ap > FRAME_P95_MS) {
            format!(
                "a Related frame costs {am:.2} / {ap:.2} ms, over {FRAME_MEDIAN_MS} / {FRAME_P95_MS}"
            )
        } else {
            return;
        };
    }
    panic!("{last}");
}
