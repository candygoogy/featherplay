//! A measurement of a live resize at 4K 150 %, offscreen, through the app's own present path:
//! the window's right edge dragged wider a few physical pixels at a time, one frame per step,
//! on Home (Up next open and closed) and on Now playing. Each step is what one WM_SIZE and its
//! WM_PAINT cost the UI thread: the resize, the layout, the render and the copy. The drags run
//! as the window's do, inside a live resize (`live_resize`), which keeps the pixel buffer;
//! `RESIZE_FRAMES=new-buffers` leaves that out, so every step makes a new one as before.
//!
//! A measurement, not a check (CPU-bound, it depends on the machine), so it's ignored. Run it
//! optimised:
//!
//! ```sh
//! CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
//!   FEATHERPLAY_HOME=<dir> RUSTFLAGS="--cfg ci" \
//!   cargo test --release -p featherplay --test app -- resize_frames:: --ignored --nocapture
//! ```

use crate::support::{cpu, home_at, quantile};
use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::Frame;
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalSize, PhysicalSize};

const W: u32 = 3840;
const H: u32 = 2160;
const SCALE: f32 = 1.5;
/// Physical pixels per step: a quick drag at 60 Hz.
const STEP: u32 = 12;
const STEPS: u32 = 80;

fn resize(w: &slint::Window, width: u32, height: u32) {
    w.set_size(PhysicalSize::new(width, height));
    w.dispatch_event(WindowEvent::Resized {
        size: LogicalSize::new(width as f32 / SCALE, height as f32 / SCALE),
    });
}

/// Drags the right edge from `W - STEPS * STEP` to `W`, one frame per step, then reports.
fn drag(scene: &Scene, label: &str) {
    let w = scene.ui.window();
    let from = W - STEPS * STEP;
    resize(w, from, H);
    scene.settle();
    for _ in 0..30 {
        offscreen::advance(crate::support::FRAME_240);
        offscreen::present(w).expect("present");
    }
    let live = std::env::var("RESIZE_FRAMES").map_or(true, |v| v != "new-buffers");
    featherplay::live_resize::set(w, live);
    let mut layout = Vec::new();
    let mut render = Vec::new();
    let mut whole = Vec::new();
    for i in 1..=STEPS {
        let c0 = cpu::cycles();
        resize(w, from + i * STEP, H);
        let c1 = cpu::cycles();
        offscreen::advance(crate::support::FRAME_240);
        offscreen::present(w).expect("present");
        let c2 = cpu::cycles();
        layout.push(cpu::ms(c1 - c0));
        render.push(cpu::ms(c2 - c1));
        whole.push(cpu::ms(c2 - c0));
    }
    let line = |v: &[f64]| {
        format!(
            "median {:>5.1} ms  p95 {:>5.1} ms  max {:>5.1} ms",
            quantile(v, 0.5),
            quantile(v, 0.95),
            quantile(v, 1.0)
        )
    };
    println!("{label:<24} step   {}", line(&whole));
    println!("{label:<24} resize {}", line(&layout));
    println!("{label:<24} paint  {}", line(&render));
    // The same window repainted whole at its size: what any full frame costs, without the
    // resize (the layout, and caches made for the old size).
    let mut repaint = Vec::new();
    for _ in 0..STEPS {
        let c0 = cpu::cycles();
        featherplay::repaint::all(w);
        offscreen::advance(crate::support::FRAME_240);
        offscreen::present(w).expect("present");
        repaint.push(cpu::ms(cpu::cycles() - c0));
    }
    println!("{label:<24} whole  {}", line(&repaint));
    // Taller only: the width stays, so nothing laid out across changes.
    let mut taller = Vec::new();
    for i in 1..=STEPS / 2 {
        let c0 = cpu::cycles();
        resize(w, W, H - STEPS / 2 * STEP + i * STEP);
        offscreen::advance(crate::support::FRAME_240);
        offscreen::present(w).expect("present");
        taller.push(cpu::ms(cpu::cycles() - c0));
    }
    println!("{label:<24} taller {}", line(&taller));
    featherplay::live_resize::set(w, false);
}

#[test]
#[ignore = "a measurement: run it optimised, see the module docs"]
fn drag_wider_at_4k() {
    let scene = home_at(W, H, SCALE);
    drag(&scene, "home, up next open");
    scene.ui.global::<Frame>().set_queue_open(false);
    scene.settle();
    drag(&scene, "home, up next closed");
    scene.open("now-playing").expect("now playing");
    scene.settle();
    drag(&scene, "now playing");
}
