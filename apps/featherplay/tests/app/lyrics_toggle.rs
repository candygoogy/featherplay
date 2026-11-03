//! The lyrics toggle animates: opening, the cover glides and shrinks from the centre of the
//! stage to its spot on the left, the title and artist slide along, the stage takes the
//! lyrics' panel colour and the lines fade in; closing reverses it. About 300 ms, one eased
//! value, then nothing runs; instant with Windows' animation effects off. Frames come from the
//! renderer's own presents on a frozen clock (`offscreen::install_presenting`); no window is
//! shown.
//!
//! `toggle_frames_at_4k` (ignored) prices the frames at 4K 150 % and, with
//! `LYRICS_TOGGLE_SHOTS=<dir>`, writes `toggle-strip.png` (every fourth frame of the opening
//! and of the closing). Run it optimised:
//!
//! ```sh
//! CARGO_PROFILE_RELEASE_LTO=false CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
//!   FEATHERPLAY_HOME=<dir> RUSTFLAGS="--cfg ci" \
//!   cargo test --release -p featherplay --test app -- lyrics_toggle:: --ignored --nocapture
//! ```

use std::time::Duration;

use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{LyricsData, LyricsState, Theme};
use image::{GenericImage, RgbaImage, imageops};
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalSize, PhysicalSize};

/// One frame at 60 Hz (Slint's clock counts whole milliseconds).
const FRAME: Duration = Duration::from_millis(16);

fn shot(scene: &Scene) -> RgbaImage {
    let px = offscreen::shown(scene.ui.window()).expect("shown");
    RgbaImage::from_raw(px.width(), px.height(), px.as_bytes().to_vec()).expect("image")
}

/// One frame: the clock moves on and the window presents. How long the present took.
fn step(scene: &Scene) -> Duration {
    offscreen::advance(FRAME);
    scene.run_queued();
    offscreen::present(scene.ui.window())
        .expect("present")
        .elapsed
}

/// Frames until nothing animates (at most 200).
fn rest(scene: &Scene) -> usize {
    let w = scene.ui.window();
    let mut n = 0;
    loop {
        step(scene);
        n += 1;
        if !w.has_active_animations() || n > 200 {
            return n;
        }
    }
}

/// The toggle, frame by frame: every frame and the time each took, until the window rests.
fn toggle(scene: &Scene) -> (Vec<RgbaImage>, Vec<Duration>) {
    scene.ui.global::<LyricsData>().invoke_toggle_view();
    let w = scene.ui.window();
    let mut frames = Vec::new();
    let mut took = Vec::new();
    for _ in 0..200 {
        took.push(step(scene));
        frames.push(shot(scene));
        if !w.has_active_animations() {
            break;
        }
    }
    (frames, took)
}

/// The scene on Now playing with the cover view at rest, the synced lyrics known.
fn scene(width: u32, height: u32, scale: f32) -> Scene {
    let scene = Scene::new(1280, 800, false, true).expect("scene");
    scene.ui.global::<Theme>().set_motion(true);
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::ScaleFactorChanged {
        scale_factor: scale,
    });
    scene.ctx.covers.set_scale(scale);
    w.set_size(PhysicalSize::new(width, height));
    w.dispatch_event(WindowEvent::Resized {
        size: LogicalSize::new(width as f32 / scale, height as f32 / scale),
    });
    scene.play(false).expect("player");
    scene.open("now-playing").expect("now playing");
    scene.theme(true);
    scene.ui.show().expect("show");
    scene.settle();
    // Look the lyrics up once, so the toggle shows them at once.
    let data = scene.ui.global::<LyricsData>();
    data.invoke_toggle_view();
    scene.settle();
    data.invoke_toggle_view();
    scene.settle();
    assert_eq!(data.get_state(), LyricsState::Synced);
    offscreen::freeze_clock();
    rest(&scene);
    scene
}

/// White pixels (the line playing, `LyricInk.current`) in the full view's lyrics column at
/// 1280 x 800: right of the cover column, under the back button's row, above the bar.
fn lines_lit(img: &RgbaImage) -> usize {
    (96..640)
        .flat_map(|y| (300..860).map(move |x| (x, y)))
        .filter(|&(x, y)| img.get_pixel(x, y).0[..3] == [255, 255, 255])
        .count()
}

fn differs(a: &RgbaImage, b: &RgbaImage) -> usize {
    a.pixels().zip(b.pixels()).filter(|(p, q)| p != q).count()
}

/// One test: the platform can be installed once per process.
#[test]
fn the_lyrics_toggle_glides_both_ways_then_rests() {
    let scene = scene(1280, 800, 1.0);
    let cover_view = shot(&scene);

    // Opening: about 300 ms of frames, then nothing runs.
    let (opening, _) = toggle(&scene);
    let n = opening.len();
    assert!((15..=25).contains(&n), "the opening ran {n} frames");
    let lyrics_view = opening[n - 1].clone();
    assert!(
        lines_lit(&lyrics_view) > 150,
        "no line playing in view at the end"
    );
    // Half-way it's neither view, and the lines haven't shown yet.
    let mid = &opening[n / 2 - 1];
    assert!(
        differs(mid, &cover_view) > 10_000,
        "nothing moved by half-way"
    );
    assert!(
        differs(mid, &lyrics_view) > 10_000,
        "the lyrics view showed at once"
    );
    assert!(
        lines_lit(&opening[n / 4]) < 50,
        "the lines showed before the panel"
    );

    // Closing reverses it and lands on the cover view as it was.
    let (closing, _) = toggle(&scene);
    let n = closing.len();
    assert!((15..=25).contains(&n), "the closing ran {n} frames");
    let back = &closing[n - 1];
    let off = differs(back, &cover_view);
    assert!(off < 2_000, "the cover view came back different ({off} px)");

    // Without animation effects nothing moves: the first frame is the final layout, with only
    // the Lyrics pill's short colour fade (which stays without motion) still running.
    scene.ui.global::<Theme>().set_motion(false);
    for way in ["opening", "closing"] {
        let (frames, _) = toggle(&scene);
        let off = differs(&frames[0], &frames[frames.len() - 1]);
        assert!(off < 6_000, "{way} without motion moved ({off} px)");
    }
}

/// The frames of the toggle at 4K 150 %: the median and slowest frame, best of 3 rounds.
#[test]
#[ignore]
fn toggle_frames_at_4k() {
    let scene = scene(3840, 2160, 1.5);
    let mut medians = Vec::new();
    let mut worst = Vec::new();
    let mut strip_frames = Vec::new();
    for round in 0..3 {
        for _ in 0..2 {
            let (frames, mut took) = toggle(&scene);
            if round == 0 {
                strip_frames.extend(frames.into_iter().step_by(4));
            }
            took.sort_unstable();
            medians.push(took[took.len() / 2]);
            worst.push(took[took.len() - 1]);
            rest(&scene);
        }
    }
    let ms = |d: &Duration| d.as_secs_f64() * 1000.0;
    println!(
        "toggle at 4K 150 %: median frame {:.2} ms (best round), slowest {:.2} ms",
        medians.iter().map(ms).fold(f64::MAX, f64::min),
        worst.iter().map(ms).fold(f64::MAX, f64::min)
    );
    if let Some(dir) = std::env::var_os("LYRICS_TOGGLE_SHOTS") {
        let (cw, ch) = (640, 360);
        let gap = 8;
        let mut out = RgbaImage::from_pixel(
            strip_frames.len() as u32 * (cw + gap) - gap,
            ch,
            image::Rgba([128, 128, 128, 255]),
        );
        for (i, f) in strip_frames.iter().enumerate() {
            let small = imageops::resize(f, cw, ch, imageops::FilterType::Triangle);
            out.copy_from(&small, i as u32 * (cw + gap), 0)
                .expect("fits");
        }
        out.save(std::path::Path::new(&dir).join("toggle-strip.png"))
            .expect("png");
    }
}
