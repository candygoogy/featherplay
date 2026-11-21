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

/// A grey cover and a blue one, as the fixtures' art: a track whose art is one or the other.
fn grey_and_blue() {
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("field-swap-covers");
    std::fs::create_dir_all(&dir).expect("covers dir");
    for (name, [r, g, b]) in [("grey", [0x80, 0x80, 0x84]), ("blue", [0x2A, 0x3E, 0xA8])] {
        let img = image::RgbImage::from_fn(440, 440, |x, y| {
            // A soft light in the middle, so the field isn't one flat colour.
            let d = ((x as f32 - 220.0).powi(2) + (y as f32 - 300.0).powi(2)).sqrt() / 440.0;
            let k = (1.3 - d).clamp(0.6, 1.3);
            image::Rgb([r, g, b].map(|v: u8| (f32::from(v) * k).min(255.0) as u8))
        });
        img.save(dir.join(format!("{name}.png"))).expect("cover");
    }
    // SAFETY: set before the scene starts any other thread.
    unsafe { std::env::set_var("FEATHERPLAY_FIXTURE_COVERS", dir) };
}

/// The radio's tracks with a grey cover and with a blue one (the neutral accent or not).
fn grey_then_blue(scene: &Scene) -> (usize, usize) {
    let (mut grey, mut blue) = (None, None);
    for t in 0..12 {
        scene.play_track(false, t).expect("track");
        support::settle_in_frames(scene);
        idle(scene, 120);
        let neutral = scene
            .ui
            .global::<featherplay::ui::Theme>()
            .get_follow_neutral();
        if neutral {
            grey.get_or_insert(t);
        } else {
            blue.get_or_insert(t);
        }
        if let (Some(g), Some(b)) = (grey, blue) {
            return (g, b);
        }
    }
    panic!("no grey and blue covers among the radio's tracks");
}

/// The cover's left edge on Now playing, physical px at scale 1: the cover's own pixels there
/// and its shadow and glow beside it, above the title (`title`, the title's place).
fn edge(shown: &RgbaImage, title: slint::LogicalPosition) -> RgbaImage {
    let x = (title.x - 70.0).max(0.0) as u32;
    let y = (title.y - 330.0).max(0.0) as u32;
    image::imageops::crop_imm(shown, x, y, 110, 320).to_image()
}

/// A new song on Now playing, a grey cover to a blue one, in the cover view and the lyrics
/// view: every frame shows around the cover's edge either all of the last song (its cover,
/// shadow, glow and field) or all of the new one, never a part of one with the rest of the
/// other, nor anything between (a fade of the glow, the card's tint or the art).
#[test]
fn a_new_song_lands_around_its_cover_in_one_frame() {
    grey_and_blue();
    let scene = scene_at(W, H, 1.0, "now-playing", true);
    let (grey, blue) = grey_then_blue(&scene);
    for full in [false, true] {
        if full {
            scene
                .ui
                .global::<featherplay::ui::LyricsData>()
                .invoke_toggle_view();
            support::settle_in_frames(&scene);
            idle(&scene, 150);
        }
        scene.play_track(false, grey).expect("grey");
        support::settle_in_frames(&scene);
        idle(&scene, 150);
        let title = scene
            .ui
            .global::<featherplay::ui::Player>()
            .get_title()
            .to_string();
        let at = support::label::rects(&scene, &title)
            .into_iter()
            .map(|r| slint::LogicalPosition::new(r.origin.x, r.origin.y))
            .fold(None::<slint::LogicalPosition>, |a, p| {
                Some(a.map_or(p, |a| if p.y > a.y { p } else { a }))
            })
            .expect("the title");
        let old = edge(&support::frame(&scene), at);
        scene.play_track(false, blue).expect("blue");
        let mut seen = Vec::new();
        for _ in 0..90 {
            std::thread::sleep(std::time::Duration::from_millis(4));
            step(&scene);
            seen.push(edge(&support::frame(&scene), at));
        }
        support::settle_in_frames(&scene);
        idle(&scene, 150);
        let new = edge(&support::frame(&scene), at);
        assert!(apart(&old, &new).0 > 4.0, "the covers look alike");
        // With `$FEATHERPLAY_FRAME_STRIPS`, the frames side by side.
        if let Some(dir) = std::env::var_os("FEATHERPLAY_FRAME_STRIPS") {
            std::fs::create_dir_all(&dir).expect("folder");
            let (w, h) = (old.width(), old.height());
            let mut strip = RgbaImage::new(w * (seen.len() as u32 + 1), h);
            image::imageops::replace(&mut strip, &old, 0, 0);
            for (i, f) in seen.iter().enumerate() {
                image::imageops::replace(&mut strip, f, i64::from(w) * (i as i64 + 1), 0);
            }
            let name = if full {
                "song-edge-lyrics.png"
            } else {
                "song-edge.png"
            };
            strip
                .save(std::path::Path::new(&dir).join(name))
                .expect("png");
        }
        for (i, f) in seen.iter().enumerate() {
            let (o, n) = (apart(f, &old), apart(f, &new));
            assert!(
                o.1 <= 2 || n.1 <= 2,
                "full={full}: frame {i} is {:.1} / {} from the last song and {:.1} / {} from the new one",
                o.0,
                o.1,
                n.0,
                n.1
            );
        }
    }
}
