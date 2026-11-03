//! The lyrics' motion at 4K 150 %, under the user's conditions: real-length lyrics (80 lines,
//! some wrapping, `FEATHERPLAY_FIXTURE_LYRICS`), art on every cover (`support::fixture_covers`),
//! the song playing with sound (the playing marks step), the side panel on its Lyrics tab. A
//! line's glide there and in the full view, and the full view's toggle both ways, each frame
//! timed in this thread's CPU time on a frozen clock stepped 4 ms (240 Hz), through the app's
//! own present path (`offscreen::install_presenting`). The ceilings hold in optimised builds.
//!
//! `bench` (ignored) prints the same frames. Run it optimised:
//!
//! ```sh
//! CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
//!   FEATHERPLAY_HOME=<dir> RUSTFLAGS="--cfg ci" \
//!   cargo test --release -p featherplay --test app -- lyrics_frames:: --ignored --nocapture
//! ```

use std::path::PathBuf;

use crate::support::{FRAME_240, cpu, fixture_covers, quantile};
use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{LyricsData, LyricsState, NowPlayingData, Theme};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalSize, Model, PhysicalSize};

const W: u32 = 3840;
const H: u32 = 2160;
const SCALE: f32 = 1.5;

/// One frame's CPU time (ms), the physical pixels it copied and their bounding box.
pub(crate) struct Frame {
    pub(crate) cpu: f64,
    pub(crate) damaged: u64,
    bounds: Option<(u32, u32, u32, u32)>,
}

thread_local! {
    /// Self-paced (`LYRICS_FRAMES_PACED`): the clock moves on by what the last frame cost (at
    /// least 4 ms), as the window's event loop does when a frame takes longer than a refresh.
    static LAST: std::cell::Cell<f64> = const { std::cell::Cell::new(4.0) };
}

pub(crate) fn step(scene: &Scene) -> Frame {
    let by = if std::env::var_os("LYRICS_FRAMES_PACED").is_some() {
        std::time::Duration::from_millis(LAST.get().ceil().max(4.0) as u64)
    } else {
        FRAME_240
    };
    let c = cpu::cycles();
    offscreen::advance(by);
    scene.run_queued();
    let p = offscreen::present(scene.ui.window()).expect("present");
    let cpu = cpu::ms(cpu::cycles() - c);
    LAST.set(if p.damaged > 0 { cpu } else { 4.0 });
    Frame {
        cpu,
        damaged: p.damaged,
        bounds: p.bounds,
    }
}

/// An LRCLIB record as long as a real song's: 80 timed lines, a verse break every eight,
/// about one in four long enough to wrap in the full view. All of them start before the
/// fixture's position (2:41), so the app arms no line timer: the test moves `current` itself,
/// as `lyrics::sync` does.
pub(crate) fn long_lyrics() -> PathBuf {
    const WORDS: [&str; 24] = [
        "light", "harbour", "morning", "never", "walking", "through", "the", "city", "again",
        "under", "neon", "rain", "and", "your", "voice", "is", "calling", "me", "back", "home",
        "tonight", "forever", "slowly", "burning",
    ];
    let mut seed: u32 = 0x2545_F491;
    let mut next = |n: u32| {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed % n
    };
    let mut synced = String::new();
    let mut plain = String::new();
    for i in 0..80u32 {
        let t = 10.0 + f64::from(i) * 1.8;
        let text = if i % 8 == 7 {
            String::new()
        } else {
            let words = if next(4) == 0 {
                12 + next(4)
            } else {
                4 + next(4)
            };
            let line: Vec<&str> = (0..words).map(|_| WORDS[next(24) as usize]).collect();
            let line = line.join(" ");
            line[..1].to_uppercase() + &line[1..]
        };
        synced.push_str(&format!(
            "[{:02}:{:05.2}] {text}\n",
            (t / 60.0) as u32,
            t % 60.0
        ));
        plain.push_str(&text);
        plain.push('\n');
    }
    let record = serde_json::json!({
        "id": 1, "name": "Long", "trackName": "Long", "artistName": "Fixture",
        "albumName": "Fixture", "duration": 367.0, "instrumental": false,
        "plainLyrics": plain, "syncedLyrics": synced,
    });
    let file = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("lyrics-long.json");
    std::fs::write(&file, record.to_string()).expect("lyrics file");
    file
}

/// Now playing at 4K 150 % with the sound playing, the cover view at rest, the long lyrics
/// looked up; the clock frozen.
pub(crate) fn scene() -> Scene {
    fixture_covers();
    if std::env::var_os("LYRICS_FRAMES_SHORT").is_none() {
        // SAFETY: set before the scene starts any other thread.
        unsafe { std::env::set_var("FEATHERPLAY_FIXTURE_LYRICS", long_lyrics()) };
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
        size: LogicalSize::new(W as f32 / SCALE, H as f32 / SCALE),
    });
    scene.sound(true).expect("player");
    scene.open("now-playing").expect("now playing");
    scene.theme(true);
    scene.ui.show().expect("show");
    scene.settle();
    let data = scene.ui.global::<LyricsData>();
    data.invoke_toggle_view();
    scene.settle();
    data.invoke_toggle_view();
    scene.settle();
    assert_eq!(data.get_state(), LyricsState::Synced);
    offscreen::freeze_clock();
    for _ in 0..200 {
        step(&scene);
    }
    scene
}

/// Frames until nothing animates (at most `max`), from a change made just before.
fn run(scene: &Scene, max: usize) -> Vec<Frame> {
    let w = scene.ui.window();
    let mut out = Vec::new();
    for _ in 0..max {
        out.push(step(scene));
        if !w.has_active_animations() {
            break;
        }
    }
    out
}

/// `n` line changes from line `from`, one line on each, every frame until it rests.
fn glides(scene: &Scene, from: i32, n: i32) -> Vec<Vec<Frame>> {
    let data = scene.ui.global::<LyricsData>();
    (0..n)
        .map(|k| {
            data.set_current(from + k);
            let f = run(scene, 200);
            // Settle the playing marks' and the hover's tails.
            for _ in 0..20 {
                step(scene);
            }
            f
        })
        .collect()
}

fn toggle(scene: &Scene) -> Vec<Frame> {
    scene.ui.global::<LyricsData>().invoke_toggle_view();
    let f = run(scene, 200);
    for _ in 0..40 {
        step(scene);
    }
    f
}

fn summary(label: &str, runs: &[Vec<Frame>]) {
    let px = f64::from(W) * f64::from(H);
    let cpu: Vec<f64> = runs
        .iter()
        .flatten()
        .filter(|f| f.damaged > 0)
        .map(|f| f.cpu)
        .collect();
    let dmg: Vec<f64> = runs
        .iter()
        .flatten()
        .filter(|f| f.damaged > 0)
        .map(|f| f.damaged as f64 / px * 100.0)
        .collect();
    let firsts: Vec<f64> = runs
        .iter()
        .filter_map(|r| r.first())
        .map(|f| f.cpu)
        .collect();
    let rest: Vec<f64> = runs
        .iter()
        .flat_map(|r| r.iter().skip(1))
        .filter(|f| f.damaged > 0)
        .map(|f| f.cpu)
        .collect();
    println!(
        "{label:<28} median {:6.2}  p95 {:6.2}  max {:6.2}  | after the first: median {:6.2} max {:6.2} | first {:6.2} | damage {:4.1} %  ({} frames)",
        quantile(&cpu, 0.5),
        quantile(&cpu, 0.95),
        quantile(&cpu, 1.0),
        quantile(&rest, 0.5),
        quantile(&rest, 1.0),
        quantile(&firsts, 0.5),
        quantile(&dmg, 0.5),
        cpu.len()
    );
    if std::env::var_os("LYRICS_FRAMES_BOUNDS").is_some() {
        for f in runs.iter().flatten() {
            println!("    {:6.2} ms {:?} {}", f.cpu, f.bounds, f.damaged);
        }
    }
}

/// The repainted frames' CPU time, without each run's first frame if `skip_first`.
fn cpu_of(runs: &[Vec<Frame>], skip_first: bool) -> Vec<f64> {
    runs.iter()
        .flat_map(|r| r.iter().skip(usize::from(skip_first)))
        .filter(|f| f.damaged > 0)
        .map(|f| f.cpu)
        .collect()
}

/// One round as the user meets it: six glides in the side panel's Lyrics tab beside the cover
/// view (a glide frame that the bar's tick lands in repaints across the stage), the full view
/// opening, four glides there, and closing. The glides' runs and the toggles' runs.
fn round(scene: &Scene, from: i32) -> (Vec<Vec<Frame>>, Vec<Vec<Frame>>) {
    let mut glide = glides(scene, from, 6);
    let open = toggle(scene);
    glide.extend(glides(scene, from + 6, 4));
    let close = toggle(scene);
    (glide, vec![open, close])
}

/// The scene with the side panel on its Lyrics tab, as the user had it.
fn side_tab(scene: &Scene) {
    let np = scene.ui.global::<NowPlayingData>();
    np.set_tab(2);
    np.invoke_tab_selected(2);
    for _ in 0..100 {
        step(scene);
    }
}

/// What a frame may cost, optimised, ms of CPU (median, worst): a line's glide, and the
/// toggle after its first frame. The full view is made beforehand and kept (2026-11-01), so
/// the toggle's first frame makes no lines; it was 14–22 ms when it made all 80.
const GLIDE_MS: (f64, f64) = (6.0, 10.0);
const TOGGLE_MS: (f64, f64) = (8.0, 12.0);
const FIRST_MS: f64 = 16.0;

/// A line's glide and the lyrics toggle at 4K 150 % fit 120 Hz at the median and never drop
/// below ~80 fps, apart from the toggle's first frame. Before (main `7ff041e`): the toggle's
/// frames over the see-through wash cost ~45 ms each, and a side-tab glide frame that the bar's
/// tick merged into the stage ~19 ms.
#[test]
fn lyrics_motion_fits_its_4k_frame_budget() {
    let scene = scene();
    side_tab(&scene);
    // CPU time per frame is noisy when other builds load the machine: up to 3 tries, and
    // the test fails only if every one is over.
    let mut last = String::new();
    for attempt in 0..3 {
        let (glide_runs, toggle_runs) = round(&scene, 20 + attempt * 12);
        let glide = cpu_of(&glide_runs, false);
        let toggled = cpu_of(&toggle_runs, true);
        let first = toggle_runs
            .iter()
            .filter_map(|r| r.first())
            .map(|f| f.cpu)
            .fold(0.0, f64::max);
        let (gm, gw) = (quantile(&glide, 0.5), quantile(&glide, 1.0));
        let (tm, tw) = (quantile(&toggled, 0.5), quantile(&toggled, 1.0));
        println!(
            "4K 150 %, ms of CPU a frame (median / worst): glide {gm:.2} / {gw:.2} ({} frames), toggle {tm:.2} / {tw:.2} ({} frames), its first frame {first:.2}",
            glide.len(),
            toggled.len()
        );
        assert!(
            glide.len() > 500 && toggled.len() > 100,
            "too few frames repainted"
        );
        last = if cfg!(debug_assertions) {
            return;
        } else if gm > GLIDE_MS.0 || gw > GLIDE_MS.1 {
            format!(
                "a glide frame costs {gm:.2} / {gw:.2} ms, over {} / {}",
                GLIDE_MS.0, GLIDE_MS.1
            )
        } else if tm > TOGGLE_MS.0 || tw > TOGGLE_MS.1 {
            format!(
                "a toggle frame costs {tm:.2} / {tw:.2} ms, over {} / {}",
                TOGGLE_MS.0, TOGGLE_MS.1
            )
        } else if first > FIRST_MS {
            format!("the toggle's first frame costs {first:.2} ms, over {FIRST_MS}")
        } else {
            return;
        };
    }
    panic!("{last}");
}

/// Prints the frames of the glides and the toggle, as the test measures them (with
/// `LYRICS_FRAMES_BOUNDS`, each frame and what it repainted; with `LYRICS_FRAMES_PACED`, the
/// clock moving on by each frame's cost).
#[test]
#[ignore]
fn bench() {
    let scene = scene();
    let np = scene.ui.global::<NowPlayingData>();
    println!(
        "{} lines; playing marks on; 4K 150 %; cover {:?}; wash dark {:?}",
        scene.ui.global::<LyricsData>().get_lines().row_count(),
        np.get_cover().size(),
        np.get_wash_dark()
    );
    side_tab(&scene);
    let side = glides(&scene, 30, 6);
    summary("glide, side tab", &side);
    let open = toggle(&scene);
    summary("toggle open", &[open]);
    let full = glides(&scene, 36, 6);
    summary("glide, full view", &full);
    let close = toggle(&scene);
    summary("toggle close", &[close]);
}
