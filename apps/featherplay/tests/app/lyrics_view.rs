//! The full lyrics view is optional: Now playing shows the cover until the user turns lyrics
//! on, with the "Lyrics" pill on the stage or the microphone in the player bar. The bar's
//! button works from any page: it opens Now playing in lyrics mode. The choice lasts the
//! session (the next song keeps it, a restart doesn't), and no lyrics are looked up until a
//! lyrics view is on screen. On the offscreen platform with the fixtures; no window is shown.

use featherplay::router::Page;
use featherplay::snapshot::Scene;
use featherplay::ui::{LyricsData, LyricsState, NowPlayingData, Theme};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition};

const W: u32 = 1280;
const H: u32 = 800;

/// The bar's microphone, the first of its right-hand buttons (bottom_bar.slint): the bar is
/// `bar-inset` in from the window's edges, and the row ends `s3` in from the bar's right.
/// Its buttons, left to right: Lyrics 36, Not for me 32, Like 36, Mute 36, the volume 88, a
/// gap of 8, Up next 36, 4 apart.
fn bar_lyrics(scene: &Scene) -> LogicalPosition {
    let t = scene.ui.global::<Theme>();
    let bar_w = W as f32 - 2.0 * t.get_bar_inset();
    let row_w = 36.0 + 32.0 + 36.0 + 36.0 + 88.0 + 8.0 + 36.0 + 6.0 * 4.0;
    // The right-hand group sits as far in from the bar's end as from its top and bottom.
    let end_gap = (t.get_bar_h() - 36.0) / 2.0;
    let x = t.get_bar_inset() + bar_w - end_gap - row_w + 18.0;
    let y = H as f32 - t.get_bar_clear() + t.get_bar_h() / 2.0;
    LogicalPosition::new(x, y)
}

fn click(scene: &Scene, at: LogicalPosition) {
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::PointerMoved { position: at });
    w.dispatch_event(WindowEvent::PointerPressed {
        position: at,
        button: PointerEventButton::Left,
    });
    w.dispatch_event(WindowEvent::PointerReleased {
        position: at,
        button: PointerEventButton::Left,
    });
    // Away again, so the next click is a fresh hover.
    w.dispatch_event(WindowEvent::PointerExited);
    scene.settle();
}

/// Nothing was looked up: the lyrics' state is where a fresh start leaves it.
fn untouched(scene: &Scene) -> bool {
    scene.ui.global::<LyricsData>().get_state() == LyricsState::Idle
}

/// One test: the platform can be installed once per process.
#[test]
fn lyrics_are_an_optional_view_that_lasts_the_session() {
    let scene = Scene::new(W, H, true, false).expect("scene");
    let data = scene.ui.global::<LyricsData>();
    scene.play(false).expect("player");
    scene.open("home").expect("home");
    scene.settle();

    // A fresh start: the cover, not the lyrics, and nothing looked up.
    assert!(!data.get_full(), "lyrics on at startup");
    assert!(untouched(&scene), "lyrics looked up on Home");

    // Now playing on its cover, then with Related picked: still nothing looked up.
    scene.open("now-playing").expect("now playing");
    scene.settle();
    assert!(!data.get_full(), "Now playing opened on the lyrics");
    assert!(
        untouched(&scene),
        "lyrics looked up with only the cover shown"
    );
    let np = scene.ui.global::<NowPlayingData>();
    np.set_tab(1);
    np.invoke_tab_selected(1);
    scene.settle();
    assert!(untouched(&scene), "lyrics looked up with Related shown");
    np.set_tab(0);
    np.invoke_tab_selected(0);
    scene.ctx.back();
    scene.settle();
    assert_eq!(scene.ctx.current_page(), Page::Home);

    // The bar's microphone, from Home: Now playing opens in lyrics mode, and they load.
    click(&scene, bar_lyrics(&scene));
    assert_eq!(
        scene.ctx.current_page(),
        Page::NowPlaying,
        "the bar's Lyrics didn't open Now playing"
    );
    assert!(data.get_full(), "Now playing didn't open on the lyrics");
    assert_eq!(
        data.get_state(),
        LyricsState::Synced,
        "the lyrics didn't load"
    );

    // The next song keeps the lyrics on.
    scene.play_track(false, 1).expect("next track");
    scene.settle();
    assert!(data.get_full(), "a new song turned the lyrics off");

    // Leaving Now playing and coming back by the bar's cover: still on.
    scene.ctx.back();
    scene.settle();
    assert!(data.get_full(), "leaving Now playing turned the lyrics off");
    // Off screen, a new song isn't looked up.
    data.set_state(LyricsState::Idle);
    scene.play_track(false, 2).expect("another track");
    assert!(
        untouched(&scene),
        "lyrics looked up while no lyrics view is shown"
    );
    scene.settle();
    scene.open("now-playing").expect("now playing");
    scene.settle();
    assert!(data.get_full(), "Now playing came back without the lyrics");
    assert_eq!(data.get_state(), LyricsState::Synced);

    // On Now playing, the bar's microphone turns them off again: the cover is back.
    click(&scene, bar_lyrics(&scene));
    assert_eq!(scene.ctx.current_page(), Page::NowPlaying);
    assert!(
        !data.get_full(),
        "the bar's Lyrics didn't turn the lyrics off"
    );
}
