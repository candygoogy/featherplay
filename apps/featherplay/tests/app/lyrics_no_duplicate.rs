//! Lyrics show once. While Now playing's full lyrics view is open, its side panel shows Up
//! next and the Lyrics tab steps aside; closing the full view brings back the tab the user
//! had. On the offscreen platform with the fixtures; no window is shown.

use featherplay::snapshot::Scene;
use featherplay::ui::{LyricsData, LyricsState, NowPlayingData, Theme};
use slint::{ComponentHandle, Model};

use crate::support::label;

const W: u32 = 1280;
const H: u32 = 800;

/// The lyric lines shown in the side panel (right of the stage, the card's room), as a screen
/// reader finds them.
fn side_lines(scene: &Scene) -> usize {
    let x0 = W as f32 - scene.ui.global::<Theme>().get_nowplaying_side_w();
    scene
        .ui
        .global::<LyricsData>()
        .get_lines()
        .iter()
        .filter(|l| !l.trim().is_empty())
        .flat_map(|l| label::rects(scene, l.as_str()))
        .filter(|r| r.size.width > 0.0 && r.origin.x >= x0)
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
    assert!(side_lines(&scene) > 0, "the side tab's lyrics don't show");

    // The full view opens: the side panel shows Up next, not the lyrics a second time.
    data.invoke_toggle_view();
    scene.settle();
    assert!(data.get_showing());
    let twice = side_lines(&scene);
    assert_eq!(
        twice, 0,
        "the lyrics show twice ({twice} lines in the side panel)"
    );
    assert_eq!(np.get_tab(), 2, "the user's choice of tab was lost");

    // Closed again: the Lyrics tab is back.
    data.invoke_toggle_view();
    scene.settle();
    assert!(!data.get_showing());
    assert!(side_lines(&scene) > 0, "the Lyrics tab didn't come back");
}
