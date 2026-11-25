//! What a crossfade's colour turn costs on Now playing at 4K 150 % (ignored, a measurement;
//! `cargo nextest run --release -p featherplay --run-ignored only -E 'test(what_the_turn)'`).
//! In alternation, rounds of: frames that each put one song's bank of rows or the other's on
//! screen whole (what a song landing on the page costs today, every frame); the turn as
//! `turn` runs it, the field blended (`follow::backdrop::blend_into`, on `IN_FRAME`
//! threads, into one of two banks) and put on screen every third frame, the accent moving
//! every frame; and the lyrics toggle, a motion with a ceiling. The lyrics view's field
//! steps also cut the strips the lines fade into. Each frame in this thread's CPU time on
//! the frozen clock, as `lyrics_frames`.

use featherplay::follow::backdrop::{self, Layout, Lines};
use featherplay::snapshot::Scene;
use featherplay::ui::{LyricsData, NowPlayingData, Theme};
use slint::{ComponentHandle, Image, Rgba8Pixel, SharedPixelBuffer};

use crate::lyrics_frames::scene;
use crate::support::{frames, quantile, step};

const W: u32 = 3840;
const H: u32 = 2160;
/// The turn at 60 frames a second: 1.2 s.
const TURN: u32 = 72;

type Bank = SharedPixelBuffer<Rgba8Pixel>;

fn bank(scene: &Scene) -> Bank {
    scene
        .ui
        .global::<NowPlayingData>()
        .get_backdrop()
        .to_rgba8()
        .expect("a bank on screen")
}

fn put(scene: &Scene, bank: &Bank) {
    let l = Layout::for_page(W, H);
    i_slint_renderer_skia::field_rows(Some((bank.clone(), l.block, l.pad, W, H)));
    scene
        .ui
        .global::<NowPlayingData>()
        .set_backdrop(Image::from_rgba8(bank.clone()));
}

fn strips(scene: &Scene, bank: &Bank) {
    let data = scene.ui.global::<NowPlayingData>();
    let b = data.get_stage_box();
    let at = |v: f32| (v * 1.5).round().max(0.0) as u32;
    let lines = Lines {
        x: at(b.x),
        y: at(b.y),
        w: at(b.lw),
        h: at(b.lh),
        edge: at(b.edge),
    };
    let (top, bottom) = backdrop::strips(&Layout::for_page(W, H), bank.as_bytes(), lines);
    let image = |s: backdrop::Strip| {
        if s.width == 0 || s.height == 0 {
            return Image::default();
        }
        Image::from_rgba8_premultiplied(SharedPixelBuffer::clone_from_slice(
            &s.pixels, s.width, s.height,
        ))
    };
    data.set_backdrop_top(image(top));
    data.set_backdrop_bottom(image(bottom));
}

fn cpu(f: &[crate::support::Frame1]) -> Vec<f64> {
    f.iter().map(|f| f.cpu).collect()
}

#[allow(clippy::print_stdout)]
fn report(name: &str, v: &[f64]) {
    println!(
        "{name}: median {:.2} ms, p95 {:.2}, worst {:.2} ({} frames)",
        quantile(v, 0.5),
        quantile(v, 0.95),
        v.iter().copied().fold(0.0, f64::max),
        v.len()
    );
}

#[test]
#[ignore = "a measurement"]
#[allow(clippy::print_stdout)]
fn what_the_turn_costs_4k() {
    let scene = scene();
    let a = bank(&scene);
    scene.play_track(false, 1).expect("track");
    scene.settle();
    for _ in 0..60 {
        step(&scene);
    }
    let b = bank(&scene);
    assert_ne!(a.as_bytes(), b.as_bytes(), "two songs with one field");

    let data = scene.ui.global::<LyricsData>();
    let theme = scene.ui.global::<Theme>();
    let (from, to) = (
        theme.get_follow_accent(),
        slint::Color::from_rgb_u8(40, 200, 120),
    );
    // Two banks the turn writes into in turn: the one on screen is held by the renderer and
    // the page, the other by nothing, so it's written in place.
    let mut spare = [
        Bank::new(a.width(), a.height()),
        Bank::new(a.width(), a.height()),
    ];
    for view in ["cover view", "lyrics view"] {
        if view == "lyrics view" {
            data.invoke_toggle_view();
            for _ in 0..240 {
                step(&scene);
            }
        }
        let lyrics = view == "lyrics view";
        let w = scene.ui.window();
        let (mut whole, mut turn, mut field, mut toggle) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for _ in 0..3 {
            whole.extend(cpu(&frames(w, TURN, |i| {
                let bank = if i % 2 == 0 { &a } else { &b };
                put(&scene, bank);
                if lyrics {
                    strips(&scene, bank);
                }
            })));
            let made = frames(w, TURN, |i| {
                let t = i as f32 / (TURN - 1) as f32;
                let ch = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t) as u8;
                theme.set_follow_accent(slint::Color::from_rgb_u8(
                    ch(from.red(), to.red()),
                    ch(from.green(), to.green()),
                    ch(from.blue(), to.blue()),
                ));
                if i % 3 == 0 || i == TURN - 1 {
                    let mixed = &mut spare[(i / 3) as usize % 2];
                    backdrop::blend_into(
                        a.as_bytes(),
                        b.as_bytes(),
                        t,
                        mixed.make_mut_bytes(),
                        backdrop::IN_FRAME,
                    )
                    .expect("same size");
                    put(&scene, mixed);
                    if lyrics {
                        strips(&scene, mixed);
                    }
                }
            });
            turn.extend(cpu(&made));
            field.extend(
                made.iter()
                    .enumerate()
                    .filter(|(i, _)| i % 3 == 0)
                    .map(|(_, f)| f.cpu),
            );
            theme.set_follow_accent(from);
            put(&scene, &b);
            if lyrics {
                strips(&scene, &b);
            }
            for _ in 0..30 {
                step(&scene);
            }
            // The lyrics toggle there and back (`lyrics_frames`): its frames past the first.
            for _ in 0..2 {
                data.invoke_toggle_view();
                let mut made = Vec::new();
                for _ in 0..200 {
                    made.push(step(&scene));
                    if !w.has_active_animations() {
                        break;
                    }
                }
                toggle.extend(made.iter().skip(1).filter(|f| f.damaged > 0).map(|f| f.cpu));
                for _ in 0..30 {
                    step(&scene);
                }
            }
        }
        println!("{view}:");
        report(
            "  a whole bank put on screen, each frame (today's landing frame)",
            &whole,
        );
        report("  the turn, all its frames (the field every third)", &turn);
        report("  the turn, its field's frames", &field);
        report("  the lyrics toggle's frames (ceiling 8 / 12 ms)", &toggle);
    }
}
