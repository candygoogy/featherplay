//! The lyrics' line change, frame by frame: Now playing's full lyrics
//! view, the line playing moves on one, and the frames are taken from the renderer's own
//! presents on a frozen clock (`offscreen::install_presenting`); no window is shown. Also
//! checks what the change costs: it animates only while it runs, then stops.
//!
//! Ignored; run with `LYRICS_SHOTS=<dir>` to write `strip.png` (six frames 80 ms apart, the
//! lyrics column cropped) and each frame.

use std::path::Path;
use std::time::Duration;

use featherplay::lyrics::{self, SnapshotLyrics};
use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{LyricsData, Theme};
use image::{GenericImage, RgbaImage, imageops};
use slint::ComponentHandle;
use slint::platform::WindowEvent;

/// One frame at 60 Hz (Slint's clock counts whole milliseconds).
const FRAME: Duration = Duration::from_millis(16);
/// Frames between two shots: 80 ms.
const EVERY: usize = 5;
const SHOTS: usize = 6;

fn shot(scene: &Scene) -> RgbaImage {
    let w = scene.ui.window();
    let px = offscreen::shown(w).expect("shown");
    RgbaImage::from_raw(px.width(), px.height(), px.as_bytes().to_vec()).expect("image")
}

/// Runs frames until nothing animates; how many it took.
fn rest(scene: &Scene) -> usize {
    let w = scene.ui.window();
    let mut n = 0;
    loop {
        offscreen::advance(FRAME);
        offscreen::present(w).expect("present");
        n += 1;
        if !w.has_active_animations() || n > 200 {
            return n;
        }
    }
}

/// The line change: the frames from the change on, how many frames animated, and the median
/// time a frame took to render and present.
fn change(scene: &Scene) -> (Vec<RgbaImage>, usize, Duration) {
    let data = scene.ui.global::<LyricsData>();
    let at = data.get_current();
    // From the fixture's line, at rest, to the next; the first shot is the frame before.
    rest(scene);
    let mut frames = vec![shot(scene)];
    data.set_current(at + 1);
    let w = scene.ui.window();
    let mut animated = 0;
    let mut damage = vec![];
    let mut took = vec![];
    for i in 1..=(EVERY * (SHOTS - 1)) {
        offscreen::advance(FRAME);
        let p = offscreen::present(w).expect("present");
        damage.push(p.damaged);
        took.push(p.elapsed);
        if w.has_active_animations() {
            animated += 1;
        }
        if i % EVERY == 0 {
            frames.push(shot(scene));
        }
    }
    animated += rest(scene);
    damage.sort_unstable();
    let size = w.size();
    let window = f64::from(size.width) * f64::from(size.height);
    println!(
        "repainted per frame, median {:.0} % of the window, max {:.0} %",
        damage[damage.len() / 2] as f64 / window * 100.0,
        damage[damage.len() - 1] as f64 / window * 100.0
    );
    // Back where it was, for the next round.
    data.set_current(at);
    rest(scene);
    took.sort_unstable();
    (frames, animated, took[took.len() / 2])
}

/// The lyrics column of each frame, side by side at half size, with a gap between.
fn strip(frames: &[RgbaImage], dir: &Path, name: &str) {
    // 1280 x 800: the full view's lyrics column (now_playing.slint: x from 268, under the
    // back button's row, down to the bar's top).
    let (x, y, w, h) = (268, 72, 600, 560);
    let gap = 8;
    let (cw, ch) = (w / 2, h / 2);
    let mut out = RgbaImage::from_pixel(
        frames.len() as u32 * (cw + gap) - gap,
        ch,
        image::Rgba([128, 128, 128, 255]),
    );
    for (i, f) in frames.iter().enumerate() {
        let crop = imageops::crop_imm(f, x, y, w, h).to_image();
        let small = imageops::resize(&crop, cw, ch, imageops::FilterType::Lanczos3);
        out.copy_from(&small, i as u32 * (cw + gap), 0)
            .expect("fits");
        f.save(dir.join(format!("{name}-frame{i}.png")))
            .expect("png");
    }
    out.save(dir.join(format!("{name}.png"))).expect("png");
}

#[test]
#[ignore]
fn line_change_frames() {
    let scene = Scene::new(1280, 800, false, true).expect("scene");
    scene.ui.global::<Theme>().set_motion(true);
    scene.play(false).expect("player");
    scene.open("now-playing").expect("now playing");
    scene.theme(true);
    scene.ui.global::<LyricsData>().set_full(true);
    lyrics::show_for_snapshot(&scene.ctx, SnapshotLyrics::Synced);
    scene.ui.show().expect("show");
    scene.settle();
    offscreen::freeze_clock();
    rest(&scene);
    assert!(
        scene.ui.global::<LyricsData>().get_current() > 0,
        "no line playing at the fixture's position"
    );

    let (frames, animated, _) = change(&scene);
    println!("animated for {animated} frames of 16 ms");
    // The glide and the colours run 400 ms (25 frames), then everything rests.
    assert!(
        (20..=30).contains(&animated),
        "animated for {animated} frames"
    );
    if let Some(dir) = std::env::var_os("LYRICS_SHOTS") {
        strip(&frames, Path::new(&dir), "strip");
    }

    // The same change at 4K (200 %): what a frame of it costs, best of 3 rounds.
    let w = scene.ui.window();
    w.set_size(slint::PhysicalSize::new(3840, 2160));
    w.dispatch_event(WindowEvent::ScaleFactorChanged { scale_factor: 2.0 });
    scene.ctx.covers.set_scale(2.0);
    scene.settle();
    rest(&scene);
    let best = (0..3).map(|_| change(&scene).2).min().expect("rounds");
    println!("at 4K: median frame {:.2} ms", best.as_secs_f64() * 1000.0);
}
