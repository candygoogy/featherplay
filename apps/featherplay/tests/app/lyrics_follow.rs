//! The lyrics follow the song from the moment they show. Opened mid-song, the line playing is
//! in view and highlighted (`LyricInk.current`), in the side tab and in Now playing's full
//! view, without waiting for the next line to start, and again after a resize. At a song's
//! start the lines sit at the top of their panel: the view never scrolls above its first
//! line, so there's no empty block over it, and the glide only takes over once the line
//! playing reaches a third of the way down. On the offscreen platform with the fixtures (the
//! song at 2:41, line 15 of 24 playing); no window is shown.

use featherplay::snapshot::Scene;
use featherplay::ui::{LyricsData, LyricsState, NowPlayingData, Theme};
use slint::ComponentHandle;

use crate::support::{self, frame};

const W: u32 = 1280;
const H: u32 = 800;

/// A box in logical (here also physical) pixels: x0, y0, x1, y1.
type Area = (u32, u32, u32, u32);

/// The full view's lyrics column (now_playing.slint's `fl-*`), inside the edges' fades.
fn full_column(t: &Theme) -> Area {
    let side = if W as f32 >= t.get_wide_min() {
        t.get_nowplaying_side_w()
    } else {
        t.get_panel_w()
    };
    let stage = W as f32 - side;
    let pad = t.get_page_pad();
    let cover = (stage * 0.2).clamp(160.0, 280.0);
    let lyrics_w = (stage - 2.0 * pad - cover - t.get_s9()).min(720.0);
    let x = pad.max((stage - cover - t.get_s9() - lyrics_w) / 2.0);
    let lyrics_x = x + cover + t.get_s9();
    let edge = t.get_lyric_edge();
    let drop = if cfg!(windows) { 40.0 } else { 0.0 };
    (
        lyrics_x as u32,
        (72.0 + drop + edge) as u32,
        (lyrics_x + lyrics_w) as u32,
        (H as f32 - t.get_bar_clear() - edge) as u32,
    )
}

/// The side tab's card (`SideTabs` puts the view 64 px down; the card is `s4` in), inside the
/// edges' fades.
fn side_card(t: &Theme) -> Area {
    let side = t.get_nowplaying_side_w();
    let x0 = W as f32 - side + t.get_s4();
    let edge = t.get_lyric_edge();
    (
        x0 as u32,
        (64.0 + t.get_panel_top() + edge) as u32,
        (W as f32 - t.get_s4()) as u32,
        (H as f32 - t.get_bar_clear() - t.get_s2() - edge) as u32,
    )
}

/// The line playing's colour.
fn current_rgb(data: &LyricsData) -> [u8; 3] {
    let c = data.get_ink().current;
    [c.red(), c.green(), c.blue()]
}

/// The pixels in `area` that are the line playing's colour, `ink`.
fn current_ink(img: &image::RgbaImage, (x0, y0, x1, y1): Area, ink: [u8; 3]) -> usize {
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|&(x, y)| img.get_pixel(x, y).0[..3] == ink)
        .count()
}

/// The first row from `y0` down with text between `x0` and `x1`: a pixel far from the
/// panel's colour, read in the middle of that first row.
fn first_text_row(img: &image::RgbaImage, x0: u32, x1: u32, y0: u32, y1: u32) -> Option<u32> {
    let ground = img.get_pixel((x0 + x1) / 2, y0).0;
    (y0..y1).find(|&y| (x0..x1).any(|x| support::far(img.get_pixel(x, y).0, ground)))
}

fn save(img: &image::RgbaImage, name: &str) {
    if let Some(dir) = std::env::var_os("LYRICS_FOLLOW_SHOTS") {
        img.save(std::path::Path::new(&dir).join(name))
            .expect("png");
    }
}

/// One test: the platform can be installed once per process.
#[test]
fn the_lyrics_start_at_the_top_and_show_the_line_playing() {
    let scene = Scene::new(W, H, true, true).expect("scene");
    scene.play(false).expect("player");
    scene.open("now-playing").expect("now playing");
    scene.theme(true);
    scene.ui.show().expect("show");
    let t = scene.ui.global::<Theme>();
    let data = scene.ui.global::<LyricsData>();
    let np = scene.ui.global::<NowPlayingData>();

    // The side tab, opened mid-song: the line playing is in view.
    np.set_tab(2);
    np.invoke_tab_selected(2);
    scene.settle();
    assert_eq!(data.get_state(), LyricsState::Synced);
    let playing = data.get_current();
    assert!(playing > 10, "the fixture plays line {playing}");
    let ink = current_rgb(&data);
    let side = frame(&scene);
    save(&side, "side-mid-song.png");
    let lit = current_ink(&side, side_card(&t), ink);
    assert!(
        lit > 150,
        "side tab mid-song: the line playing isn't in view ({lit} px)"
    );

    // Before the first line, then on it: the lines start right under the side tab's top.
    let side_w = t.get_nowplaying_side_w();
    let (x0, x1) = (
        (W as f32 - side_w + t.get_s4() + t.get_s3()) as u32,
        (W as f32 - t.get_s4() - t.get_s3()) as u32,
    );
    let (top, bottom) = (64.0 + t.get_panel_top(), H as f32 - t.get_bar_clear());
    for current in [-1, 0] {
        data.set_current(current);
        scene.settle();
        let img = frame(&scene);
        save(&img, &format!("side-start{current}.png"));
        let row = first_text_row(&img, x0, x1, top as u32 + 2, bottom as u32).expect("lines");
        assert!(
            (row as f32) < top + t.get_s5() + t.get_t_lyric_side(),
            "side tab at the start, line {current}: the first line starts at y {row}"
        );
    }

    // The full view, opened mid-song on lines already loaded: the line playing is in view.
    data.set_current(playing);
    data.set_full(true);
    scene.settle();
    let full = frame(&scene);
    save(&full, "full-mid-song.png");
    let lit = current_ink(&full, full_column(&t), ink);
    assert!(
        lit > 150,
        "full view mid-song: the line playing isn't in view ({lit} px)"
    );

    // The full view at the start: the lines start right under its top too.
    let stage = W as f32 - side_w;
    let (x0, x1) = ((stage * 0.45) as u32, (stage - t.get_s5()) as u32);
    let top = 72.0 + crate::support::np_drop(&scene.ui);
    let (top, bottom) = (top, H as f32 - t.get_bar_clear());
    let size = t.get_t_lyric_wide().max(t.get_t_lyric_full());
    for current in [-1, 0] {
        data.set_current(current);
        scene.settle();
        let img = frame(&scene);
        save(&img, &format!("full-start{current}.png"));
        let row = first_text_row(&img, x0, x1, top as u32 + 2, bottom as u32).expect("lines");
        assert!(
            (row as f32) < top + t.get_s5() + size,
            "full view at the start, line {current}: the first line starts at y {row}"
        );
    }

    // Far into the song the glide has taken over: the first line has scrolled away, and
    // the line playing sits about a third of the way down.
    data.set_current(15);
    scene.settle();
    let img = frame(&scene);
    save(&img, "full-later.png");
    let third = (top + (bottom - top) / 3.0) as u32;
    let lit: Vec<u32> = (top as u32 + 24..bottom as u32 - 24)
        .filter(|&y| (x0..x1).any(|x| img.get_pixel(x, y).0[..3] == ink))
        .collect();
    assert!(!lit.is_empty(), "full view at line 15: no line is lit");
    let (first, last) = (lit[0], lit[lit.len() - 1]);
    assert!(
        first <= third + 8 && last + 8 >= third,
        "full view at line 15: the line playing spans y {first}..{last}, not the third at {third}"
    );

    // A resize lays the lines out again: still on the line playing.
    let w = scene.ui.window();
    for (width, height) in [(1000.0, 640.0), (W as f32, H as f32)] {
        support::resize(w, width as u32, height as u32);
        scene.settle();
    }
    let full = frame(&scene);
    save(&full, "full-resized.png");
    let lit = current_ink(&full, full_column(&t), ink);
    assert!(
        lit > 150,
        "full view after a resize: the line playing isn't in view ({lit} px)"
    );
}
