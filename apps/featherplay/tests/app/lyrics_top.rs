//! At a song's start the synced lines sit at the top of their panel: the view never scrolls
//! above its first line, so there's no empty block over it. The glide only takes over once
//! the line playing reaches a third of the way down. Both views: the side tab and Now
//! playing's full view. On the offscreen platform with the fixtures; no window is shown.

use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{LyricsData, LyricsState, NowPlayingData, Theme};
use slint::ComponentHandle;

const W: u32 = 1280;
const H: u32 = 800;

fn frame(scene: &Scene) -> image::RgbaImage {
    let w = scene.ui.window();
    offscreen::present(w).expect("present");
    let px = offscreen::shown(w).expect("shown");
    image::RgbaImage::from_raw(px.width(), px.height(), px.as_bytes().to_vec()).expect("image")
}

fn save(img: &image::RgbaImage, name: &str) {
    if let Some(dir) = std::env::var_os("LYRICS_FOLLOW_SHOTS") {
        img.save(std::path::Path::new(&dir).join(name))
            .expect("png");
    }
}

/// The first row from `y0` down with text between `x0` and `x1`: a pixel far from the
/// panel's colour, read in the middle of that first row.
fn first_text_row(img: &image::RgbaImage, x0: u32, x1: u32, y0: u32, y1: u32) -> Option<u32> {
    let ground = img.get_pixel((x0 + x1) / 2, y0).0;
    (y0..y1).find(|&y| {
        (x0..x1).any(|x| {
            let p = img.get_pixel(x, y).0;
            (0..3)
                .map(|i| u32::from(p[i].abs_diff(ground[i])))
                .sum::<u32>()
                > 120
        })
    })
}

/// Where a line at `share` of the way down the lines' view sits, from the window's top.
fn row_at(top: f32, bottom: f32, share: f32) -> f32 {
    top + (bottom - top) * share
}

/// One test: the platform can be installed once per process.
#[test]
fn the_lines_start_at_the_top_of_the_panel() {
    let scene = Scene::new(W, H, true, true).expect("scene");
    scene.play(false).expect("player");
    scene.open("now-playing").expect("now playing");
    scene.theme(true);
    scene.ui.show().expect("show");
    let t = scene.ui.global::<Theme>();
    let data = scene.ui.global::<LyricsData>();
    let np = scene.ui.global::<NowPlayingData>();
    np.set_tab(2);
    np.invoke_tab_selected(2);
    scene.settle();
    assert_eq!(data.get_state(), LyricsState::Synced);

    // Before the first line, then on it: the lines start right under the panel's top.
    let side = t.get_nowplaying_side_w();
    let (x0, x1) = (
        (W as f32 - side + t.get_s4() + t.get_s3()) as u32,
        (W as f32 - t.get_s4() - t.get_s3()) as u32,
    );
    let (top, bottom) = (64.0, H as f32 - t.get_bar_clear());
    for current in [-1, 0] {
        data.set_current(current);
        scene.settle();
        let img = frame(&scene);
        save(&img, &format!("side-start{current}.png"));
        let row = first_text_row(&img, x0, x1, top as u32 + 2, bottom as u32).expect("lines");
        assert!(
            (row as f32) < top + t.get_s5() + t.get_t_lyric_side(),
            "side tab, line {current}: the first line starts at y {row}"
        );
    }

    // Full view, the same.
    data.set_full(true);
    scene.settle();
    let stage = W as f32 - side;
    let (x0, x1) = ((stage * 0.45) as u32, (stage - t.get_s5()) as u32);
    let (top, bottom) = (72.0, H as f32 - t.get_bar_clear());
    let size = t.get_t_lyric_wide().max(t.get_t_lyric_full());
    for current in [-1, 0] {
        data.set_current(current);
        scene.settle();
        let img = frame(&scene);
        save(&img, &format!("full-start{current}.png"));
        let row = first_text_row(&img, x0, x1, top as u32 + 2, bottom as u32).expect("lines");
        assert!(
            (row as f32) < top + t.get_s5() + size,
            "full view, line {current}: the first line starts at y {row}"
        );
    }

    // Far into the song the glide has taken over: the first line has scrolled away, and
    // the line playing sits about a third of the way down (white, the current ink).
    data.set_current(15);
    scene.settle();
    let img = frame(&scene);
    save(&img, "full-later.png");
    let third = row_at(top, bottom, 1.0 / 3.0) as u32;
    let lit: Vec<u32> = (top as u32 + 24..bottom as u32 - 24)
        .filter(|&y| (x0..x1).any(|x| img.get_pixel(x, y).0[..3] == [255, 255, 255]))
        .collect();
    let (first, last) = (lit[0], lit[lit.len() - 1]);
    assert!(
        first <= third + 8 && last + 8 >= third,
        "the line playing spans y {first}..{last}, not the third at {third}"
    );
}
