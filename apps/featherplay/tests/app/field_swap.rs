//! Now playing's ground never changes from the bands to the cover's field in front of the
//! user: entering the page, a new song while it shows, a theme change and a resize each go
//! straight to the field (or from one field to the next). The frames come 4 ms of real time
//! apart, as at 240 Hz, so the work on other threads (the cover's decode, the field, its
//! bank) lands in them as it would on screen (a debug build's being slower). On the
//! offscreen platform at 1600 x 1000, art on every cover, a frozen clock; no window is shown.
//!
//! `frame_strips` (ignored) writes each motion's frames, a slice of the ground each, to
//! `$FEATHERPLAY_FRAME_STRIPS`, with how far each frame's ground is from the last's.

use featherplay::snapshot::Scene;
use featherplay::ui::NowPlayingData;
use image::RgbaImage;
use slint::ComponentHandle;

use crate::support::{self, idle, resize, scene_at, step};

const W: u32 = 1600;
const H: u32 = 1000;

/// The slice of the ground a frame shows: the stage's left edge, clear of the header row, the
/// cover's block and the bar.
fn ground(scene: &Scene) -> RgbaImage {
    let shown = support::frame(scene);
    image::imageops::crop_imm(&shown, 0, 120, 48, 640).to_image()
}

/// How far two slices are apart: the mean and the largest difference of a channel, levels.
fn apart(a: &RgbaImage, b: &RgbaImage) -> (f64, u8) {
    let (mut sum, mut most, mut n) = (0u64, 0u8, 0u64);
    for (p, q) in a.pixels().zip(b.pixels()) {
        for k in 0..3 {
            let d = p[k].abs_diff(q[k]);
            sum += u64::from(d);
            most = most.max(d);
            n += 1;
        }
    }
    (sum as f64 / n as f64, most)
}

/// The frames after `act`, 4 ms of real time apart: the ground's slice of each, and whether
/// the page had its picture then.
fn record(scene: &Scene, frames: usize, act: &dyn Fn()) -> Vec<(RgbaImage, bool)> {
    act();
    (0..frames)
        .map(|_| {
            std::thread::sleep(std::time::Duration::from_millis(4));
            step(scene);
            let pictured = scene
                .ui
                .global::<NowPlayingData>()
                .get_backdrop()
                .size()
                .width
                > 0;
            (ground(scene), pictured)
        })
        .collect()
}

/// Home, playing, the clock frozen.
fn scene() -> Scene {
    let scene = scene_at(W, H, 1.0, "home", true);
    idle(&scene, 30);
    scene
}

/// The motions, in order: entering Now playing, a new song on it, the light theme, a resize.
fn motions(scene: &Scene) -> Vec<(&'static str, Vec<(RgbaImage, bool)>)> {
    let mut out = Vec::new();
    out.push((
        "enter",
        record(scene, 70, &|| {
            scene.open("now-playing").expect("now playing")
        }),
    ));
    out.push((
        "song",
        record(scene, 70, &|| scene.play_track(false, 3).expect("track")),
    ));
    out.push(("theme", record(scene, 70, &|| scene.theme(false))));
    out.push((
        "resize",
        record(scene, 70, &|| resize(scene.ui.window(), 1500, 940)),
    ));
    out
}

#[test]
#[ignore]
#[allow(clippy::print_stdout)]
fn frame_strips() {
    let Some(dir) = std::env::var_os("FEATHERPLAY_FRAME_STRIPS") else {
        return;
    };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).expect("folder");
    let scene = scene();
    let mut before: Option<RgbaImage> = None;
    for (name, frames) in motions(&scene) {
        let last = &frames.last().expect("frames").0;
        if let Some(b) = before.replace(last.clone()) {
            let (mean, most) = apart(&b, last);
            println!("{name}: the ground moved {mean:.1}/{most} from the last motion's");
        }
        let (w, h) = (last.width(), last.height());
        let mut strip = RgbaImage::new(w * frames.len() as u32, h);
        let mut line = Vec::new();
        for (i, (slice, pictured)) in frames.iter().enumerate() {
            image::imageops::replace(&mut strip, slice, i64::from(w) * i as i64, 0);
            let (mean, most) = apart(slice, last);
            line.push(format!(
                "{i}:{}{mean:.1}/{most}",
                if *pictured { "" } else { "bands " }
            ));
        }
        strip.save(dir.join(format!("{name}.png"))).expect("png");
        println!("{name}: {}", line.join(" "));
    }
}

/// No frame of Now playing shows the bands where its field belongs: entering the page and a
/// theme change show the field from their first frame (each frame within the dither of the
/// last), a new song changes it at most once, from one field to the next, and a resize shows
/// the last one stretched, within a few levels of the new one.
#[test]
fn now_playing_never_swaps_the_bands_for_its_field() {
    let scene = scene();
    for (name, frames) in motions(&scene) {
        let last = &frames.last().expect("frames").0;
        let mut changes = 0;
        for (i, (slice, pictured)) in frames.iter().enumerate() {
            assert!(*pictured, "{name}: frame {i} has no picture");
            let (mean, most) = apart(slice, last);
            match name {
                "enter" | "theme" => assert!(
                    most <= 2 && mean < 0.5,
                    "{name}: frame {i} is {mean:.1} / {most} levels from the last"
                ),
                "resize" => assert!(
                    most <= 3,
                    "{name}: frame {i} is {mean:.1} / {most} levels from the last"
                ),
                _ => {}
            }
            if i > 0 && apart(slice, &frames[i - 1].0).1 > 2 {
                changes += 1;
            }
        }
        assert!(changes <= 1, "{name}: the ground changed {changes} times");
    }
}
