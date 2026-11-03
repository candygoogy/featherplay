//! The lyrics follow the song from the moment they show. Opened mid-song, the line playing is
//! in view and highlighted (white, `LyricInk.current`), in the side tab and in Now playing's
//! full view, without waiting for the next line to start, and again after a resize. On the
//! offscreen platform with the fixtures (the song at 2:41, line 15 of 24 playing); no window
//! is shown.

use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{LyricsData, LyricsState, NowPlayingData, Theme};
use slint::ComponentHandle;
use slint::platform::WindowEvent;

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
    (
        lyrics_x as u32,
        (72.0 + edge) as u32,
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
        (64.0 + edge) as u32,
        (W as f32 - t.get_s4()) as u32,
        (H as f32 - t.get_bar_clear() - t.get_s2() - edge) as u32,
    )
}

fn frame(scene: &Scene) -> image::RgbaImage {
    let w = scene.ui.window();
    offscreen::present(w).expect("present");
    let px = offscreen::shown(w).expect("shown");
    image::RgbaImage::from_raw(px.width(), px.height(), px.as_bytes().to_vec()).expect("image")
}

/// The pixels in `area` that are the line playing's colour, white.
fn current_ink(img: &image::RgbaImage, (x0, y0, x1, y1): Area) -> usize {
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .filter(|&(x, y)| img.get_pixel(x, y).0[..3] == [255, 255, 255])
        .count()
}

fn save(img: &image::RgbaImage, name: &str) {
    if let Some(dir) = std::env::var_os("LYRICS_FOLLOW_SHOTS") {
        img.save(std::path::Path::new(&dir).join(name))
            .expect("png");
    }
}

/// One test: the platform can be installed once per process.
#[test]
fn the_lyrics_show_the_line_playing_from_the_start() {
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
    assert!(
        data.get_current() > 10,
        "the fixture plays line {}",
        data.get_current()
    );
    let side = frame(&scene);
    save(&side, "side-mid-song.png");
    let lit = current_ink(&side, side_card(&t));
    assert!(
        lit > 150,
        "side tab: the line playing isn't in view ({lit} px)"
    );

    // The full view, opened mid-song on lines already loaded: the same.
    data.set_full(true);
    scene.settle();
    let full = frame(&scene);
    save(&full, "full-mid-song.png");
    let lit = current_ink(&full, full_column(&t));
    assert!(
        lit > 150,
        "full view: the line playing isn't in view ({lit} px)"
    );

    // A resize lays the lines out again: still on the line playing.
    let w = scene.ui.window();
    for (width, height) in [(1000.0, 640.0), (W as f32, H as f32)] {
        w.set_size(slint::PhysicalSize::new(width as u32, height as u32));
        w.dispatch_event(WindowEvent::Resized {
            size: slint::LogicalSize::new(width, height),
        });
        scene.settle();
    }
    let full = frame(&scene);
    save(&full, "full-resized.png");
    let lit = current_ink(&full, full_column(&t));
    assert!(
        lit > 150,
        "after a resize the line playing isn't in view ({lit} px)"
    );
}
