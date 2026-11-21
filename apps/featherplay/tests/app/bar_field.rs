//! The player bar on the song's cover field (`ui/bottom_bar.slint`, `BarField`): while a song
//! plays it shows Now playing's bank of rows, the slice that lies under the bar on Now
//! playing, dimmed 20 % toward black in the dark theme; with nothing playing it keeps its flat
//! colour. A song change and a theme change go straight from one field to the next, never
//! through the flat colour. On the offscreen platform at 1600 x 1000, the clock frozen.
//!
//! `what_the_bar_on_the_field_costs_*` (ignored) measure a frame of the bar's own repaint (a
//! second of the clock), Home's scroll and a song change: `cargo nextest run --release -p
//! featherplay --test app what_the_bar --run-ignored only --no-capture`.

use featherplay::snapshot::Scene;
use featherplay::ui::{Frame, Player, Theme};
use image::RgbaImage;
use slint::{ComponentHandle, LogicalPosition};

use crate::support::{self, frames, idle, quantile, scene_at, step, wheel_run};

const W: u32 = 1600;
const H: u32 = 1000;

/// Library, the fixture radio playing, motion off (a theme change at once), the clock frozen.
/// Neither Now playing nor Home's band: only the bar asks for the field.
fn scene() -> Scene {
    let scene = scene_at(W, H, 1.0, "library", false);
    scene.ui.global::<Theme>().set_motion(false);
    support::settle_in_frames(&scene);
    idle(&scene, 30);
    scene
}

/// A strip along the bar's top, inside its edge and clear of its corners, the cover and the
/// text: its ground, in physical pixels (scale 1).
fn strip(shown: &RgbaImage, scene: &Scene) -> RgbaImage {
    let t = scene.ui.global::<Theme>();
    let x = (t.get_bar_inset() + t.get_r_xl() + 10.0) as u32;
    let y = (H as f32 - t.get_bar_clear()) as u32 + 3;
    image::imageops::crop_imm(shown, x, y, 300, 4).to_image()
}

/// The strip as the bar draws it from the bank on screen: the window's pixels where Now
/// playing draws them, dimmed by `dim` (0-255) toward black. `None` without a bank.
fn from_bank(scene: &Scene, dim: u32) -> Option<RgbaImage> {
    let field = support::field(&scene.ui)?;
    let t = scene.ui.global::<Theme>();
    let x = (t.get_bar_inset() + t.get_r_xl() + 10.0) as u32;
    let y = (H as f32 - t.get_bar_clear()) as u32 + 3;
    Some(RgbaImage::from_fn(300, 4, |i, j| {
        let [r, g, b] = field(x + i, y + j);
        let d = |v: u8| ((u32::from(v) * (255 - dim) + 127) / 255) as u8;
        image::Rgba([d(r), d(g), d(b), 255])
    }))
}

/// The largest difference of a channel between two strips, in levels.
fn apart(a: &RgbaImage, b: &RgbaImage) -> u8 {
    a.pixels()
        .zip(b.pixels())
        .flat_map(|(p, q)| (0..3).map(move |k| p[k].abs_diff(q[k])))
        .max()
        .unwrap_or(0)
}

/// The strip in the bar's flat colour (`Theme.float-bar`).
fn flat(scene: &Scene) -> RgbaImage {
    let c = scene.ui.global::<Theme>().get_float_bar();
    RgbaImage::from_pixel(300, 4, image::Rgba([c.red(), c.green(), c.blue(), 255]))
}

fn dim(scene: &Scene) -> u32 {
    if scene.ui.global::<Theme>().get_dark() {
        0x33
    } else {
        0
    }
}

/// A song playing: the bar shows the bank's rows under it, dimmed in the dark theme and as
/// they are in the light one, the same on Library and on Now playing. Nothing playing: its
/// flat colour.
#[test]
fn the_bar_sits_on_the_field_while_a_song_plays_and_is_flat_with_nothing_playing() {
    let scene = scene();
    for dark in [true, false] {
        scene.theme(dark);
        support::settle_in_frames(&scene);
        idle(&scene, 30);
        let library = strip(&support::frame(&scene), &scene);
        let field = from_bank(&scene, dim(&scene)).expect("no bank while a song plays");
        assert!(
            apart(&library, &field) <= 3,
            "dark={dark}: {} levels from the field",
            apart(&library, &field)
        );
        assert!(
            apart(&library, &flat(&scene)) > 6,
            "dark={dark}: the field looks like the flat colour"
        );
        support::show(&scene, "now-playing");
        let now_playing = strip(&support::frame(&scene), &scene);
        assert!(
            apart(&now_playing, &library) <= 2,
            "dark={dark}: the bar changed on Now playing, {} levels",
            apart(&now_playing, &library)
        );
        support::show(&scene, "library");
    }

    scene.play_track(true, 0).expect("nothing playing");
    support::settle_in_frames(&scene);
    idle(&scene, 120);
    let s = strip(&support::frame(&scene), &scene);
    assert!(
        apart(&s, &flat(&scene)) <= 1,
        "with nothing playing the bar isn't its flat colour"
    );
}

/// A song change, a theme change and the window back from minimised (the bank is let go
/// meanwhile), on a page with no field of its own: every frame after shows the bar on the
/// bank on screen, never its flat colour.
#[test]
fn the_bar_never_shows_its_flat_colour_for_a_frame() {
    let scene = scene();
    let check = |name: &str, act: &dyn Fn()| {
        act();
        for i in 0..40 {
            std::thread::sleep(std::time::Duration::from_millis(4));
            step(&scene);
            let s = strip(&support::frame(&scene), &scene);
            let field = from_bank(&scene, dim(&scene))
                .unwrap_or_else(|| panic!("{name}: frame {i} has no bank"));
            assert!(
                apart(&s, &field) <= 3,
                "{name}: frame {i} is {} levels from the field",
                apart(&s, &field)
            );
        }
    };
    check("song", &|| scene.play_track(false, 1).expect("track"));
    check("theme", &|| scene.theme(false));
    let w = scene.ui.window();
    w.set_minimized(true);
    idle(&scene, 10);
    check("restore", &|| w.set_minimized(false));
}

/// A short scroll near Home's top, a notch at a time: the repainted frames' CPU, the cheaper
/// of two runs after a first.
fn scroll(scene: &Scene, at: LogicalPosition) -> Vec<f64> {
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

/// The bar's clock moving on a second at a time, as `bar::show_position` sets it: each
/// frame's CPU and the physical pixels it repainted.
fn ticks(scene: &Scene, n: u32) -> (Vec<f64>, Vec<u64>) {
    let player = scene.ui.global::<Player>();
    let from = player.get_position_s().floor();
    let made = frames(scene.ui.window(), 2 * n, |i| {
        if i % 2 == 0 {
            let s = from + 1.0 + (i / 2) as f32;
            player.set_position_s(s);
            player.set_elapsed(format!("{}:{:02}", s as u32 / 60, s as u32 % 60).into());
        }
    });
    let shown: Vec<_> = made.iter().step_by(2).collect();
    (
        shown.iter().map(|f| f.cpu).collect(),
        shown.iter().map(|f| f.damaged).collect(),
    )
}

/// The frames a song change repaints, real time between them.
fn song_change(scene: &Scene, track: usize) -> Vec<f64> {
    scene.play_track(false, track).expect("track");
    frames(scene.ui.window(), 150, |_| {
        std::thread::sleep(std::time::Duration::from_millis(3));
        scene.run_queued();
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
    let t = scene.ui.global::<Theme>();
    let bar =
        f64::from(w as f32 - 2.0 * t.get_bar_inset() * scale) * f64::from(t.get_bar_h() * scale);
    for page in ["home", "library"] {
        support::show(&scene, page);
        frames(scene.ui.window(), 60, |_| {});
        let mut cpu = Vec::new();
        let mut damaged = Vec::new();
        for _ in 0..3 {
            let (c, d) = ticks(&scene, 30);
            cpu.extend(c);
            damaged.extend(d.into_iter().map(|d| d as f64));
        }
        let top = if page == "home" {
            scroll(&scene, LogicalPosition::new(w as f32 / scale * 0.4, 600.0))
        } else {
            Vec::new()
        };
        let mut change = Vec::new();
        for track in [1, 0] {
            change.extend(song_change(&scene, track));
            scene.settle();
            frames(scene.ui.window(), 60, |_| {});
        }
        println!(
            "{w}x{h} @{scale} {page}: clock tick {:.3} / {:.3} ms, {:.0} px ({:.1} % of the bar); scroll {:.2} / {:.2} ms ({} frames); song change {} frames, median {:.2}, worst {:.2}, sum {:.1} ms",
            quantile(&cpu, 0.5),
            quantile(&cpu, 0.95),
            quantile(&damaged, 0.5),
            100.0 * quantile(&damaged, 0.5) / bar,
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
fn what_the_bar_on_the_field_costs_4k() {
    measure(3840, 2160, 1.5);
}

#[test]
#[ignore = "a measurement"]
fn what_the_bar_on_the_field_costs_1568() {
    measure(1568, 900, 1.0);
}
