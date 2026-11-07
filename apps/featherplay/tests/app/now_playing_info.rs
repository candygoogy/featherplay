//! Now playing's info block, offscreen (no window is shown): "Playing from" sits in the
//! stage's header, shown only when the queue has a source, and a click opens that source
//! (its page, or Up next for a radio). Under the cover, the title, the artists and "album ·
//! year", and no lyric line: lyrics show only in their views. The cover has its full size in
//! a 1280 x 800 window.

use std::ops::ControlFlow;

use featherplay::controller::PlayingFrom;
use featherplay::lyrics::{self, SnapshotLyrics};
use featherplay::router::Page;
use featherplay::snapshot::Scene;
use featherplay::ui::{Frame, LyricsData, NowPlayingData, Player, QueueData, Theme};
use featherplay_core::model::AlbumId;
use i_slint_core::accessibility::AccessibleStringProperty;
use i_slint_core::item_tree::ItemRc;
use i_slint_core::lengths::LogicalRect;
use i_slint_core::window::WindowInner;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model};

/// The stage's right edge: the side panel takes the rest.
fn stage_right(scene: &Scene) -> f32 {
    let theme = scene.ui.global::<Theme>();
    let width = scene.ui.window().size().width as f32;
    if scene.ui.global::<Frame>().get_side_open() {
        width
            - if width >= theme.get_wide_min() {
                theme.get_nowplaying_side_w()
            } else {
                theme.get_panel_w()
            }
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

/// The block under the cover, above the bar: the cover's size (from the title's left edge,
/// which is the cover's, to the song menu's button, which hangs `s2` past its right edge),
/// the title, and the album line, the block's last.
fn block(scene: &Scene) -> (f32, LogicalRect, LogicalRect) {
    let theme = scene.ui.global::<Theme>();
    let bar_top = scene.ui.window().size().height as f32 - theme.get_bar_clear();
    let above_bar = |label: &str| {
        shown(scene, AccessibleStringProperty::Label, label)
            .into_iter()
            .filter(|r| r.max_y() <= bar_top)
            .min_by(|a, b| a.origin.y.total_cmp(&b.origin.y))
            .unwrap_or_else(|| panic!("no {label:?} in the stage"))
    };
    let player = scene.ui.global::<Player>();
    let title = above_bar(&player.get_title());
    let more = above_bar("More");
    let album = above_bar(&player.get_album());
    (more.max_x() - theme.get_s2() - title.origin.x, title, album)
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

/// Now playing in a 1280 x 800 window, with lyrics on.
fn now_playing() -> Scene {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.ui.global::<LyricsData>().set_on(true);
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
fn no_lyric_line_shows_under_the_credits() {
    let scene = now_playing();
    let data = scene.ui.global::<LyricsData>();
    lyrics::show_for_snapshot(&scene.ctx, SnapshotLyrics::Synced);
    scene.settle();
    let lines: Vec<_> = data
        .get_lines()
        .iter()
        .filter(|l| !l.trim().is_empty())
        .collect();
    assert!(!lines.is_empty(), "no synced lines");
    let shows_a_line = |scene: &Scene| {
        lines
            .iter()
            .any(|l| !shown(scene, AccessibleStringProperty::Label, l).is_empty())
    };
    assert!(
        !shows_a_line(&scene),
        "a lyric line shows on the cover view"
    );

    // The lyrics view shows them, so the probe above would see one.
    data.invoke_toggle_view();
    scene.settle();
    assert!(data.get_showing(), "the lyrics view didn't open");
    assert!(shows_a_line(&scene), "no lines in the lyrics view");
}

#[test]
fn the_cover_has_its_full_size_with_lyrics_on_in_a_1280_by_800_window() {
    let scene = now_playing();
    lyrics::show_for_snapshot(&scene.ctx, SnapshotLyrics::Synced);
    scene.settle();
    let (cover, _, _) = block(&scene);
    let full = scene.ui.global::<Theme>().get_nowplaying_cover();
    assert!(
        (cover - full).abs() < 0.5,
        "the cover is {cover} px beside the panel, not {full}"
    );
}
