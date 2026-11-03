//! Frame times, offscreen, through the app's own present path (`offscreen::install_presenting`):
//! a page enter and a fast wheel scroll on Home, Liked songs and a long playlist, maximized at
//! 1920×1080, at 2560×1440 with 150 % scaling, and at 3840×2160 with 150 % and 200 % scaling
//! (Up next open there, as on a 4K desktop). A frame is what the event loop does in one turn:
//! the wheel event, the timers and animations, then the render and the copy to the window.
//! The pointer rests over the list, so rows pass under it as they would.
//!
//! Two scrolls per page: "scroll" is the 60 Hz run of earlier rounds (a notch every 16 ms
//! frame), "scroll 240" steps the clock 4 ms as a 240 Hz display does, with a notch every
//! 32 ms (a fast flick of the wheel). "damage" is the median share of the window a repainted
//! frame copies to the screen. The pointer stays still while the wheel turns, so the rows
//! passing under it don't take the hover (`Frame.scrolling`, as `wheel.rs` sets it).
//!
//! A measurement, not a check (CPU-bound, it depends on the machine), so it's ignored. Run it
//! optimised, as the app ships (opt-level "z"; no LTO only to keep the build short):
//!
//! ```sh
//! CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
//!   FEATHERPLAY_HOME=<dir> RUSTFLAGS="--cfg ci" \
//!   cargo test --release -p featherplay --test app -- render_timing:: --ignored --nocapture
//! ```
//!
//! `RENDER_TIMING=4k` measures only the 4K sizes.

use std::rc::Rc;
use std::time::{Duration, Instant};

use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{Frame, LibraryData, PlaylistData, Theme, TrackRow};
use slint::Window;
use slint::platform::WindowEvent;
use slint::{
    ComponentHandle, LogicalPosition, LogicalSize, Model, ModelRc, PhysicalSize, VecModel,
};

/// One frame at 60 Hz: the budget, and how far the clock moves per frame.
const FRAME: Duration = Duration::from_millis(16);
/// One frame at 240 Hz (4.17 ms; Slint's clock counts whole milliseconds).
const FRAME_240: Duration = Duration::from_millis(4);
/// Rows in the long lists (the fixtures' rows, repeated).
const LONG: usize = 300;

struct Stats {
    frames: Vec<f64>,
    /// Physical pixels each frame copied to the window.
    damaged: Vec<u64>,
    /// The window's physical pixels.
    window: u64,
    /// The frame budget the run is held to, in ms.
    budget: f64,
}

impl Stats {
    fn repainted(&self) -> usize {
        self.damaged.iter().filter(|&&d| d > 0).count()
    }

    fn line(&self, what: &str) -> String {
        let mut v = self.frames.clone();
        v.sort_by(f64::total_cmp);
        let at = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
        let over = v.iter().filter(|&&t| t > self.budget).count();
        let mut damaged: Vec<u64> = self.damaged.iter().copied().filter(|&d| d > 0).collect();
        damaged.sort_unstable();
        let share = damaged
            .get(damaged.len() / 2)
            .map_or(0.0, |&d| d as f64 * 100.0 / self.window as f64);
        format!(
            "{what:<30} {:>3} frames ({:>3} repainted, damage {share:>3.0} %)  median {:>5.1} ms  p95 {:>5.1} ms  max {:>5.1} ms  over {:.0} ms {over}",
            v.len(),
            self.repainted(),
            at(0.5),
            at(0.95),
            at(1.0),
            self.budget,
        )
    }
}

/// `n` frames, the clock stepping `step` each; `each` runs first in each one (input).
fn frames(w: &Window, n: u32, step: Duration, mut each: impl FnMut(u32)) -> Stats {
    let mut frames = Vec::with_capacity(n as usize);
    let mut damaged = Vec::with_capacity(n as usize);
    for i in 0..n {
        let started = Instant::now();
        each(i);
        offscreen::advance(step);
        damaged.push(offscreen::present(w).expect("present").damaged);
        frames.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    let size = w.size();
    Stats {
        frames,
        damaged,
        window: u64::from(size.width) * u64::from(size.height),
        budget: step.as_secs_f64() * 1000.0,
    }
}

fn resize(w: &Window, width: u32, height: u32) {
    w.set_size(PhysicalSize::new(width, height));
    let scale = w.scale_factor();
    w.dispatch_event(WindowEvent::Resized {
        size: LogicalSize::new(width as f32 / scale, height as f32 / scale),
    });
}

/// Moves the window to a display of `width`×`height` at `scale`, as Windows does when it's
/// dragged there and maximized; covers are decoded for the new scale, as the app does.
fn display(scene: &Scene, width: u32, height: u32, scale: f32) {
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::ScaleFactorChanged {
        scale_factor: scale,
    });
    scene.ctx.covers.set_scale(scale);
    resize(w, width, height);
    frames(w, 30, FRAME, |_| {});
}

/// The loaded rows, repeated to `LONG`. The current track stays marked in the first round
/// only: a list shows it once.
fn long(rows: ModelRc<TrackRow>) -> ModelRc<TrackRow> {
    let loaded: Vec<TrackRow> = rows.iter().collect();
    assert!(!loaded.is_empty(), "no rows loaded");
    let many: Vec<TrackRow> = (0..LONG)
        .map(|i| {
            let mut row = loaded[i % loaded.len()].clone();
            row.playing &= i < loaded.len();
            row
        })
        .collect();
    ModelRc::from(Rc::new(VecModel::from(many)))
}

/// Opens `page`, makes its list long, and lets it settle.
fn open(scene: &Scene, page: &str) {
    scene.open(page).expect("page");
    scene.settle();
    let ui = &scene.ui;
    match page {
        "library" => {
            let d = ui.global::<LibraryData>();
            d.set_liked(long(d.get_liked()));
        }
        "playlist" => {
            let d = ui.global::<PlaylistData>();
            d.set_tracks(long(d.get_tracks()));
        }
        _ => {}
    }
    scene.settle();
}

fn page_timings(scene: &Scene, label: &str) {
    let w = scene.ui.window();
    // Over the page's list, left of Up next when it's open.
    let list = LogicalPosition::new(w.size().to_logical(w.scale_factor()).width * 0.45, 500.0);
    for page in ["home", "library", "playlist"] {
        // Library opens on its Liked songs tab.
        let name = if page == "library" { "liked" } else { page };
        let mut enter = frames(w, 20, FRAME, |i| {
            if i == 0 {
                open(scene, page);
            }
        });
        // The first frame also loads the page (fixtures, covers): not a render cost.
        enter.frames.remove(0);
        enter.damaged.remove(0);
        println!("{}", enter.line(&format!("{label} {name} enter")));
        w.dispatch_event(WindowEvent::PointerMoved { position: list });
        frames(w, 10, FRAME, |_| {});
        scene.ui.global::<Frame>().set_scrolling(true);
        let scroll = frames(w, 90, FRAME, |i| {
            if i < 60 {
                offscreen::wheel(w, list, -1.0);
            }
        });
        println!("{}", scroll.line(&format!("{label} {name} scroll")));
        // Back up by the same amount (60 notches), at 240 Hz.
        let scroll = frames(w, 300, FRAME_240, |i| {
            if i < 240 && i % 8 == 0 {
                offscreen::wheel(w, list, 2.0);
            }
        });
        println!("{}", scroll.line(&format!("{label} {name} scroll 240")));
        scene.ui.global::<Frame>().set_scrolling(false);
    }
}

#[test]
#[ignore = "a measurement: run it optimised, see the module docs"]
fn scroll_and_enter_frame_times() {
    let only_4k = std::env::var("RENDER_TIMING").is_ok_and(|v| v == "4k");
    let scene = Scene::new(1280, 800, false, true).expect("scene");
    scene.ui.global::<Theme>().set_motion(true);
    scene.play(false).expect("player");
    scene.open("home").expect("home");
    scene.theme(true);
    scene.ui.show().expect("show");
    scene.settle();
    offscreen::freeze_clock();
    let w = scene.ui.window();
    frames(w, 30, FRAME, |_| {});
    if !only_4k {
        // Maximizing from 1280×800, on Home: the frame of the resize and what follows it.
        let max = frames(w, 30, FRAME, |i| {
            if i == 0 {
                resize(w, 1920, 1080);
            }
        });
        println!("{}", max.line("maximize 1280x800 to 1080p"));
        page_timings(&scene, "1080p");

        display(&scene, 2560, 1440, 1.5);
        page_timings(&scene, "1440p 150 %");
    }

    scene.ui.global::<Frame>().set_queue_open(true);
    for (scale, label) in [(1.5, "4K 150 %"), (2.0, "4K 200 %")] {
        display(&scene, 3840, 2160, scale);
        page_timings(&scene, label);
    }
}
