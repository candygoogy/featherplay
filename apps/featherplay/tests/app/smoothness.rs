//! The smoothness benchmark: frame times at 3840×2160, offscreen, through the app's own present
//! path (`offscreen::install_presenting`): scrolls (paused and playing), page enters, hovers
//! and the Up next slide, and the price of a small continuously animated region
//! (`ui/probe.slint`).
//!
//! A frame is what the event loop does in one turn: the input, the timers and animations
//! (`advance`, which also re-lays out what changed), then the render and the copy to the
//! window (`present`). "full" marks the whole window dirty before every frame, as a renderer
//! without partial rendering would draw it. Up next is open, as in the user's recording.
//!
//! Scrolls are a fast wheel at 240 Hz: the clock steps 4 ms, a two-notch event every 32 ms,
//! 30 events down and 30 back up (600 frames); only frames that repainted count. Pages are
//! long: Home 12 shelves of 12 cards, Liked songs and a playlist 300 rows, the albums grid 150
//! cards, all with art (`support::fixture_covers`).
//!
//! A measurement, not a check, so it's ignored. Run it optimised, as the app ships:
//!
//! ```sh
//! CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
//!   FEATHERPLAY_HOME=<dir> RUSTFLAGS="--cfg ci" \
//!   cargo test --release -p featherplay --test app -- smoothness:: --ignored --nocapture
//! ```
//!
//! `SMOOTHNESS=quick` measures 150 % only; `SMOOTHNESS_ONLY=scroll|playing|region|transitions`
//! runs one part; `SMOOTHNESS_ROUNDS=n` sets the rounds (3).

use crate::support;
use std::rc::Rc;
use std::time::{Duration, Instant};

use featherplay::offscreen;
use featherplay::repaint;
use featherplay::snapshot::Scene;
use featherplay::ui::{CardItem, Frame, LibraryData, Mark, PlaylistData, Probe, Theme, TrackRow};
use slint::platform::WindowEvent;
use slint::{
    ComponentHandle, LogicalPosition, LogicalSize, Model, ModelRc, PhysicalSize, VecModel, Window,
};

/// One frame at 240 Hz (Slint's clock counts whole milliseconds).
const FRAME_240: Duration = Duration::from_millis(4);
/// The 240 Hz budget, in ms.
const BUDGET_240: f64 = 1000.0 / 240.0;
/// Rows in the long lists (the fixtures' rows, repeated).
const LONG: usize = 300;
/// Shelves on the long Home (the fixtures' shelves, repeated), as a real feed has.
const HOME_SHELVES: usize = 12;
/// Cards per shelf on the long Home (the shelf's own, repeated): a real shelf fills its row.
const HOME_SHELF_ITEMS: usize = 12;
/// The machine's logical cores (`[Environment]::ProcessorCount`), for the CPU budget.
const CORES: f64 = 16.0;

/// This thread's CPU time, from Windows' per-thread cycle counter, and the process raised to
/// high priority so the measured thread is preempted less.
mod cpu {
    use std::sync::OnceLock;
    use std::time::{Duration, Instant};

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThread() -> isize;
        fn GetCurrentProcess() -> isize;
        fn QueryThreadCycleTime(thread: isize, cycles: *mut u64) -> i32;
        fn SetPriorityClass(process: isize, class: u32) -> i32;
    }

    const HIGH_PRIORITY_CLASS: u32 = 0x80;

    /// Cycles this thread has run.
    pub fn cycles() -> u64 {
        let mut c = 0;
        // SAFETY: the pseudo-handle of the calling thread and a valid out pointer.
        unsafe { QueryThreadCycleTime(GetCurrentThread(), &mut c) };
        c
    }

    /// Cycles per millisecond: the most a 100 ms spin got (a preempted spin gets fewer).
    fn rate() -> f64 {
        static RATE: OnceLock<f64> = OnceLock::new();
        *RATE.get_or_init(|| {
            (0..5)
                .map(|_| {
                    let (t, c) = (Instant::now(), cycles());
                    while t.elapsed() < Duration::from_millis(100) {
                        std::hint::spin_loop();
                    }
                    (cycles() - c) as f64 / (t.elapsed().as_secs_f64() * 1000.0)
                })
                .fold(0.0, f64::max)
        })
    }

    pub fn ms(cycles: u64) -> f64 {
        cycles as f64 / rate()
    }

    /// High priority, and the rate calibrated now.
    pub fn prepare() -> f64 {
        // SAFETY: the pseudo-handle of this process.
        unsafe { SetPriorityClass(GetCurrentProcess(), HIGH_PRIORITY_CLASS) };
        rate()
    }
}

#[derive(Default)]
struct Stats {
    /// Whole frames (input, timers, layout, render, copy), ms.
    frames: Vec<f64>,
    /// The same frames in this thread's own CPU time (its cycles, at the calibrated rate), ms:
    /// what the frame costs with the machine to itself. Other agents compiling on the same
    /// machine stretch the wall clock (preemption) far more than this.
    cpu: Vec<f64>,
    /// Physical pixels each frame copied to the window.
    damaged: Vec<u64>,
    window: u64,
}

fn quantile(v: &[f64], q: f64) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    let mut v = v.to_vec();
    v.sort_by(f64::total_cmp);
    v[((v.len() - 1) as f64 * q).round() as usize]
}

impl Stats {
    /// Only the frames that repainted.
    fn repainted(&self) -> Stats {
        let keep: Vec<usize> = (0..self.damaged.len())
            .filter(|&i| self.damaged[i] > 0)
            .collect();
        Stats {
            frames: keep.iter().map(|&i| self.frames[i]).collect(),
            cpu: keep.iter().map(|&i| self.cpu[i]).collect(),
            damaged: keep.iter().map(|&i| self.damaged[i]).collect(),
            window: self.window,
        }
    }

    fn median_damage(&self) -> f64 {
        let mut d = self.damaged.clone();
        d.sort_unstable();
        d.get(d.len() / 2)
            .map_or(0.0, |&d| d as f64 * 100.0 / self.window as f64)
    }

    fn line(&self, what: &str, budget: f64) -> String {
        let r = self.repainted();
        let over = r.cpu.iter().filter(|&&t| t > budget).count();
        format!(
            "{what:<50} {:>3} repainted, damage {:>6.2} % (max {:>5.1} %)  cpu median {:>5.2} p95 {:>5.2} max {:>6.2} ms (over {budget:.2} ms: {over:>3})  wall median {:>5.2} p95 {:>5.2} ms",
            r.frames.len(),
            r.median_damage(),
            r.damaged
                .iter()
                .max()
                .map_or(0.0, |&d| d as f64 * 100.0 / r.window as f64),
            quantile(&r.cpu, 0.5),
            quantile(&r.cpu, 0.95),
            quantile(&r.cpu, 1.0),
            quantile(&r.frames, 0.5),
            quantile(&r.frames, 0.95),
        )
    }
}

/// `n` frames, the clock stepping `step` each; `each` runs first in each one (input).
/// `full`: the whole window is marked dirty first.
fn frames(w: &Window, n: u32, step: Duration, full: bool, mut each: impl FnMut(u32)) -> Stats {
    let mut s = Stats::default();
    for i in 0..n {
        let started = Instant::now();
        let cycles = cpu::cycles();
        each(i);
        offscreen::advance(step);
        if full {
            repaint::all(w);
        }
        let presented = offscreen::present(w).expect("present");
        s.cpu.push(cpu::ms(cpu::cycles() - cycles));
        if std::env::var_os("SMOOTHNESS_HEAVY").is_some() && s.cpu[s.cpu.len() - 1] > 2.0 {
            println!(
                "  heavy frame {i}: {:.2} ms, {} px",
                s.cpu[s.cpu.len() - 1],
                presented.damaged
            );
        }
        s.damaged.push(presented.damaged);
        s.frames.push(started.elapsed().as_secs_f64() * 1000.0);
    }
    let size = w.size();
    s.window = u64::from(size.width) * u64::from(size.height);
    s
}

fn merge(a: Stats, b: Stats) -> Stats {
    Stats {
        frames: [a.frames, b.frames].concat(),
        cpu: [a.cpu, b.cpu].concat(),
        damaged: [a.damaged, b.damaged].concat(),
        window: a.window,
    }
}

fn resize(w: &Window, width: u32, height: u32) {
    w.set_size(PhysicalSize::new(width, height));
    let scale = w.scale_factor();
    w.dispatch_event(WindowEvent::Resized {
        size: LogicalSize::new(width as f32 / scale, height as f32 / scale),
    });
}

fn display(scene: &Scene, width: u32, height: u32, scale: f32) {
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::ScaleFactorChanged {
        scale_factor: scale,
    });
    scene.ctx.covers.set_scale(scale);
    resize(w, width, height);
    scene.settle();
    frames(w, 30, FRAME_240, false, |_| {});
}

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
        "home" => support::long_home(ui, HOME_SHELVES, HOME_SHELF_ITEMS),
        "library" => {
            let d = ui.global::<LibraryData>();
            d.set_liked(long(d.get_liked()));
        }
        "playlist" => {
            let d = ui.global::<PlaylistData>();
            d.set_tracks(long(d.get_tracks()));
        }
        "library-albums" => {
            let d = ui.global::<LibraryData>();
            let loaded: Vec<CardItem> = d.get_cards().iter().collect();
            assert!(!loaded.is_empty(), "no albums loaded");
            let many: Vec<CardItem> = (0..LONG / 2)
                .map(|i| loaded[i % loaded.len()].clone())
                .collect();
            d.set_cards(ModelRc::from(Rc::new(VecModel::from(many))));
        }
        _ => {}
    }
    scene.settle();
    frames(ui.window(), 60, FRAME_240, false, |_| {});
}

/// The wheel run: 30 two-notch events down, then 30 back up, at 240 Hz.
fn scroll(scene: &Scene, at: LogicalPosition, full: bool) -> Stats {
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::PointerMoved { position: at });
    frames(w, 10, FRAME_240, false, |_| {});
    scene.ui.global::<Frame>().set_scrolling(true);
    let run = |notches: f32| {
        frames(w, 300, FRAME_240, full, |i| {
            if i < 240 && i % 8 == 0 {
                offscreen::wheel(w, at, notches);
            }
        })
    };
    let down = run(-2.0);
    let up = run(2.0);
    scene.ui.global::<Frame>().set_scrolling(false);
    merge(down, up)
}

/// How a scroll is measured: as it comes, or with the whole window marked dirty before every
/// frame (what a renderer without partial rendering would draw).
#[derive(Clone, Copy, PartialEq)]
enum Variant {
    Baseline,
    Full,
}

impl Variant {
    const ALL: [Variant; 2] = [Variant::Baseline, Variant::Full];

    fn name(self) -> &'static str {
        match self {
            Variant::Baseline => "baseline",
            Variant::Full => "full repaint",
        }
    }
}

/// Rounds of every measurement. The runs are deterministic (the same frames at the same
/// scroll positions), so each frame keeps its cheapest round: other work on the machine only
/// ever adds time.
fn rounds() -> usize {
    std::env::var("SMOOTHNESS_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(3)
}

/// Frame by frame, the cheapest of the runs.
fn best(runs: Vec<Stats>) -> Stats {
    let mut runs = runs.into_iter();
    let mut out = runs.next().expect("at least one run");
    for r in runs {
        if r.cpu.len() != out.cpu.len() {
            continue;
        }
        for i in 0..out.cpu.len() {
            if r.cpu[i] < out.cpu[i] {
                out.cpu[i] = r.cpu[i];
                out.frames[i] = r.frames[i];
                out.damaged[i] = r.damaged[i];
            }
        }
    }
    out
}

/// Wheel runs on Home, Liked songs and a 300-row playlist: a first pass, then the best of the
/// rounds as they come and with the whole window repainted.
fn scrolls(scene: &Scene, label: &str) {
    let w = scene.ui.window();
    let list = LogicalPosition::new(w.size().to_logical(w.scale_factor()).width * 0.4, 600.0);
    for page in ["home", "library", "library-albums", "playlist"] {
        let name = match page {
            "library" => "liked",
            "library-albums" => "albums grid",
            _ => page,
        };
        open(scene, page);
        // A first pass instantiates the rows and fills the caches: not what a scroll costs.
        let first = scroll(scene, list, false);
        println!(
            "{}",
            first.line(&format!("{label} {name} first pass"), BUDGET_240)
        );
        let mut runs: Vec<Vec<Stats>> = Variant::ALL.iter().map(|_| Vec::new()).collect();
        for _ in 0..rounds() {
            for (k, &v) in Variant::ALL.iter().enumerate() {
                runs[k].push(scroll(scene, list, v == Variant::Full));
                frames(w, 20, FRAME_240, false, |_| {});
            }
        }
        for (v, r) in Variant::ALL.iter().zip(runs) {
            println!(
                "{}",
                best(r).line(&format!("{label} {name} {}", v.name()), BUDGET_240)
            );
        }
    }
}

/// Paused against playing: the same scrolls with the player's `playing` flag off and on,
/// Up next closed and open, so what playing adds to a scroll frame shows on its own.
fn paused_playing(scene: &Scene, label: &str) {
    let ui = &scene.ui;
    let w = ui.window();
    let list = LogicalPosition::new(w.size().to_logical(w.scale_factor()).width * 0.4, 600.0);
    let frame = ui.global::<Frame>();
    for queue in [false, true] {
        frame.set_queue_open(queue);
        frames(w, 120, FRAME_240, false, |_| {});
        for page in ["home", "playlist"] {
            open(scene, page);
            let _ = scroll(scene, list, false);
            for playing in [false, true] {
                scene.sound(playing).expect("player");
                frames(w, 20, FRAME_240, false, |_| {});
                let runs = (0..rounds()).map(|_| scroll(scene, list, false)).collect();
                let s = best(runs);
                let r = s.repainted();
                let big = r.damaged.iter().filter(|&&d| d * 10 > r.window * 9).count();
                println!(
                    "{}  frames over 90 % damage: {big}",
                    s.line(
                        &format!(
                            "{label} {page} {} queue {}",
                            if playing { "PLAYING" } else { "paused" },
                            if queue { "open" } else { "closed" }
                        ),
                        BUDGET_240
                    )
                );
            }
        }
    }
    scene.play(false).expect("player");
}

/// CPU share of the whole machine for `fps` frames a second of `ms` each.
fn machine_share(fps: f64, ms: f64) -> f64 {
    fps * ms / 1000.0 / CORES * 100.0
}

/// `fps` frames of `each`, best of the rounds, printed with its CPU share of the machine.
fn price(w: &Window, what: &str, fps: u64, each: impl Fn()) {
    let step = Duration::from_millis(1000 / fps);
    let runs = (0..rounds())
        .map(|_| frames(w, 120, step, false, |_| each()))
        .collect();
    let s = best(runs);
    let median = quantile(&s.repainted().cpu, 0.5);
    println!(
        "{}  CPU {:.4} % of machine",
        s.line(what, 1000.0 / fps as f64),
        machine_share(fps as f64, median),
    );
}

/// A small region animating on its own, the page still: the probe's 24×24 bars, and the app's
/// own playing mark (on the playlist's playing row and in Up next).
fn animated_region(scene: &Scene, label: &str) {
    let ui = &scene.ui;
    let w = ui.window();
    let size = w.size().to_logical(w.scale_factor());
    let bar_top = size.height - ui.global::<Theme>().get_bar_clear();
    let probe = ui.global::<Probe>();
    let frame = ui.global::<Frame>();
    for page in ["playlist", "about"] {
        open(scene, page);
        // The pointer away from everything.
        w.dispatch_event(WindowEvent::PointerMoved {
            position: LogicalPosition::new(size.width * 0.4, 20.0),
        });
        frames(w, 30, FRAME_240, false, |_| {});
        let spots = [
            ("in the bar", 330.0, bar_top + 32.0),
            ("over the page", size.width * 0.4, 700.0),
        ];
        // The app's own playing mark steps at 30 fps on its own timer while playing (the
        // playing row, Up next); `Frame.hidden` stops that timer and nothing else.
        for (alone, with) in [(true, "alone"), (false, "+ app mark 30 fps")] {
            frame.set_hidden(alone);
            for (spot, x, y) in spots {
                for (single, shape) in [(false, "3 rects"), (true, "1 path")] {
                    probe.set_region_x(x);
                    probe.set_region_y(y);
                    probe.set_region_single(single);
                    probe.set_region(true);
                    frames(w, 10, FRAME_240, false, |_| {});
                    for fps in [30, 60] {
                        price(
                            w,
                            &format!("{label} {page}: region {shape} {spot}, {with}, {fps} fps"),
                            fps,
                            || probe.set_tick(probe.get_tick() + 1),
                        );
                    }
                    probe.set_region(false);
                    frames(w, 10, FRAME_240, false, |_| {});
                }
            }
        }
        frame.set_hidden(false);
    }
    // The app's own mark (the playlist's playing row and Up next), stepped every frame.
    open(scene, "playlist");
    frame.set_hidden(true);
    let mark = ui.global::<Mark>();
    for fps in [10, 30, 60] {
        price(
            w,
            &format!("{label} playlist: PlayingMark (row + Up next) {fps} fps"),
            fps,
            || mark.set_frame(mark.get_frame() + 1),
        );
    }
    frame.set_hidden(false);
}

/// `n` frames after `start`, best of the rounds; `undo` puts things back between rounds.
fn transition(w: &Window, what: &str, n: u32, start: impl Fn(), undo: impl Fn()) {
    let runs = (0..rounds())
        .map(|_| {
            let s = frames(w, n, FRAME_240, false, |i| {
                if i == 0 {
                    start();
                }
            });
            undo();
            frames(w, 120, FRAME_240, false, |_| {});
            s
        })
        .collect();
    println!("{}", best(runs).line(what, BUDGET_240));
}

/// Page enters, hovers and the Up next slide, with motion on.
fn transitions(scene: &Scene, label: &str) {
    let ui = &scene.ui;
    let w = ui.window();
    let size = w.size().to_logical(w.scale_factor());
    let away = LogicalPosition::new(size.width * 0.4, 20.0);
    let point = |at: LogicalPosition| w.dispatch_event(WindowEvent::PointerMoved { position: at });
    let show = |page: &str| {
        scene.open(page).expect("page");
        scene.settle();
        frames(w, 120, FRAME_240, false, |_| {});
    };
    // Every page loaded once, so an enter is the motion, not the load.
    for page in ["home", "library", "playlist"] {
        open(scene, page);
    }
    for (page, back) in [
        ("library", "home"),
        ("playlist", "home"),
        ("home", "library"),
    ] {
        let name = if page == "library" { "liked" } else { page };
        show(back);
        transition(
            w,
            &format!("{label} enter {name}"),
            80,
            || scene.open(page).expect("page"),
            || show(back),
        );
    }
    show("home");
    let card = LogicalPosition::new(size.width * 0.3, 700.0);
    transition(
        w,
        &format!("{label} hover onto a card"),
        60,
        || point(card),
        || point(away),
    );
    show("playlist");
    let row = LogicalPosition::new(size.width * 0.4, 700.0);
    transition(
        w,
        &format!("{label} hover onto a row"),
        60,
        || point(row),
        || point(away),
    );
    let frame = ui.global::<Frame>();
    transition(
        w,
        &format!("{label} Up next closes"),
        100,
        || frame.set_queue_open(false),
        || frame.set_queue_open(true),
    );
    frame.set_queue_open(false);
    frames(w, 120, FRAME_240, false, |_| {});
    transition(
        w,
        &format!("{label} Up next opens"),
        100,
        || frame.set_queue_open(true),
        || frame.set_queue_open(false),
    );
    frame.set_queue_open(true);
    frames(w, 120, FRAME_240, false, |_| {});
}

#[test]
#[ignore = "a measurement: run it optimised, see the module docs"]
fn smoothness() {
    let quick = std::env::var("SMOOTHNESS").is_ok_and(|v| v == "quick");
    println!("thread cycles: {:.0} per ms", cpu::prepare());
    support::fixture_covers();
    let scene = Scene::new(1280, 800, false, true).expect("scene");
    scene.ui.global::<Theme>().set_motion(true);
    let scales: &[(f32, &str)] = if quick {
        &[(1.5, "4K 150 %")]
    } else {
        &[(1.5, "4K 150 %"), (2.0, "4K 200 %")]
    };
    // Covers are decoded for the display's scale, as the app does: set before Home loads.
    scene
        .ui
        .window()
        .dispatch_event(WindowEvent::ScaleFactorChanged {
            scale_factor: scales[0].0,
        });
    scene.ctx.covers.set_scale(scales[0].0);
    scene.play(false).expect("player");
    scene.open("home").expect("home");
    scene.theme(true);
    scene.ui.show().expect("show");
    scene.settle();
    offscreen::freeze_clock();
    scene.ui.global::<Frame>().set_queue_open(true);
    for &(scale, label) in scales {
        display(&scene, 3840, 2160, scale);
        let only = std::env::var("SMOOTHNESS_ONLY").ok();
        let run = |part: &str| only.as_deref().is_none_or(|o| o == part);
        if run("scroll") {
            scrolls(&scene, label);
        }
        if run("playing") {
            paused_playing(&scene, label);
        }
        if run("region") {
            animated_region(&scene, label);
        }
        if run("transitions") {
            transitions(&scene, label);
        }
    }
}
