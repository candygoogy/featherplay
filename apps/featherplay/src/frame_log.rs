//! The real window's frames, for measuring smoothness on the user's own screen: with
//! `FEATHERPLAY_FRAME_LOG` set, logged once a second while frames come. Off (the default), a
//! WM_PAINT or a wheel message costs one read of a cached flag.
//!
//! Each line has how many WM_PAINTs there were, how long each took (Slint's render plus
//! softbuffer's BitBlt), how much of the window each one copied (the renderer's damage, in
//! millions of physical pixels: a frame that repaints across the window shows here), the gaps
//! between frames in a run of them, the whole-window repaints from an update region, the wheel messages (and how many carried a fraction of a notch), and
//! the UI thread's own CPU time inside those paints: user time is Slint's work, kernel time is
//! mostly GDI's copy to the window, and the rest of the paint time was spent waiting
//! (preempted, or blocked in the copy). Windows counts thread time in whole clock ticks
//! (15.6 ms), so only the sums over the second mean something, not single frames.
//!
//! The window's paint subclass (`featherplay_platform::window::repaint_when_invalidated`)
//! calls `around_paint` and `wheel` here.

use std::cell::RefCell;
use std::sync::OnceLock;
use std::sync::atomic::Ordering;

use i_slint_renderer_skia::software_surface::PRESENTED_PIXELS;
use std::time::{Duration, Instant};

/// Frames further apart than this start a new run (the window went idle in between).
const RUN_GAP: Duration = Duration::from_millis(100);

/// A moment on this thread: the clock, its user and kernel time so far (100 ns units), and
/// the pixels presented so far.
#[derive(Clone, Copy)]
struct Mark {
    at: Instant,
    user: u64,
    kernel: u64,
    pixels: u64,
}

impl Mark {
    fn now() -> Self {
        // In 100 ns units, as Windows counts them; zero where it didn't say.
        let ticks = |t: Duration| (t.as_nanos() / 100) as u64;
        let (user, kernel) = featherplay_platform::thread::user_and_kernel_time()
            .map_or((0, 0), |(user, kernel)| (ticks(user), ticks(kernel)));
        Self {
            at: Instant::now(),
            user,
            kernel,
            pixels: PRESENTED_PIXELS.load(Ordering::Relaxed),
        }
    }
}

#[derive(Default)]
struct Window {
    since: Option<Instant>,
    last: Option<Instant>,
    paints: Vec<f64>,
    /// Millions of physical pixels each paint copied to the window.
    copied: Vec<f64>,
    gaps: Vec<f64>,
    whole: u32,
    /// Wheel messages, and how many of them carried a fraction of a notch.
    wheels: u32,
    fractional: u32,
    /// This thread's CPU time inside the paints, in 100 ns units.
    user: u64,
    kernel: u64,
}

thread_local! {
    static LOG: RefCell<Window> = RefCell::new(Window::default());
}

fn enabled() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| std::env::var_os("FEATHERPLAY_FRAME_LOG").is_some())
}

fn quantile(v: &mut [f64], q: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * q).round() as usize]
}

/// A wheel message with `delta`, in 120ths of a notch.
pub fn wheel(delta: i16) {
    if !enabled() {
        return;
    }
    LOG.with_borrow_mut(|w| {
        w.wheels += 1;
        w.fractional += u32::from(delta % 120 != 0);
    });
}

/// Runs one WM_PAINT's drawing, `paint`, timing it when the frame log or the performance
/// harness wants it. `whole`: the paint had an update region.
pub fn around_paint(whole: bool, paint: &mut dyn FnMut()) {
    #[cfg(feature = "perf-harness")]
    if crate::perf::on() {
        let started = Instant::now();
        let logged = enabled().then(Mark::now);
        paint();
        if let Some(mark) = logged {
            painted(mark, whole);
        }
        crate::perf::painted(started);
        return;
    }
    if enabled() {
        // winit draws inside this WM_PAINT: Slint's render and softbuffer's BitBlt.
        let started = Mark::now();
        paint();
        painted(started, whole);
        return;
    }
    paint();
}

fn painted(started: Mark, whole: bool) {
    let ended = Mark::now();
    let ms = (ended.at - started.at).as_secs_f64() * 1000.0;
    LOG.with_borrow_mut(|w| {
        if let Some(last) = w.last {
            let gap = started.at - last;
            if gap < RUN_GAP {
                w.gaps.push(gap.as_secs_f64() * 1000.0);
            }
        }
        w.last = Some(started.at);
        w.paints.push(ms);
        w.copied
            .push(ended.pixels.saturating_sub(started.pixels) as f64 / 1e6);
        w.whole += u32::from(whole);
        w.user += ended.user.saturating_sub(started.user);
        w.kernel += ended.kernel.saturating_sub(started.kernel);
        let since = *w.since.get_or_insert(started.at);
        if started.at - since >= Duration::from_secs(1) {
            let frames = w.paints.len();
            let painting: f64 = w.paints.iter().sum();
            let slow = w.gaps.iter().filter(|&&g| g > 8.4).count();
            let (p50, p95, max) = (
                quantile(&mut w.paints, 0.5),
                quantile(&mut w.paints, 0.95),
                quantile(&mut w.paints, 1.0),
            );
            let (g50, gmax) = (quantile(&mut w.gaps, 0.5), quantile(&mut w.gaps, 1.0));
            let (c50, cmax) = (
                quantile(&mut w.copied, 0.5),
                quantile(&mut w.copied, 1.0),
            );
            let (user, kernel) = (w.user as f64 / 1e4, w.kernel as f64 / 1e4);
            log!(
                "frames: {frames} in {:.2} s, paint median {p50:.2} p95 {p95:.2} max {max:.2} ms, copied median {c50:.2} max {cmax:.2} Mpx, gap median {g50:.2} max {gmax:.2} ms, gaps over 8.4 ms {slow}, whole-window {}, wheel messages {} ({} fractional); painting took {painting:.0} ms, of it CPU {user:.0} ms user + {kernel:.0} ms kernel",
                (started.at - since).as_secs_f64(),
                w.whole,
                w.wheels,
                w.fractional
            );
            *w = Window {
                last: w.last,
                ..Window::default()
            };
        }
    });
}
