//! The smoothness run's inventory (2026-11-01): every kind of motion in the app, timed frame by
//! frame at 4K 150 % under the user's conditions: art on every cover
//! (`support::fixture_covers`), real-length lyrics (`lyrics_frames::long_lyrics`), the song
//! playing with sound (the playing marks step, the bar ticks), Up next open on the pages that
//! have it. Each frame is one turn of the event loop on a frozen clock stepped 4 ms (240 Hz):
//! the input, the queued answers, the timers and animations, then the render and the copy
//! through the app's own present path (`offscreen::install_presenting`), in this thread's CPU
//! time.
//!
//! Each motion runs three times. The first run is reported on its own, since a first time
//! (a menu's first open, a page's first show, the first lyrics of a song) is where one-off
//! costs land; the other two keep each frame's cheapest.
//!
//! A measurement, not a check, so it's ignored. Run it optimised:
//!
//! ```sh
//! CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
//!   FEATHERPLAY_HOME=<dir> RUSTFLAGS="--cfg ci" \
//!   cargo test --release -p featherplay --test app -- animations:: --ignored --nocapture
//! ```
//!
//! `ANIMATIONS_ONLY=<substring>` runs the motions whose name contains it;
//! `ANIMATIONS_FRAMES=1` prints every frame of a first run that cost over 4 ms.

use std::rc::Rc;

use crate::support::{FRAME_240, cpu, fixture_covers, quantile};
use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{
    AccountMenu, Frame, LyricsData, NowPlayingData, SongMenu, Theme, Toast, TrackRow,
};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, LogicalSize, Model, PhysicalSize, VecModel};

const W: u32 = 3840;
const H: u32 = 2160;
const SCALE: f32 = 1.5;
/// The window in logical pixels.
const LW: f32 = W as f32 / SCALE;
const LH: f32 = H as f32 / SCALE;

/// One frame: its CPU time (ms) and the physical pixels it copied.
#[derive(Clone, Copy)]
pub(crate) struct F {
    pub(crate) cpu: f64,
    pub(crate) damaged: u64,
}

pub(crate) fn step(scene: &Scene) -> F {
    let c = cpu::cycles();
    offscreen::advance(FRAME_240);
    scene.run_queued();
    let p = offscreen::present(scene.ui.window()).expect("present");
    F {
        cpu: cpu::ms(cpu::cycles() - c),
        damaged: p.damaged,
    }
}

/// `n` frames; `first` runs at the start of the first one, before the clock moves (the
/// input), and its own CPU time comes back apart: a page's `Scene::open` parses its fixtures
/// there, which the app does on a worker thread.
fn frames(scene: &Scene, n: usize, first: &dyn Fn()) -> (Vec<F>, f64) {
    let c = cpu::cycles();
    first();
    let input = cpu::ms(cpu::cycles() - c);
    ((0..n).map(|_| step(scene)).collect(), input)
}

pub(crate) fn idle(scene: &Scene, n: usize) {
    for _ in 0..n {
        step(scene);
    }
}

pub(crate) fn point(scene: &Scene, at: LogicalPosition) {
    scene
        .ui
        .window()
        .dispatch_event(WindowEvent::PointerMoved { position: at });
}

pub(crate) fn press(scene: &Scene, at: LogicalPosition) {
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::PointerMoved { position: at });
    w.dispatch_event(WindowEvent::PointerPressed {
        position: at,
        button: PointerEventButton::Left,
    });
}

pub(crate) fn release(scene: &Scene, at: LogicalPosition) {
    scene
        .ui
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position: at,
            button: PointerEventButton::Left,
        });
}

pub(crate) fn click(scene: &Scene, at: LogicalPosition) {
    press(scene, at);
    release(scene, at);
}

/// Somewhere with nothing under it that reacts: the top bar's empty middle.
pub(crate) fn away() -> LogicalPosition {
    LogicalPosition::new(LW * 0.45, 30.0)
}

/// The bar's vertical middle.
pub(crate) fn bar_y(scene: &Scene) -> f32 {
    let t = scene.ui.global::<Theme>();
    LH - t.get_bar_inset() - t.get_bar_h() / 2.0
}

/// The bar's microphone, found by clicking along the bar on Now playing: where a click
/// turns the lyrics on (each hit turned off again).
pub(crate) fn lyrics_button(scene: &Scene) -> LogicalPosition {
    let data = scene.ui.global::<LyricsData>();
    let y = bar_y(scene);
    let mut hits = Vec::new();
    for x in (0..400).step_by(3) {
        let at = LogicalPosition::new(LW - 400.0 + x as f32, y);
        click(scene, at);
        if data.get_full() {
            hits.push(at.x);
            data.set_full(false);
        }
        point(scene, away());
        idle(scene, 2);
    }
    assert!(!hits.is_empty(), "the bar's lyrics button not found");
    idle(scene, 200);
    LogicalPosition::new((hits[0] + hits[hits.len() - 1]) / 2.0, y)
}

struct Stats {
    n: usize,
    median: f64,
    p95: f64,
    max: f64,
    first: f64,
    dmg: f64,
    dmg_max: f64,
}

fn stats(run: &[F]) -> Stats {
    let px = f64::from(W) * f64::from(H);
    let rep: Vec<&F> = run.iter().filter(|f| f.damaged > 0).collect();
    let cpu: Vec<f64> = rep.iter().map(|f| f.cpu).collect();
    let dmg: Vec<f64> = rep.iter().map(|f| f.damaged as f64 / px * 100.0).collect();
    Stats {
        n: rep.len(),
        median: quantile(&cpu, 0.5),
        p95: quantile(&cpu, 0.95),
        max: quantile(&cpu, 1.0),
        first: run.first().map_or(0.0, |f| f.cpu),
        dmg: quantile(&dmg, 0.5),
        dmg_max: quantile(&dmg, 1.0),
    }
}

fn line(name: &str, which: &str, s: &Stats) {
    println!(
        "{name:<44} {which:<5} {:>4} repainted  median {:>6.2}  p95 {:>6.2}  max {:>6.2}  first {:>6.2} ms  damage {:>5.1} % (max {:>5.1} %)",
        s.n, s.median, s.p95, s.max, s.first, s.dmg, s.dmg_max
    );
}

/// Frame by frame, the cheapest of the runs (they're the same frames).
fn cheapest(runs: &[Vec<F>]) -> Vec<F> {
    let mut out = runs[0].clone();
    for r in &runs[1..] {
        for (o, f) in out.iter_mut().zip(r) {
            if f.cpu < o.cpu {
                *o = *f;
            }
        }
    }
    out
}

/// One motion: `start` in the first frame, `n` frames, then `undo` and a rest; three times.
fn measure(scene: &Scene, name: &str, n: usize, start: &dyn Fn(), undo: &dyn Fn()) {
    if std::env::var("ANIMATIONS_ONLY").is_ok_and(|o| !name.contains(o.as_str())) {
        return;
    }
    let mut input = 0.0;
    let runs: Vec<Vec<F>> = (0..3)
        .map(|i| {
            let (r, ms) = frames(scene, n, start);
            if i == 0 {
                input = ms;
            }
            undo();
            idle(scene, 150);
            r
        })
        .collect();
    if std::env::var_os("ANIMATIONS_FRAMES").is_some() {
        for (i, f) in runs[0].iter().enumerate() {
            if f.cpu > 4.0 {
                println!("    frame {i}: {:.2} ms, {} px", f.cpu, f.damaged);
            }
        }
    }
    line(name, "first", &stats(&runs[0]));
    if input > 1.0 {
        println!("{:<44} (its input, apart: {input:.2} ms)", "");
    }
    line(name, "warm", &stats(&cheapest(&runs[1..])));
}

fn track() -> TrackRow {
    TrackRow {
        id: "menu".into(),
        title: "A song with a title".into(),
        artist: "An artist".into(),
        album: "An album".into(),
        duration: "3:21".into(),
        ..Default::default()
    }
}

pub(crate) fn scene() -> Scene {
    fixture_covers();
    // SAFETY: set before the scene starts any other thread.
    unsafe {
        std::env::set_var(
            "FEATHERPLAY_FIXTURE_LYRICS",
            crate::lyrics_frames::long_lyrics(),
        );
    }
    let scene = Scene::new(1280, 800, false, true).expect("scene");
    scene.ui.global::<Theme>().set_motion(true);
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::ScaleFactorChanged {
        scale_factor: SCALE,
    });
    scene.ctx.covers.set_scale(SCALE);
    w.set_size(PhysicalSize::new(W, H));
    w.dispatch_event(WindowEvent::Resized {
        size: LogicalSize::new(LW, LH),
    });
    scene.sound(true).expect("player");
    scene.open("home").expect("home");
    scene.theme(true);
    scene.ui.show().expect("show");
    scene.settle();
    offscreen::freeze_clock();
    scene.ui.global::<Frame>().set_queue_open(true);
    idle(&scene, 100);
    scene
}

pub(crate) fn show(scene: &Scene, page: &str) {
    scene.open(page).expect("page");
    scene.settle();
    idle(scene, 120);
}

#[test]
#[ignore = "a measurement: run it optimised, see the module docs"]
fn inventory() {
    let scene = scene();
    let ui = &scene.ui;
    let data = ui.global::<LyricsData>();
    let rest = |n| idle(&scene, n);
    let off = || {
        point(&scene, away());
    };
    println!("4K 150 %, playing, art on every cover, ms of CPU a repainted frame");

    // Pages, each first show then warm ones (the page left for Home in between).
    for page in [
        "library",
        "playlist",
        "explore",
        "settings",
        "album",
        "now-playing",
    ] {
        measure(
            &scene,
            &format!("enter {page} (from Home)"),
            100,
            &|| scene.open(page).expect("page"),
            &|| show(&scene, "home"),
        );
    }
    show(&scene, "playlist");
    measure(
        &scene,
        "enter Home (from a playlist)",
        100,
        &|| scene.open("home").expect("page"),
        &|| show(&scene, "playlist"),
    );

    // On Home: the bar, menus, the toast, Up next.
    show(&scene, "home");
    let y = bar_y(&scene);
    let frame = ui.global::<Frame>();
    measure(
        &scene,
        "Up next closes",
        100,
        &|| frame.set_queue_open(false),
        &|| frame.set_queue_open(true),
    );
    frame.set_queue_open(false);
    rest(150);
    measure(
        &scene,
        "Up next opens",
        100,
        &|| frame.set_queue_open(true),
        &|| frame.set_queue_open(false),
    );
    frame.set_queue_open(true);
    rest(150);
    let menu = ui.global::<SongMenu>();
    measure(
        &scene,
        "song menu opens",
        80,
        &|| menu.invoke_show(track(), LW * 0.5, LH * 0.4, LH * 0.4 - 40.0),
        &|| menu.invoke_close(),
    );
    menu.invoke_show(track(), LW * 0.5, LH * 0.4, LH * 0.4 - 40.0);
    rest(150);
    measure(
        &scene,
        "song menu closes",
        80,
        &|| menu.invoke_close(),
        &|| menu.invoke_show(track(), LW * 0.5, LH * 0.4, LH * 0.4 - 40.0),
    );
    menu.invoke_close();
    rest(150);
    let account = ui.global::<AccountMenu>();
    measure(
        &scene,
        "account menu opens",
        80,
        &|| account.invoke_show(20.0, LH - 200.0),
        &|| account.invoke_close(),
    );
    let toast = ui.global::<Toast>();
    measure(
        &scene,
        "toast appears",
        80,
        &|| {
            toast.set_text("Added to queue: A song with a title".into());
            toast.set_open(true);
        },
        &|| toast.set_open(false),
    );
    toast.set_open(true);
    rest(150);
    measure(
        &scene,
        "toast leaves",
        80,
        &|| toast.set_open(false),
        &|| toast.set_open(true),
    );
    toast.set_open(false);
    rest(150);
    measure(&scene, "playing, nothing else (1 s)", 250, &|| {}, &|| {});
    let card = LogicalPosition::new(LW * 0.3, 700.0);
    measure(
        &scene,
        "hover onto a Home card",
        60,
        &|| point(&scene, card),
        &off,
    );
    let seek = LogicalPosition::new(LW / 2.0, y + 26.0);
    measure(
        &scene,
        "hover onto the seek line",
        60,
        &|| point(&scene, seek),
        &off,
    );

    // The bar's microphone, found on Now playing.
    show(&scene, "now-playing");
    let mic = lyrics_button(&scene);
    println!("the bar's lyrics button at {:.0}, {:.0}", mic.x, mic.y);
    let heart = LogicalPosition::new(mic.x + 36.0 + 4.0 + 32.0 + 4.0 + 18.0, mic.y);
    measure(
        &scene,
        "lyrics button: hover on (tooltip at 500 ms)",
        200,
        &|| point(&scene, mic),
        &off,
    );
    measure(&scene, "lyrics button: hover off", 60, &off, &|| {
        point(&scene, mic);
        rest(150);
    });
    measure(
        &scene,
        "like button: hover on",
        60,
        &|| point(&scene, heart),
        &off,
    );
    measure(
        &scene,
        "lyrics button: press",
        60,
        &|| press(&scene, mic),
        &|| {
            point(&scene, away());
            release(&scene, away());
        },
    );
    point(&scene, mic);
    rest(150);
    measure(
        &scene,
        "lyrics button: click opens the lyrics",
        120,
        &|| click(&scene, mic),
        &|| {
            data.set_full(false);
            rest(150);
            point(&scene, mic);
        },
    );
    measure(
        &scene,
        "lyrics button: click closes the lyrics",
        120,
        &|| click(&scene, mic),
        &|| {
            data.set_full(true);
            rest(150);
            point(&scene, mic);
        },
    );
    off();
    rest(100);
    let np = ui.global::<NowPlayingData>();
    measure(
        &scene,
        "line change, no lyrics on screen",
        120,
        &|| data.set_current(data.get_current() + 1),
        &|| {},
    );
    // A new song's lines (the same text in a new model), handed over as `lyrics::fill` does:
    // four at once, then four more every 4 ms.
    let feed = slint::Timer::default();
    let new_lines = || {
        let mut lines = data
            .get_lines()
            .iter()
            .collect::<Vec<slint::SharedString>>()
            .into_iter();
        let model = Rc::new(VecModel::from(lines.by_ref().take(4).collect::<Vec<_>>()));
        data.set_lines(model.clone().into());
        let rest = std::cell::RefCell::new(lines);
        feed.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(4),
            move || {
                for line in rest.borrow_mut().by_ref().take(4) {
                    model.push(line);
                }
            },
        );
    };
    measure(
        &scene,
        "new lines arrive, cover view",
        60,
        &new_lines,
        &|| {},
    );
    np.set_tab(2);
    np.invoke_tab_selected(2);
    rest(150);
    measure(
        &scene,
        "line glide, side tab",
        120,
        &|| data.set_current(data.get_current() + 1),
        &|| {},
    );
    np.set_tab(0);
    np.invoke_tab_selected(0);
    data.set_full(true);
    rest(150);
    measure(
        &scene,
        "line glide, full view",
        120,
        &|| data.set_current(data.get_current() + 1),
        &|| {},
    );
    measure(
        &scene,
        "new lines arrive, full view",
        60,
        &new_lines,
        &|| {},
    );
    feed.stop();
    data.set_full(false);
    rest(150);
    let tabs_y = crate::support::TABS_Y + crate::support::TABS_H / 2.0;
    let (x0, side) = crate::support::panel(ui, LW);
    let s5 = ui.global::<Theme>().get_s5();
    let seg = (side - 2.0 * s5) / 3.0;
    let tab = |i: f32| LogicalPosition::new(x0 + s5 + seg * (i + 0.5), tabs_y);
    measure(
        &scene,
        "tab pill: Up next to Related",
        120,
        &|| click(&scene, tab(1.0)),
        &|| {
            click(&scene, tab(0.0));
            rest(150);
        },
    );
    measure(
        &scene,
        "tab pill: Up next to Lyrics",
        120,
        &|| click(&scene, tab(2.0)),
        &|| {
            click(&scene, tab(0.0));
            rest(150);
        },
    );

    // From another page, the microphone opens Now playing on the lyrics.
    show(&scene, "home");
    measure(
        &scene,
        "lyrics button on Home: Now playing on the lyrics",
        120,
        &|| click(&scene, mic),
        &|| {
            data.set_full(false);
            show(&scene, "home");
        },
    );
}
