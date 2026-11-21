//! Home's band on the song's cover field (`ui/pages/home/hero.slint`, `BandField`): while a
//! song plays it shows Now playing's bank of rows, its middle under the band, dimmed 20 %
//! toward black in the dark theme; a fresh start's featured pick keeps its flat colour. A
//! theme change and the window back from minimised go straight from one field to the next,
//! never through the flat colour. On the offscreen platform at 1600 x 1000, the clock frozen,
//! the frames 4 ms of real time apart (as `field_swap.rs`), so the bank made on another
//! thread lands as it would on screen.
//!
//! `what_the_band_on_the_field_costs_*` (ignored) measure a frame of Home with the band in
//! view, on the field and on its flat colour: `cargo nextest run --release -p featherplay
//! --test app what_the_band --run-ignored only --no-capture`.

use featherplay::follow::backdrop::{Layout, at};
use featherplay::snapshot::Scene;
use featherplay::ui::{Frame, HomeData, LyricsData, NowPlayingData, Theme};
use image::RgbaImage;
use slint::{ComponentHandle, LogicalPosition};

use crate::support::{self, frames, idle, label, quantile, scene_at, step, wheel_run};

const W: u32 = 1600;
const H: u32 = 1000;

/// Home, the fixture radio playing, motion off (a theme change at once), the clock frozen.
fn scene() -> Scene {
    let scene = scene_at(W, H, 1.0, "home", true);
    scene.ui.global::<Theme>().set_motion(false);
    idle(&scene, 30);
    scene
}

/// The band's top left corner (the lead line, "Now playing · …" or "Start listening · …", sits
/// `text-x` across and `pad` down from it) and its width (to the first pixel of page ground
/// right of it).
fn band(scene: &Scene, shown: &RgbaImage) -> (u32, u32, u32) {
    let lead = label::shown_where(scene.ui.window(), |_, l| {
        l.starts_with("Now playing") || l.starts_with("Start listening")
    });
    let r = label::rect(lead.first().expect("the band's lead line"));
    let (x, y) = ((r.origin.x - 228.0) as u32, (r.origin.y - 24.0) as u32);
    let ground = *shown.get_pixel(x - 8, y + 100);
    let right = (x + 400..W)
        .find(|&i| support::dist(shown.get_pixel(i, y + 100).0, ground.0) <= 1)
        .expect("the band's right edge");
    (x, y, right - x)
}

/// The band's ground in a frame: a strip along its top, above the cover, its shadow and the
/// text.
fn strip(shown: &RgbaImage, (x, y, _): (u32, u32, u32)) -> RgbaImage {
    image::imageops::crop_imm(shown, x + 30, y + 3, 360, 6).to_image()
}

/// The strip as the band draws it from the bank: the page's rows under the band's middle,
/// dimmed by `dim` (0-255) toward black.
fn from_bank(scene: &Scene, (x, y, w): (u32, u32, u32), dim: u32) -> RgbaImage {
    let np = scene.ui.global::<NowPlayingData>();
    let (bw, bh) = (np.get_backdrop_w() as u32, np.get_backdrop_h() as u32);
    let bank = np.get_backdrop().to_rgba8().expect("the bank");
    let layout = Layout::for_page(bw, bh);
    // The image's top left: its middle under the band's, whole pixels (`BandField`).
    let ix = f64::from(x) + ((f64::from(w) - f64::from(bw)) / 2.0).round();
    let iy = f64::from(y) + ((224.0 - f64::from(bh)) / 2.0).round();
    RgbaImage::from_fn(360, 6, |i, j| {
        let px = (f64::from(x + 30 + i) - ix) as u32;
        let py = (f64::from(y + 3 + j) - iy) as u32;
        let [r, g, b] = at(&layout, bank.as_bytes(), px, py);
        let d = |v: u8| ((u32::from(v) * (255 - dim) + 127) / 255) as u8;
        image::Rgba([d(r), d(g), d(b), 255])
    })
}

/// The largest difference of a channel between two strips, in levels.
fn apart(a: &RgbaImage, b: &RgbaImage) -> u8 {
    a.pixels()
        .zip(b.pixels())
        .flat_map(|(p, q)| (0..3).map(move |k| p[k].abs_diff(q[k])))
        .max()
        .unwrap_or(0)
}

/// How far a strip is from a flat colour at most.
fn from_flat(s: &RgbaImage, c: slint::Color) -> u8 {
    let flat = RgbaImage::from_pixel(
        s.width(),
        s.height(),
        image::Rgba([c.red(), c.green(), c.blue(), 255]),
    );
    apart(s, &flat)
}

/// The frames after `act`, 4 ms of real time apart: the band's strip in each.
fn record(scene: &Scene, n: usize, act: &dyn Fn()) -> Vec<RgbaImage> {
    act();
    (0..n)
        .map(|_| {
            std::thread::sleep(std::time::Duration::from_millis(4));
            step(scene);
            let shown = support::frame(scene);
            strip(&shown, band(scene, &shown))
        })
        .collect()
}

/// A song playing: the band shows the bank's middle rows, dimmed in the dark theme and as
/// they are in the light one. A fresh start's featured pick (nothing playing): its flat
/// colour.
#[test]
fn the_band_sits_on_the_field_while_a_song_plays_and_a_pick_keeps_its_colour() {
    let scene = scene();
    for dark in [true, false] {
        scene.theme(dark);
        support::settle_in_frames(&scene);
        idle(&scene, 30);
        let shown = support::frame(&scene);
        let at = band(&scene, &shown);
        let s = strip(&shown, at);
        let field = from_bank(&scene, at, if dark { 0x33 } else { 0 });
        assert!(
            apart(&s, &field) <= 3,
            "dark={dark}: {} levels from the field",
            apart(&s, &field)
        );
        let lyrics = scene.ui.global::<LyricsData>();
        let flat = if dark {
            lyrics.get_ink_dark()
        } else {
            lyrics.get_ink_tint()
        }
        .ground;
        assert!(
            from_flat(&s, flat) > 6,
            "dark={dark}: the field looks like the flat colour"
        );
    }

    scene.play_track(true, 0).expect("nothing playing");
    support::settle_in_frames(&scene);
    idle(&scene, 120);
    let home = scene.ui.global::<HomeData>();
    assert!(
        !home.get_resume() && home.get_has_featured(),
        "no featured pick"
    );
    let shown = support::frame(&scene);
    let s = strip(&shown, band(&scene, &shown));
    let flat = home.get_featured_ink_light().ground;
    assert!(
        from_flat(&s, flat) <= 1,
        "the pick's band isn't its flat colour"
    );
}

/// A theme change on Home, and the window back from minimised (the bank is let go meanwhile):
/// every frame after shows the new field, none the flat colour.
#[test]
fn the_band_never_shows_its_flat_colour_for_a_frame() {
    let scene = scene();
    let flat = scene.ui.global::<LyricsData>().get_ink_tint().ground;
    let mut seen = vec![("theme", record(&scene, 40, &|| scene.theme(false)))];
    let w = scene.ui.window();
    w.set_minimized(true);
    idle(&scene, 10);
    let np = scene.ui.global::<NowPlayingData>();
    assert_eq!(
        np.get_backdrop().size().width,
        0,
        "the bank stays while minimised"
    );
    seen.push(("restore", record(&scene, 40, &|| w.set_minimized(false))));
    for (name, frames) in seen {
        let last = frames.last().expect("frames");
        assert!(
            from_flat(last, flat) > 6,
            "{name}: the field looks like the flat colour"
        );
        for (i, s) in frames.iter().enumerate() {
            assert!(
                apart(s, last) <= 2,
                "{name}: frame {i} is {} levels from the field",
                apart(s, last)
            );
        }
    }
}

/// A short scroll near the top, a notch at a time, so the band stays in view: the repainted
/// frames' CPU, the cheaper of two runs after a first.
fn scroll_top(scene: &Scene, at: LogicalPosition) -> Vec<f64> {
    let ui = &scene.ui;
    let run = || {
        let mut out = Vec::new();
        for _ in 0..4 {
            for notches in [-1.0, 1.0] {
                ui.global::<Frame>().set_scrolling(true);
                out.extend(
                    wheel_run(ui.window(), at, 2, notches)
                        .into_iter()
                        .filter(|f| f.damaged > 0)
                        .map(|f| f.cpu),
                );
            }
        }
        ui.global::<Frame>().set_scrolling(false);
        out
    };
    let _ = run();
    let (a, b) = (run(), run());
    if a.len() == b.len() {
        a.iter().zip(&b).map(|(x, y)| x.min(*y)).collect()
    } else {
        a
    }
}

/// The frames a song change repaints on Home, real time between them.
fn song_change(scene: &Scene, track: usize, flat: bool) -> Vec<f64> {
    scene.play_track(false, track).expect("track");
    let np = scene.ui.global::<NowPlayingData>();
    let dark = scene.ui.global::<Theme>().get_dark();
    frames(scene.ui.window(), 150, |_| {
        std::thread::sleep(std::time::Duration::from_millis(3));
        scene.run_queued();
        if flat {
            np.set_backdrop_dark(!dark);
        }
    })
    .into_iter()
    .filter(|f| f.damaged > 0)
    .map(|f| f.cpu)
    .collect()
}

#[allow(clippy::print_stdout)]
fn measure(w: u32, h: u32, scale: f32) {
    let scene = scene_at(w, h, scale, "home", false);
    support::long_home(&scene.ui, 12, 12);
    scene.settle();
    frames(scene.ui.window(), 60, |_| {});
    let at = LogicalPosition::new(w as f32 / scale * 0.4, 600.0);
    let np = scene.ui.global::<NowPlayingData>();
    let dark = scene.ui.global::<Theme>().get_dark();
    for flat in [true, false, true, false] {
        // The flat band: the bank said to be the other theme's, as before it's in.
        np.set_backdrop_dark(if flat { !dark } else { dark });
        scene.settle();
        frames(scene.ui.window(), 60, |_| {});
        let top = scroll_top(&scene, at);
        let mut change = Vec::new();
        for track in [1, 0] {
            change.extend(song_change(&scene, track, flat));
            scene.settle();
            frames(scene.ui.window(), 60, |_| {});
        }
        println!(
            "{w}x{h} @{scale} {}: band in view {:.2} / {:.2} ms ({} frames); song change {} frames, median {:.2}, worst {:.2}, sum {:.1} ms",
            if flat { "flat " } else { "field" },
            quantile(&top, 0.5),
            quantile(&top, 0.95),
            top.len(),
            change.len(),
            quantile(&change, 0.5),
            change.iter().copied().fold(0.0, f64::max),
            change.iter().sum::<f64>(),
        );
    }
}

#[test]
#[ignore = "a measurement"]
fn what_the_band_on_the_field_costs_4k() {
    measure(3840, 2160, 1.5);
}

#[test]
#[ignore = "a measurement"]
fn what_the_band_on_the_field_costs_1568() {
    measure(1568, 900, 1.0);
}
