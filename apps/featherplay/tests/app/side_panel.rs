//! A measurement of the side panel (`SideTabs`, `ui/pages/now_playing.slint`) at 4K 150 %,
//! offscreen, through the app's own present path: scrolling Up next and Related (with and
//! without art, in the frame's panel and on Now playing), and each frame of the tab pill's
//! slide. Ignored; run it optimised:
//!
//! ```sh
//! CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
//!   FEATHERPLAY_HOME=<dir> RUSTFLAGS="--cfg ci" \
//!   cargo test --release -p featherplay --test app -- side_panel:: --ignored --nocapture
//! ```
//!
//! The checks are `related_scroll.rs` and `tab_pill.rs`.

use crate::support;
use crate::support::{frames, home_at, long_rows, quantile, wheel_run};
use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{Frame, NowPlayingData, QueueData, TrackRow};
use slint::{ComponentHandle, LogicalPosition, Model, ModelRc};

const W: u32 = 3840;
const H: u32 = 2160;
const SCALE: f32 = 1.5;
const WIDTH: f32 = W as f32 / SCALE;
/// Rows in a long list, the fixture rows repeated.
const LONG: usize = 300;

fn panel(scene: &Scene) -> (f32, f32) {
    support::panel(&scene.ui, WIDTH)
}

fn click_tab(scene: &Scene, i: usize) {
    support::click_tab(&scene.ui, WIDTH, i);
}

fn long(loaded: &[TrackRow]) -> ModelRc<TrackRow> {
    long_rows(loaded, LONG)
}

fn rest(scene: &Scene) {
    scene.settle();
    frames(scene.ui.window(), 60, |_| {});
}

/// Related open in the frame's panel, its list made long, settled. Returns the loaded rows.
fn related(scene: &Scene) -> Vec<TrackRow> {
    click_tab(scene, 1);
    scene.settle();
    let d = scene.ui.global::<NowPlayingData>();
    let loaded: Vec<TrackRow> = d.get_related().iter().collect();
    assert!(!loaded.is_empty(), "no related rows loaded");
    let with_art = loaded.iter().filter(|r| r.cover.size().width > 0).count();
    assert!(
        with_art * 2 >= loaded.len(),
        "only {with_art} rows have art"
    );
    d.set_related(long(&loaded));
    rest(scene);
    loaded
}

/// One wheel run down and back up over the panel: CPU per repainted frame, and the median
/// share of the window those frames copied.
fn run(scene: &Scene) -> (Vec<f64>, f64) {
    let (x0, side) = panel(scene);
    let at = LogicalPosition::new(x0 + side / 2.0, 600.0);
    let w = scene.ui.window();
    let mut cpu = Vec::new();
    let mut damage = Vec::new();
    for notches in [-2.0, 2.0] {
        for f in wheel_run(w, at, 15, notches) {
            if f.damaged > 0 {
                cpu.push(f.cpu);
                damage.push(f.damaged as f64 / (W as f64 * H as f64));
            }
        }
    }
    (cpu, quantile(&damage, 0.5))
}

/// The first run (new rows), then the cheaper of two more, per frame.
fn scroll(scene: &Scene, name: &str) {
    let (first, fd) = run(scene);
    let (a, da) = run(scene);
    let (b, _) = run(scene);
    let best: Vec<f64> = if a.len() == b.len() {
        a.iter().zip(&b).map(|(x, y)| x.min(*y)).collect()
    } else {
        a
    };
    println!(
        "{name}: first {:.2} / {:.2} ms (max {:.2}), damage {:.1} %; then {:.2} / {:.2} ms, damage {:.1} %, {} frames",
        quantile(&first, 0.5),
        quantile(&first, 0.95),
        quantile(&first, 1.0),
        fd * 100.0,
        quantile(&best, 0.5),
        quantile(&best, 0.95),
        da * 100.0,
        best.len()
    );
}

/// The frames from picking tab `to` (a click, or `NowPlayingData.tab` set as the click
/// does) until nothing runs: the first frame (the panel's contents change), then the rest.
fn pill(scene: &Scene, to: usize, click: bool) {
    let w = scene.ui.window();
    if click {
        click_tab(scene, to);
    } else {
        let d = scene.ui.global::<NowPlayingData>();
        d.set_tab(to as i32);
        d.invoke_tab_selected(to as i32);
    }
    let mut cpu = Vec::new();
    let mut damage = Vec::new();
    let mut rows = (u32::MAX, 0);
    let mut last_damage = 0;
    loop {
        let c = support::cpu::cycles();
        offscreen::advance(support::FRAME_240);
        let p = offscreen::present(w).expect("present");
        cpu.push(support::cpu::ms(support::cpu::cycles() - c));
        damage.push(p.damaged as f64 / (W as f64 * H as f64) * 100.0);
        if let Some((_, y0, _, y1)) = p.bounds {
            last_damage = cpu.len();
            if cpu.len() > 1 {
                rows = (rows.0.min(y0), rows.1.max(y1));
            }
        }
        if !w.has_active_animations() || cpu.len() > 400 {
            break;
        }
    }
    let rest = &cpu[1..];
    println!(
        "  to {to} ({}): first frame {:.2} ms, {:.1} % of the window; then {:.2} / {:.2} ms (median / max), {:.2} % median, rows {}..{}; last repaint {} ms, stops {} ms",
        if click { "click" } else { "set" },
        cpu[0],
        damage[0],
        quantile(rest, 0.5),
        quantile(rest, 1.0),
        quantile(&damage[1..], 0.5),
        rows.0,
        rows.1,
        last_damage * 4,
        cpu.len() * 4
    );
    scene.settle();
    frames(w, 30, |_| {});
}

#[test]
#[ignore = "a measurement"]
fn measure_side_panel() {
    let scene = home_at(W, H, SCALE);
    let q = scene.ui.global::<QueueData>();
    let upcoming: Vec<TrackRow> = q.get_upcoming().iter().collect();
    println!("fixture Up next: {} rows", upcoming.len());
    q.set_upcoming(long(&upcoming));
    rest(&scene);
    scroll(&scene, "panel over Home, Up next (300)");

    let rows = related(&scene);
    println!("fixture Related: {} rows", rows.len());
    scroll(&scene, "panel over Home, Related (300)");
    let d = scene.ui.global::<NowPlayingData>();
    let bare: Vec<TrackRow> = rows
        .iter()
        .cloned()
        .map(|mut r| {
            r.cover = Default::default();
            r
        })
        .collect();
    d.set_related(long(&bare));
    rest(&scene);
    scroll(&scene, "panel over Home, Related, no art");
    d.set_related(long(&rows));
    rest(&scene);

    println!("tab pill, playing:");
    for to in [0, 1, 2, 0, 1] {
        pill(&scene, to, true);
    }
    for to in [0, 1] {
        pill(&scene, to, false);
    }
    scene.sound(false).expect("paused");
    rest(&scene);
    println!("tab pill, paused:");
    for to in [0, 1, 2, 0, 1] {
        pill(&scene, to, true);
    }
    scene.sound(true).expect("playing");
    rest(&scene);

    scene.ui.global::<Frame>().set_queue_open(false);
    scene.open("now-playing").expect("now playing");
    scene.settle();
    frames(scene.ui.window(), 120, |_| {});
    d.set_related(long(&rows));
    rest(&scene);
    scroll(&scene, "Now playing, Related (300)");
}
