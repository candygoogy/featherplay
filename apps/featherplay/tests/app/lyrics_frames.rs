//! The lyrics' motion at 4K 150 %, under the user's conditions: real-length lyrics (80 lines,
//! some wrapping, `FEATHERPLAY_FIXTURE_LYRICS`), art on every cover (`support::fixture_covers`),
//! the song playing with sound (the playing marks step), the side panel on its Lyrics tab. A
//! line's glide there and in the full view, and the full view's toggle both ways, each frame
//! timed in this thread's CPU time on a frozen clock stepped 4 ms (240 Hz), through the app's
//! own present path (`offscreen::install_presenting`). The ceilings hold in optimised builds.

use crate::support::{Frame1, fixture_covers, long_lyrics, quantile, resize, step, three_tries};
use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{LyricsData, LyricsState, NowPlayingData, Theme};
use slint::ComponentHandle;
use slint::platform::WindowEvent;

const SCALE: f32 = 1.5;

/// Now playing at 4K 150 % with the sound playing, the cover view at rest, the long lyrics
/// looked up; the clock frozen.
pub(crate) fn scene() -> Scene {
    fixture_covers();
    long_lyrics();
    let scene = Scene::new(1280, 800, false, true).expect("scene");
    scene.ui.global::<Theme>().set_motion(true);
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::ScaleFactorChanged {
        scale_factor: SCALE,
    });
    scene.ctx.covers.set_scale(SCALE);
    resize(w, 3840, 2160);
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
fn run(scene: &Scene, max: usize) -> Vec<Frame1> {
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
fn glides(scene: &Scene, from: i32, n: i32) -> Vec<Vec<Frame1>> {
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

fn toggle(scene: &Scene) -> Vec<Frame1> {
    scene.ui.global::<LyricsData>().invoke_toggle_view();
    let f = run(scene, 200);
    for _ in 0..40 {
        step(scene);
    }
    f
}

/// The repainted frames' CPU time, without each run's first frame if `skip_first`.
fn cpu_of(runs: &[Vec<Frame1>], skip_first: bool) -> Vec<f64> {
    runs.iter()
        .flat_map(|r| r.iter().skip(usize::from(skip_first)))
        .filter(|f| f.damaged > 0)
        .map(|f| f.cpu)
        .collect()
}

/// One round as the user meets it: six glides in the side panel's Lyrics tab beside the cover
/// view (a glide frame that the bar's tick lands in repaints across the stage), the full view
/// opening, four glides there, and closing. The glides' runs and the toggles' runs.
fn round(scene: &Scene, from: i32) -> (Vec<Vec<Frame1>>, Vec<Vec<Frame1>>) {
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
    three_tries(|attempt| {
        let (glide_runs, toggle_runs) = round(&scene, 20 + attempt as i32 * 12);
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
        if gm > GLIDE_MS.0 || gw > GLIDE_MS.1 {
            Err(format!(
                "a glide frame costs {gm:.2} / {gw:.2} ms, over {} / {}",
                GLIDE_MS.0, GLIDE_MS.1
            ))
        } else if tm > TOGGLE_MS.0 || tw > TOGGLE_MS.1 {
            Err(format!(
                "a toggle frame costs {tm:.2} / {tw:.2} ms, over {} / {}",
                TOGGLE_MS.0, TOGGLE_MS.1
            ))
        } else if first > FIRST_MS {
            Err(format!(
                "the toggle's first frame costs {first:.2} ms, over {FIRST_MS}"
            ))
        } else {
            Ok(())
        }
    });
}
