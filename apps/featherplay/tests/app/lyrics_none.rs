//! Lyrics on, but none to show: Now playing's full view doesn't turn into an empty lyrics
//! page. The cover stays large and centred in the stage, on the lyrics' panel colour, with the
//! title and artist under it and one quiet line under those saying why. After a failed
//! lookup that line ends in Try again, a text button that asks again (`LyricsData.retry`);
//! "No lyrics for this song" is an answer, so it has none. On the offscreen platform with the
//! fixtures and art on every cover; no window is shown.

use std::cell::Cell;
use std::rc::Rc;

use featherplay::fixtures::Fixtures;
use featherplay::lyrics::{self, SnapshotLyrics};
use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{LyricsData, LyricsState, Theme};
use image::RgbaImage;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition};

use crate::support;

const W: u32 = 1280;
const H: u32 = 800;

fn frame(scene: &Scene) -> RgbaImage {
    let w = scene.ui.window();
    offscreen::present(w).expect("present");
    let px = offscreen::shown(w).expect("shown");
    RgbaImage::from_raw(px.width(), px.height(), px.as_bytes().to_vec()).expect("image")
}

fn save(img: &RgbaImage, name: &str) {
    if let Some(dir) = std::env::var_os("LYRICS_NONE_SHOTS") {
        img.save(std::path::Path::new(&dir).join(name))
            .expect("png");
    }
}

fn far(p: [u8; 4], q: [u8; 4]) -> bool {
    (0..3).map(|i| u32::from(p[i].abs_diff(q[i]))).sum::<u32>() > 120
}

/// What the stage shows below its top row of buttons: the cover's box (left, right, top,
/// bottom) and, under it, the runs of rows with text (top, bottom, and the text's right end).
struct Stage {
    cover: (u32, u32, u32, u32),
    runs: Vec<(u32, u32, u32)>,
}

fn read_stage(scene: &Scene, img: &RgbaImage) -> Stage {
    let t = scene.ui.global::<Theme>();
    let side = if W as f32 >= t.get_wide_min() {
        t.get_nowplaying_side_w()
    } else {
        t.get_panel_w()
    };
    let stage = (W as f32 - side) as u32 - 8;
    let bottom = (H as f32 - t.get_bar_clear()) as u32;
    // The stage's ground, left of the cover and the text.
    let ground = img.get_pixel(4, 300).0;
    // The cover: the first block of rows where most of a wide stretch differs from the
    // ground (the art), ending at the first row that doesn't (the gap above the title).
    let mut cover: Option<(u32, u32, u32, u32)> = None;
    for y in 72..bottom {
        let hits: Vec<u32> = (0..stage)
            .filter(|&x| far(img.get_pixel(x, y).0, ground))
            .collect();
        let art = match (hits.first(), hits.last()) {
            (Some(&l), Some(&r)) if r - l > 120 && hits.len() as u32 * 2 > r - l => Some((l, r)),
            _ => None,
        };
        match (art, cover) {
            (Some((l, r)), None) => cover = Some((l, r, y, y)),
            (Some((l, r)), Some((l0, r0, t0, _))) => cover = Some((l0.min(l), r0.max(r), t0, y)),
            (None, Some(_)) => break,
            (None, None) => {}
        }
    }
    let cover = cover.expect("no cover on the stage");
    // Under it, every row with text from the cover's left edge on.
    let mut runs: Vec<(u32, u32, u32)> = Vec::new();
    for y in cover.3 + 1..bottom {
        let right = (cover.0..stage)
            .rev()
            .find(|&x| far(img.get_pixel(x, y).0, ground));
        let Some(right) = right else { continue };
        match runs.last_mut() {
            Some(run) if y <= run.1 + 2 => {
                run.1 = y;
                run.2 = run.2.max(right);
            }
            _ => runs.push((y, y, right)),
        }
    }
    // A single row is the cover's rounded bottom edge, not a line of text.
    runs.retain(|run| run.1 > run.0);
    Stage { cover, runs }
}

fn lyrics_on(scene: &Scene, kind: SnapshotLyrics) {
    let data = scene.ui.global::<LyricsData>();
    data.set_full(true);
    scene.settle();
    lyrics::show_for_snapshot(&scene.ctx, kind);
    scene.settle();
}

/// The playing track (the radio's first) wears a dark cover with a narrow red stripe, whatever
/// the machine has cached: the panel takes the red, and every row of the art is mostly dark,
/// clear of it, so the stage reading finds the cover's edges.
fn covers() {
    let f = Fixtures::load().expect("fixtures");
    let radio = f.radio().expect("radio");
    let url = radio[0].thumbnail.clone().expect("track 0's cover");
    let art = image::RgbImage::from_fn(120, 120, |x, _| {
        if (50..70).contains(&x) {
            image::Rgb([210, 40, 50])
        } else {
            image::Rgb([16, 16, 20])
        }
    });
    support::home_with_covers("lyrics-none-home", &[(&url, art)]);
}

fn scene() -> Scene {
    covers();
    support::fixture_covers();
    let scene = Scene::new(W, H, true, true).expect("scene");
    scene.play(false).expect("player");
    scene.open("now-playing").expect("now playing");
    scene.theme(true);
    scene.ui.show().expect("show");
    scene.settle();
    scene
}

#[test]
fn no_lyrics_keep_the_cover_centred_with_a_quiet_line_under_the_title() {
    let scene = scene();
    lyrics_on(&scene, SnapshotLyrics::None);
    assert_eq!(
        scene.ui.global::<LyricsData>().get_state(),
        LyricsState::None
    );
    let img = frame(&scene);
    save(&img, "none.png");
    let s = read_stage(&scene, &img);
    let t = scene.ui.global::<Theme>();
    let stage = W as f32 - t.get_nowplaying_side_w();
    let (l, r, top, bottom) = s.cover;

    // Large and centred, as the cover view has it: not the lyrics view's small column.
    assert!(r - l > 300, "the cover is {} px wide", r - l);
    let mid = (l + r) as f32 / 2.0;
    assert!(
        (mid - stage / 2.0).abs() <= 6.0,
        "the cover's centre is at x {mid}, the stage's at {}",
        stage / 2.0
    );
    assert!(bottom > top);

    // Under it: the title, the artist, then the quiet line, and nothing else.
    assert_eq!(
        s.runs.len(),
        3,
        "under the cover, text at {:?} (title, artist and the note expected)",
        s.runs
    );
    let note = s.runs[2];
    assert!(note.0 > s.runs[1].1, "the note isn't under the artist");
    // The note is quieter than the title: smaller.
    assert!(
        note.1 - note.0 < s.runs[0].1 - s.runs[0].0,
        "the note is as tall as the title"
    );
}

#[test]
fn only_a_failed_lookup_offers_try_again_and_it_asks_again() {
    let scene = scene();
    let data = scene.ui.global::<LyricsData>();
    let w = scene.ui.window();
    let press = |at: LogicalPosition| {
        w.dispatch_event(WindowEvent::PointerMoved { position: at });
        w.dispatch_event(WindowEvent::PointerPressed {
            position: at,
            button: PointerEventButton::Left,
        });
        w.dispatch_event(WindowEvent::PointerReleased {
            position: at,
            button: PointerEventButton::Left,
        });
        w.dispatch_event(WindowEvent::PointerExited);
        scene.settle();
    };

    // "No lyrics for this song": the line ends where its words do.
    lyrics_on(&scene, SnapshotLyrics::None);
    let none = read_stage(&scene, &frame(&scene));
    let none_note = *none.runs.last().expect("note");

    // A failed lookup: the same layout, the line running on into Try again.
    lyrics_on(&scene, SnapshotLyrics::Failed);
    assert_eq!(data.get_state(), LyricsState::Error);
    let asked = Rc::new(Cell::new(0));
    let a = asked.clone();
    data.on_retry(move || a.set(a.get() + 1));
    let img = frame(&scene);
    save(&img, "failed.png");
    let failed = read_stage(&scene, &img);
    assert_eq!(failed.cover, none.cover, "a failure moved the cover");
    assert_eq!(failed.runs.len(), 3, "under the cover: {:?}", failed.runs);
    let note = failed.runs[2];
    // Try again is the line's last words: press its middle, a little in from the end.
    let at = LogicalPosition::new((note.2 as f32) - 24.0, ((note.0 + note.1) as f32) / 2.0);

    // It shows it's pressable: a hover lights it.
    let before = frame(&scene);
    w.dispatch_event(WindowEvent::PointerMoved { position: at });
    scene.settle();
    let after = frame(&scene);
    let lit = (note.0 - 8..note.1 + 8)
        .flat_map(|y| (note.2 - 120..note.2 + 16).map(move |x| (x, y)))
        .filter(|&(x, y)| before.get_pixel(x, y) != after.get_pixel(x, y))
        .count();
    assert!(lit > 50, "Try again didn't answer the pointer ({lit} px)");
    w.dispatch_event(WindowEvent::PointerExited);
    scene.settle();

    press(at);
    assert_eq!(asked.get(), 1, "Try again didn't ask");

    // LRCLIB down: the same offer.
    lyrics_on(&scene, SnapshotLyrics::Unavailable);
    assert_eq!(data.get_message(), "Lyrics are unavailable right now");
    let down = read_stage(&scene, &frame(&scene));
    let down_note = *down.runs.last().expect("note");
    press(LogicalPosition::new(
        (down_note.2 as f32) - 24.0,
        ((down_note.0 + down_note.1) as f32) / 2.0,
    ));
    assert_eq!(asked.get(), 2, "Try again didn't ask after an outage");

    // Nothing to try again for "No lyrics for this song".
    lyrics_on(&scene, SnapshotLyrics::None);
    press(at);
    assert_eq!(asked.get(), 2, "a none-found view asked again");
    assert!(
        none_note.2 + 40 < note.2,
        "the none-found line ({}) reaches as far as the failed one ({})",
        none_note.2,
        note.2
    );
}
