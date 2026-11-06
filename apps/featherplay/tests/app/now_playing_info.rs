//! Now playing's info block, offscreen (no window is shown): "Playing from" sits in the
//! stage's header, shown only when the queue has a source, and a click opens that source
//! (its page, or Up next for a radio); under the credits, the current lyric line shows in a
//! strip only with synced lyrics and the lyrics view off, and a click on it opens the view.

use std::ops::ControlFlow;

use featherplay::controller::PlayingFrom;
use featherplay::lyrics::{self, SnapshotLyrics};
use featherplay::router::Page;
use featherplay::snapshot::Scene;
use featherplay::ui::{Frame, LyricsData, NowPlayingData, QueueData, Theme};
use featherplay_core::model::AlbumId;
use i_slint_core::accessibility::AccessibleStringProperty;
use i_slint_core::item_tree::ItemRc;
use i_slint_core::lengths::LogicalRect;
use i_slint_core::window::WindowInner;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition};

const WIDTH: u32 = 1280;

/// The stage's right edge: the side panel takes the rest.
fn stage_right(scene: &Scene) -> f32 {
    let theme = scene.ui.global::<Theme>();
    let width = WIDTH as f32;
    if scene.ui.global::<Frame>().get_side_open() {
        width - theme.get_nowplaying_side_w()
    } else {
        width
    }
}

/// Where the shown elements whose `property` is `value` are in the stage, in window
/// coordinates.
fn shown(scene: &Scene, property: AccessibleStringProperty, value: &str) -> Vec<LogicalRect> {
    let right = stage_right(scene);
    let root = ItemRc::new_root(WindowInner::from_pub(scene.ui.window()).component());
    let mut found = Vec::new();
    root.visit_descendants(|item| {
        if item.is_visible()
            && item
                .accessible_string_property(property)
                .is_some_and(|v| v == value)
        {
            let g = item.geometry();
            let r = LogicalRect::new(item.map_to_window(g.origin), g.size);
            if r.origin.x < right && r.size.width > 0.0 {
                found.push(r);
            }
        }
        ControlFlow::<()>::Continue(())
    });
    found
}

/// The header's "Playing from …", if it shows.
fn header(scene: &Scene) -> Option<LogicalRect> {
    let from = scene.ui.global::<QueueData>().get_playing_from();
    let label = format!("Playing from {from}");
    shown(scene, AccessibleStringProperty::Label, &label)
        .into_iter()
        .next()
}

/// The lyric strip, if it shows.
fn strip(scene: &Scene) -> Option<LogicalRect> {
    shown(
        scene,
        AccessibleStringProperty::Description,
        "Shows all the lyrics",
    )
    .into_iter()
    .next()
}

fn click(scene: &Scene, r: LogicalRect) {
    let w = scene.ui.window();
    let position = LogicalPosition::new(r.center().x, r.center().y);
    let button = PointerEventButton::Left;
    w.dispatch_event(WindowEvent::PointerMoved { position });
    w.dispatch_event(WindowEvent::PointerPressed { position, button });
    w.dispatch_event(WindowEvent::PointerReleased { position, button });
    scene.settle();
}

fn now_playing() -> Scene {
    let scene = Scene::new(WIDTH, 800, true, false).expect("scene");
    scene.play(false).expect("player");
    scene.open("now-playing").expect("now playing");
    scene.settle();
    scene
}

#[test]
fn the_header_names_the_source_and_hides_without_one() {
    let scene = now_playing();
    let from = header(&scene).expect("no Playing from in the header");
    // Centred in the stage, in its top row with the back button.
    let middle = stage_right(&scene) / 2.0;
    assert!(
        (from.center().x - middle).abs() < 1.0,
        "not centred in the stage: {from:?}, the middle is {middle}"
    );
    assert!(from.max_y() <= 64.0, "below the header row: {from:?}");

    scene.play_from(PlayingFrom::default()).expect("no source");
    scene.settle();
    assert!(header(&scene).is_none(), "it shows with no source");
}

#[test]
fn the_header_opens_the_source_or_up_next() {
    let scene = now_playing();
    // A radio has no page: the side panel comes back on Up next.
    let frame = scene.ui.global::<Frame>();
    let np = scene.ui.global::<NowPlayingData>();
    frame.set_side_open(false);
    np.set_tab(1);
    scene.settle();
    click(&scene, header(&scene).expect("the header"));
    assert!(frame.get_side_open(), "the side panel stayed hidden");
    assert_eq!(np.get_tab(), 0, "not on Up next");
    assert_eq!(scene.ctx.current_page(), Page::NowPlaying);

    // An album: its page.
    let album = Page::Album {
        id: AlbumId("MPREb_hw".into()),
    };
    scene
        .play_from(PlayingFrom::on_page("Homework", album.clone()))
        .expect("album");
    scene.settle();
    click(&scene, header(&scene).expect("the header"));
    assert_eq!(scene.ctx.current_page(), album);
}

#[test]
fn the_lyric_strip_shows_only_with_synced_lyrics() {
    let scene = now_playing();
    let data = scene.ui.global::<LyricsData>();
    lyrics::show_for_snapshot(&scene.ctx, SnapshotLyrics::Plain);
    scene.settle();
    assert!(strip(&scene).is_none(), "it shows with plain lyrics");

    lyrics::show_for_snapshot(&scene.ctx, SnapshotLyrics::Synced);
    scene.settle();
    let at = strip(&scene).expect("no strip with synced lyrics");

    // A click opens the lyrics view, where the strip has no place.
    click(&scene, at);
    assert!(data.get_showing(), "the lyrics view didn't open");
    assert!(strip(&scene).is_none(), "it shows in the lyrics view");
}
