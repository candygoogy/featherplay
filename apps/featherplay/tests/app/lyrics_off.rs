//! Settings' "Show lyrics" off: the full lyrics view closes, the Lyrics toggles (the bar's
//! microphone, Now playing's pill) and the side panel's Lyrics tab are gone, and nothing asks
//! for lyrics, whatever the user does. On the offscreen platform with the fixtures; no window
//! is shown, and no controller runs, so the setting isn't saved.

use featherplay::lyrics;
use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{LyricsData, NowPlayingData, SettingKey, SettingsData, Theme};
use slint::ComponentHandle;

const W: u32 = 1280;
const H: u32 = 800;

/// The window's pixels in `x0..x1`, `y0..y1`, as presented.
fn region(scene: &Scene, (x0, y0): (u32, u32), (x1, y1): (u32, u32)) -> Vec<u8> {
    let w = scene.ui.window();
    offscreen::present(w).expect("present");
    let px = offscreen::shown(w).expect("shown");
    let img =
        image::RgbaImage::from_raw(px.width(), px.height(), px.as_bytes().to_vec()).expect("image");
    (y0..y1)
        .flat_map(|y| (x0..x1).map(move |x| (x, y)))
        .flat_map(|(x, y)| img.get_pixel(x, y).0)
        .collect()
}

/// The floating bar, where the microphone sits left of ⊘ and the heart.
fn bar(scene: &Scene) -> Vec<u8> {
    let clear = scene.ui.global::<Theme>().get_bar_clear() as u32;
    region(scene, (0, H - clear), (W, H))
}

/// The stage's top right corner, where the Lyrics pill sits.
fn pill(scene: &Scene) -> Vec<u8> {
    let stage_right = W - scene.ui.global::<Theme>().get_nowplaying_side_w() as u32;
    region(scene, (stage_right - 240, 0), (stage_right, 72))
}

/// The side panel's pixels in the lyrics' panel colour: the Lyrics tab's card fills most of
/// the panel when it shows.
fn lyrics_card(scene: &Scene) -> usize {
    let t = scene.ui.global::<Theme>();
    let x0 = (W as f32 - t.get_nowplaying_side_w()) as u32 + 1;
    let y1 = (H as f32 - t.get_bar_clear()) as u32;
    let ground = scene.ui.global::<LyricsData>().get_ink().ground;
    let want = [ground.red(), ground.green(), ground.blue()];
    region(scene, (x0, 64), (W, y1))
        .chunks(4)
        .filter(|p| p[..3] == want)
        .count()
}

/// One test: the platform can be installed once per process.
#[test]
fn with_lyrics_off_nothing_is_looked_up_and_no_lyrics_toggle_or_tab_shows() {
    let scene = Scene::new(W, H, true, true).expect("scene");
    scene.play(false).expect("player");
    scene.open("now-playing").expect("now playing");
    scene.theme(true);
    scene.ui.show().expect("show");
    scene.settle();
    let data = scene.ui.global::<LyricsData>();
    let np = scene.ui.global::<NowPlayingData>();
    let (bar_on, pill_on) = (bar(&scene), pill(&scene));

    // The full view is open when lyrics are turned off in Settings: it closes.
    data.invoke_toggle_view();
    scene.settle();
    assert!(data.get_showing());
    scene
        .ui
        .global::<SettingsData>()
        .invoke_toggled(SettingKey::Lyrics, false);
    scene.settle();
    assert!(
        !data.get_full() && !data.get_showing(),
        "the full view stayed open"
    );
    assert!(!scene.ui.global::<SettingsData>().get_lyrics());

    // Nothing brings them back: the side panel's Lyrics tab picked, the toggles, a new song.
    let asked = lyrics::lookups();
    np.set_tab(2);
    np.invoke_tab_selected(2);
    data.invoke_toggle_view();
    scene.play_track(false, 1).expect("next track");
    scene.settle();
    assert_eq!(lyrics::lookups(), asked, "lyrics were looked up");
    assert!(!data.get_showing(), "the full view opened");
    let card = lyrics_card(&scene);
    assert!(card < 1_000, "the Lyrics tab shows ({card} px)");

    // The microphone and the pill are gone (the full view is closed in both shots).
    scene.play(false).expect("player");
    scene.settle();
    assert_ne!(bar(&scene), bar_on, "the bar's Lyrics button still shows");
    assert_ne!(
        pill(&scene),
        pill_on,
        "Now playing's Lyrics pill still shows"
    );
}
