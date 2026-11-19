//! "Cover behind the lyrics": the lyrics menu's switch lays the song's cover, blurred, behind
//! Now playing's full lyrics, made at the stage's size in physical pixels, and takes it away
//! again; and with it on, a playing song draws no more frames than with it off. On the
//! offscreen platform at 1600 x 1000 at 150 %, art on every cover, a frozen clock; no window
//! is shown.

use featherplay::snapshot::Scene;
use featherplay::ui::{LyricsData, LyricsMenu, LyricsState, NowPlayingData, SettingsData, Theme};
use i_slint_core::accessibility::AccessibilityAction;
use i_slint_core::items::AccessibleRole;
use slint::ComponentHandle;

use crate::support::{self, idle, label, long_lyrics, scene_at, step};

const W: u32 = 1600;
const H: u32 = 1000;
const SCALE: f32 = 1.5;

/// Now playing's full view, the long lyrics' synced lines in, the song playing.
fn scene() -> Scene {
    long_lyrics();
    let scene = scene_at(W, H, SCALE, "now-playing", true);
    let data = scene.ui.global::<LyricsData>();
    data.invoke_toggle_view();
    support::settle_in_frames(&scene);
    idle(&scene, 150);
    assert_eq!(data.get_state(), LyricsState::Synced);
    assert!(data.get_showing(), "the full view didn't open");
    scene
}

/// Flips the menu's switch, as a screen reader does.
fn flip(scene: &Scene) {
    let found = label::shown_where(scene, |item, l| {
        l == "Cover behind the lyrics" && item.accessible_role() == AccessibleRole::Switch
    });
    assert_eq!(found.len(), 1, "{} switches shown", found.len());
    found[0].accessible_action(&AccessibilityAction::Default);
    scene.settle();
}

/// The picture's size in physical pixels, (0, 0) without one.
fn picture(scene: &Scene) -> (u32, u32) {
    let s = scene.ui.global::<NowPlayingData>().get_backdrop().size();
    (s.width, s.height)
}

#[test]
fn the_menus_switch_lays_the_cover_behind_the_full_lyrics() {
    let scene = scene();
    assert_eq!(picture(&scene), (0, 0), "a picture while the option is off");
    let menu = scene.ui.global::<LyricsMenu>();
    menu.set_open_asked(menu.get_open_asked() + 1);
    idle(&scene, 60);
    assert!(menu.get_open(), "the ⋯ didn't open the menu");

    flip(&scene);
    support::settle_in_frames(&scene);
    idle(&scene, 120);
    assert!(scene.ui.global::<SettingsData>().get_lyrics_blur());
    assert!(featherplay::pages::settings::lyrics_blur(), "not saved");
    // The stage's size in physical pixels: the window less the side panel (340 px wide at
    // this width), all its height.
    let side = scene.ui.global::<Theme>().get_panel_w();
    let stage = (W as f32 / SCALE - side) * SCALE;
    let (w, h) = picture(&scene);
    assert!(
        (w as f32 - stage).abs() <= 1.5 && h == H,
        "a {w}x{h} picture for {stage}x{H}"
    );
    let np = scene.ui.global::<NowPlayingData>();
    for (edge, strip) in [
        ("top", np.get_backdrop_top()),
        ("bottom", np.get_backdrop_bottom()),
    ] {
        assert!(strip.size().width > 0, "no {edge} strip");
    }
    // It's what the stage shows: the cover's colours across it, where the plain panel (and
    // its wash, which changes only down the stage) is one colour along a row.
    let shown = support::frame(&scene);
    let (left, right) = (
        shown.get_pixel(60, H - 240),
        shown.get_pixel(w - 60, H - 240),
    );
    let apart = (0..3)
        .map(|k| left[k].abs_diff(right[k]))
        .max()
        .unwrap_or(0);
    assert!(
        apart > 6,
        "one colour across the stage: {left:?} and {right:?}"
    );

    // And off again: the picture goes, the panel's colour comes back.
    flip(&scene);
    idle(&scene, 30);
    assert!(!featherplay::pages::settings::lyrics_blur());
    assert_eq!(picture(&scene), (0, 0), "the picture stayed");
}

/// Frames that repainted something, and the pixels they repainted, over `n` frames in which
/// the lines move on `glides` times.
fn playing(scene: &Scene, n: usize, glides: i32) -> (usize, u64) {
    let data = scene.ui.global::<LyricsData>();
    let from = data.get_current().max(0) + 1;
    let (mut frames, mut pixels) = (0, 0);
    for i in 0..n {
        if i % (n / glides as usize) == 0 {
            data.set_current(from + (i / (n / glides as usize)) as i32);
        }
        let f = step(scene);
        frames += usize::from(f.damaged > 0);
        pixels += f.damaged;
    }
    (frames, pixels)
}

/// While the song plays nothing more runs with the picture: the same frames repaint (give or
/// take the mark's phase), over the same pixels, a line's glide and the playing mark's steps alike, and nothing at rest.
#[test]
fn a_playing_song_draws_no_more_frames_with_the_cover_behind() {
    let scene = scene();
    let w = scene.ui.window();
    // The same stretch of the song for each, from the same line.
    let data = scene.ui.global::<LyricsData>();
    data.set_current(10);
    idle(&scene, 200);
    let off = playing(&scene, 960, 4);

    scene.ui.global::<SettingsData>().set_lyrics_blur(true);
    support::settle_in_frames(&scene);
    idle(&scene, 200);
    assert!(picture(&scene).0 > 0, "no picture");
    assert!(!w.has_active_animations(), "something animates at rest");
    data.set_current(10);
    idle(&scene, 200);
    let on = playing(&scene, 960, 4);
    println!(
        "4 s playing, 4 glides: {} frames over {:.1} Mpx without the cover, {} over {:.1} Mpx with it",
        off.0,
        off.1 as f64 / 1e6,
        on.0,
        on.1 as f64 / 1e6
    );
    // The playing mark's steps (30 a second) may land a frame apart from one stretch to
    // the next.
    assert!(
        on.0 <= off.0 + 2,
        "{} frames repainted with the cover, {} without",
        on.0,
        off.0
    );
    assert!(
        on.1 as f64 <= off.1 as f64 * 1.01,
        "{} pixels repainted with the cover, {} without",
        on.1,
        off.1
    );
}
