//! Lyrics show once. While Now playing's full lyrics view is open, its side panel shows Up
//! next and the Lyrics tab steps aside; closing the full view brings back the tab the user
//! had. On the offscreen platform with the fixtures; no window is shown.

use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{LyricsData, LyricsState, NowPlayingData, Theme};
use slint::ComponentHandle;

const W: u32 = 1280;
const H: u32 = 800;

/// The side panel's pixels in the lyrics' panel colour (`LyricInk.ground`): the Lyrics tab's
/// card fills most of the panel when it shows.
fn lyrics_card(scene: &Scene) -> usize {
    let w = scene.ui.window();
    offscreen::present(w).expect("present");
    let px = offscreen::shown(w).expect("shown");
    let img =
        image::RgbaImage::from_raw(px.width(), px.height(), px.as_bytes().to_vec()).expect("image");
    let ground = scene.ui.global::<LyricsData>().get_ink().ground;
    let want = [ground.red(), ground.green(), ground.blue()];
    let t = scene.ui.global::<Theme>();
    let x0 = (W as f32 - t.get_nowplaying_side_w()) as u32 + 1;
    let y1 = (H as f32 - t.get_bar_clear()) as u32;
    (64..y1)
        .flat_map(|y| (x0..W).map(move |x| (x, y)))
        .filter(|&(x, y)| img.get_pixel(x, y).0[..3] == want)
        .count()
}

/// One test: the platform can be installed once per process.
#[test]
fn the_side_panel_steps_aside_while_the_full_lyrics_view_is_open() {
    let scene = Scene::new(W, H, true, true).expect("scene");
    scene.play(false).expect("player");
    scene.open("now-playing").expect("now playing");
    scene.theme(true);
    scene.ui.show().expect("show");
    let data = scene.ui.global::<LyricsData>();
    let np = scene.ui.global::<NowPlayingData>();

    // The user reads the lyrics in the side tab.
    np.set_tab(2);
    np.invoke_tab_selected(2);
    scene.settle();
    assert_eq!(data.get_state(), LyricsState::Synced);
    let card = lyrics_card(&scene);
    assert!(
        card > 50_000,
        "the side tab's lyrics don't show ({card} px)"
    );

    // The full view opens: the side panel shows Up next, not the lyrics a second time.
    data.invoke_toggle_view();
    scene.settle();
    assert!(data.get_showing());
    let card = lyrics_card(&scene);
    assert!(
        card < 1_000,
        "the lyrics show twice ({card} px in the side panel)"
    );
    assert_eq!(np.get_tab(), 2, "the user's choice of tab was lost");

    // Closed again: the Lyrics tab is back.
    data.invoke_toggle_view();
    scene.settle();
    assert!(!data.get_showing());
    let card = lyrics_card(&scene);
    assert!(card > 50_000, "the Lyrics tab didn't come back ({card} px)");
}
