//! Wheel scrolling at 4K stays cheap per frame on the pages the user scrolls most: Home (12
//! shelves of 12 cards), a 300-row playlist and the side panel's Related (300 rows). On the
//! offscreen platform, through the app's own present path, 3840×2160 at scale 1.5 with Up
//! next open, art on every cover (`support::fixture_covers`), the hover held as the app holds
//! it under the wheel (`Frame.scrolling`).
//!
//! Measured here 2026-10-31 (release, LTO off), median / p95 ms of CPU a repainted frame,
//! before → after:
//! - Home 6.30 / 10.23 → 3.65 / 4.95. A shelf's clip edges fell between physical pixels at
//!   150 %, so skia drew every cover layer and fill in it through an anti-aliased clip mask;
//!   and the cover layers were RGBA, blended into the BGRA window pixel by pixel.
//! - playlist 5.0 / 6.3 → 3.4 / 3.9. The scrim under the top bar and the header's wash were
//!   see-through gradients.
//! - Related 2.03 / 2.22 → 1.9 / 2.05. Each row's cover edge was stroked as an anti-aliased
//!   path; it's now one cached image.
//!
//! Art adds little to a frame: on Home and Related the same scroll is measured again with no
//! art. Resampling Home's art on every frame (it's decoded at a fixed size and the tiles are
//! fluid) cost 10 ms of CPU a frame; drawn once into each cover's layer, it costs about 2 ms
//! more than no art. Related's art is decoded at its displayed size (`ROW_COVER` × the
//! scale), so it's drawn 1:1, with no resampling and no layer, and adds nothing (2026-10-30).
//! That difference is skia's work, compiled optimised in every build, so it holds in a debug
//! run too.
//!
//! The frame's ceilings hold only in an optimised build, where Slint's part of the frame is
//! what ships (a debug build's item-tree walk alone is over them). CPU time per frame is noisy
//! when other work loads the machine, so each page gets up to 3 tries.

use crate::support::{
    self, TABS_H, TABS_Y, click_tab, frames, home_at, long_rows, panel, quantile, wheel_run,
};
use featherplay::snapshot::Scene;
use featherplay::ui::{AppWindow, Frame, HomeData, NowPlayingData, PlaylistData, TrackRow};
use slint::{ComponentHandle, Image, LogicalPosition, Model};

const W: u32 = 3840;
const H: u32 = 2160;
const SCALE: f32 = 1.5;
const WIDTH: f32 = W as f32 / SCALE;
/// The ceilings (median, p95), ms of CPU a frame, optimised: between before and after.
const HOME: (f64, f64) = (4.6, 7.0);
const PLAYLIST: (f64, f64) = (4.2, 5.0);
const RELATED: (f64, f64) = (2.0, 2.2);
/// What art may add to a frame's median and p95, ms of CPU, in any build.
const HOME_ART: (f64, f64) = (2.5, 4.0);
const RELATED_ART: (f64, f64) = (1.0, 1.5);

/// The repainted frames' CPU times of a wheel run down and back up (two notches every 32 ms),
/// the cheaper of two runs after a first one (which creates the rows and their layers).
fn scroll(ui: &AppWindow, at: LogicalPosition) -> Vec<f64> {
    let w = ui.window();
    let run = || {
        let mut out = Vec::new();
        for notches in [-2.0, 2.0] {
            // As the app's wheel filter does: the hover stays put while the wheel turns.
            ui.global::<Frame>().set_scrolling(true);
            out.extend(
                wheel_run(w, at, 30, notches)
                    .into_iter()
                    .filter(|f| f.damaged > 0)
                    .map(|f| f.cpu),
            );
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

/// What art may add to a page's scroll frame, and how the page loses its art.
struct Art<'a> {
    most: (f64, f64),
    drop: &'a dyn Fn(&Scene),
}

/// Measures `page` up to 3 times and fails only if every try is over a ceiling. `reset`
/// gives the page its long list, with art; with `art`, the same scroll again without it.
fn within(
    scene: &Scene,
    page: &str,
    at: LogicalPosition,
    ceiling: (f64, f64),
    reset: impl Fn(&Scene),
    art: Option<Art>,
) {
    let mut last = String::new();
    for _ in 0..3 {
        reset(scene);
        let cpu = scroll(&scene.ui, at);
        let (m, p) = (quantile(&cpu, 0.5), quantile(&cpu, 0.95));
        println!(
            "4K 150 % {page} scroll, ms of CPU a frame (median / p95): {m:.2} / {p:.2}, {} frames",
            cpu.len()
        );
        assert!(cpu.len() > 200, "only {} frames repainted", cpu.len());
        // Once the wheel stops, nothing big repaints: only the playing marks step. (A cover
        // layer that never settled repainted its cards on every frame, and the scroll's frame
        // count rose to every frame of the run.)
        let window = u64::from(W) * u64::from(H);
        let idle = frames(scene.ui.window(), 100, |_| {});
        let worst = idle.iter().map(|f| f.damaged).max().unwrap_or(0);
        assert!(
            worst * 100 <= window,
            "{page} at rest repaints {worst} px a frame, over 1 % of the window"
        );
        let art_over = art.as_ref().and_then(|art| {
            (art.drop)(scene);
            settle(scene);
            let bare = scroll(&scene.ui, at);
            let (bm, bp) = (quantile(&bare, 0.5), quantile(&bare, 0.95));
            println!(
                "{page} with no art: {bm:.2} / {bp:.2}, {} frames; art adds {:.2} / {:.2}",
                bare.len(),
                m - bm,
                p - bp
            );
            (m - bm > art.most.0 || p - bp > art.most.1).then(|| {
                format!(
                    "art adds {:.2} / {:.2} ms to a {page} frame (median / p95), over {} / {}",
                    m - bm,
                    p - bp,
                    art.most.0,
                    art.most.1
                )
            })
        });
        last = if let Some(over) = art_over {
            over
        } else if !cfg!(debug_assertions) && (m > ceiling.0 || p > ceiling.1) {
            format!(
                "a {page} scroll frame costs {m:.2} / {p:.2} ms, over {} / {}",
                ceiling.0, ceiling.1
            )
        } else {
            return;
        };
    }
    panic!("{last}");
}

fn settle(scene: &Scene) {
    scene.settle();
    frames(scene.ui.window(), 60, |_| {});
}

/// Every card of Home's shelves with its cover made by `f`.
fn set_covers(ui: &AppWindow, mut f: impl FnMut(&Image) -> Image) {
    for shelf in ui.global::<HomeData>().get_shelves().iter() {
        for i in 0..shelf.items.row_count() {
            if let Some(mut item) = shelf.items.row_data(i) {
                item.cover = f(&item.cover);
                shelf.items.set_row_data(i, item);
            }
        }
    }
}

#[test]
fn a_4k_home_scroll_frame_is_cheap() {
    let scene = home_at(W, H, SCALE);
    let home = scene.ui.global::<HomeData>();
    let loaded = home.get_shelves();
    let long = |scene: &Scene| {
        scene.ui.global::<HomeData>().set_shelves(loaded.clone());
        support::long_home(&scene.ui, 12, 12);
        settle(scene);
    };
    long(&scene);
    let with_art = home
        .get_shelves()
        .iter()
        .flat_map(|s| s.items.iter().map(|it| it.cover).collect::<Vec<_>>())
        .filter(|i| i.size().width > 0)
        .count();
    assert!(with_art >= 100, "only {with_art} cards have art");
    within(
        &scene,
        "Home",
        LogicalPosition::new(WIDTH * 0.4, 600.0),
        HOME,
        long,
        Some(Art {
            most: HOME_ART,
            drop: &|scene| set_covers(&scene.ui, |_| Image::default()),
        }),
    );
}

#[test]
fn a_4k_playlist_scroll_frame_is_cheap() {
    let scene = home_at(W, H, SCALE);
    scene.open("playlist").expect("playlist");
    settle(&scene);
    let d = scene.ui.global::<PlaylistData>();
    let rows: Vec<TrackRow> = d.get_tracks().iter().collect();
    assert!(!rows.is_empty(), "no playlist rows");
    within(
        &scene,
        "playlist",
        LogicalPosition::new(WIDTH * 0.4, 600.0),
        PLAYLIST,
        |scene| {
            scene
                .ui
                .global::<PlaylistData>()
                .set_tracks(long_rows(&rows, 300));
            settle(scene);
        },
        None,
    );
}

#[test]
fn a_4k_related_scroll_frame_is_cheap() {
    let scene = home_at(W, H, SCALE);
    click_tab(&scene.ui, WIDTH, 1);
    settle(&scene);
    let d = scene.ui.global::<NowPlayingData>();
    let rows: Vec<TrackRow> = d.get_related().iter().collect();
    let with_art = rows.iter().filter(|r| r.cover.size().width > 0).count();
    assert!(
        !rows.is_empty() && with_art == rows.len(),
        "{with_art} of {} related rows have art",
        rows.len()
    );
    let bare: Vec<TrackRow> = rows
        .iter()
        .cloned()
        .map(|mut r| {
            r.cover = Image::default();
            r
        })
        .collect();
    let (x0, side) = panel(&scene.ui, WIDTH);
    within(
        &scene,
        "Related",
        LogicalPosition::new(x0 + side / 2.0, TABS_Y + TABS_H + 500.0),
        RELATED,
        |scene| {
            scene
                .ui
                .global::<NowPlayingData>()
                .set_related(long_rows(&rows, 300));
            settle(scene);
        },
        Some(Art {
            most: RELATED_ART,
            drop: &|scene| {
                scene
                    .ui
                    .global::<NowPlayingData>()
                    .set_related(long_rows(&bare, 300));
            },
        }),
    );
}
