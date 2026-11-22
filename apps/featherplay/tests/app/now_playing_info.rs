//! Now playing's info block, offscreen (no window is shown): "Playing from" sits in the
//! stage's header, shown only when the queue has a source, and a click opens that source
//! (its page, or Up next for a radio). Under the cover, the title, the artists and "album ·
//! year", and no lyric line: lyrics show only in their views. The cover has its full size in
//! a 1280 x 800 window, and in smaller ones the block gives way so it ends above the bar.

use featherplay::controller::PlayingFrom;
use featherplay::lyrics::{self, SnapshotLyrics};
use featherplay::router::Page;
use featherplay::snapshot::Scene;
use featherplay::ui::{Frame, LyricsData, NowPlayingData, Player, QueueData, Theme};
use featherplay_controller::place::Place;
use featherplay_core::model::AlbumId;
use slint::{ComponentHandle, LogicalPosition, Model};

use crate::support;
use crate::support::label::{self, LogicalRect};

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

/// Where the shown elements labelled `value` are in the stage, in window coordinates.
fn shown(scene: &Scene, value: &str) -> Vec<LogicalRect> {
    let right = stage_right(scene);
    let mut found = label::rects(scene, value);
    found.retain(|r| r.origin.x < right && r.size.width > 0.0);
    found
}

/// The header's "Playing from …", if it shows.
fn header(scene: &Scene) -> Option<LogicalRect> {
    let from = scene.ui.global::<QueueData>().get_playing_from();
    let label = format!("Playing from {from}");
    shown(scene, &label).into_iter().next()
}

/// The block under the cover, above the bar: the cover's size (from the title's left edge,
/// which is the cover's, to the song menu's button, which hangs `s2` past its right edge),
/// the title, and the album line, the block's last.
fn block(scene: &Scene) -> (f32, LogicalRect, LogicalRect) {
    let theme = scene.ui.global::<Theme>();
    let bar_top = scene.ui.window().size().height as f32 - theme.get_bar_clear();
    let above_bar = |label: &str| {
        shown(scene, label)
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
    support::click(scene, LogicalPosition::new(r.center().x, r.center().y));
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
fn the_header_names_the_source_opens_it_or_up_next_and_hides_without_one() {
    let scene = now_playing();
    let from = header(&scene).expect("no Playing from in the header");
    // Centred in the stage, in its top row with the back button.
    let middle = stage_right(&scene) / 2.0;
    assert!(
        (from.center().x - middle).abs() < 1.0,
        "not centred in the stage: {from:?}, the middle is {middle}"
    );
    let row_end = 64.0 + support::np_drop(&scene.ui);
    assert!(from.max_y() <= row_end, "below the header row: {from:?}");

    // A radio has no page: the side panel comes back on Up next.
    let frame = scene.ui.global::<Frame>();
    let np = scene.ui.global::<NowPlayingData>();
    frame.set_side_open(false);
    np.set_tab(1);
    scene.settle();
    click(&scene, header(&scene).expect("the header"));
    assert!(
        frame.get_side_open(),
        "a radio's header: the side panel stayed hidden"
    );
    assert_eq!(np.get_tab(), 0, "a radio's header: not on Up next");
    assert_eq!(
        scene.ctx.current_page(),
        Page::NowPlaying,
        "a radio's header left Now playing"
    );

    scene.play_from(PlayingFrom::default()).expect("no source");
    scene.settle();
    assert!(header(&scene).is_none(), "it shows with no source");

    // An album: its page.
    let album = Place::Album(AlbumId("MPREb_hw".into()));
    scene
        .play_from(PlayingFrom::on_page("Homework", album.clone()))
        .expect("album");
    scene.settle();
    click(&scene, header(&scene).expect("the header"));
    assert_eq!(
        scene.ctx.current_page(),
        Page::from(album),
        "an album's header didn't open its page"
    );
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
    let shows_a_line = |scene: &Scene| lines.iter().any(|l| !shown(scene, l).is_empty());
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
fn the_cover_has_its_full_size_at_1280_by_800_and_in_small_windows_the_title_steps_down() {
    let scene = now_playing();
    // With lyrics on, in a 1280 x 800 window, the cover has its full size beside the panel.
    lyrics::show_for_snapshot(&scene.ctx, SnapshotLyrics::Synced);
    scene.settle();
    let (cover, _, _) = block(&scene);
    let full = scene.ui.global::<Theme>().get_nowplaying_cover();
    assert!(
        (cover - full).abs() < 0.5,
        "1280 x 800: the cover is {cover} px beside the panel, not {full}"
    );

    // A title that wraps to its second line.
    scene
        .ui
        .global::<Player>()
        .set_title("Harder, Better, Faster, Stronger (Alive 2007 Live Edit)".into());
    scene.settle();
    let (_, large, _) = block(&scene);
    let theme = scene.ui.global::<Theme>();
    // The last is a 1456 x 1000 window at 150 %.
    // With the header row in the window's own title bar the stage has 8 px more room above
    // the cover: the same room is a window 8 px shorter.
    let less = if theme.get_panel_top() > 0.0 { 8 } else { 0 };
    for (width, height) in [(1100, 700 - less), (1000, 650 - less), (970, 667 - less)] {
        let w = scene.ui.window();
        support::resize(w, width, height);
        scene.settle();
        let (cover, title, album) = block(&scene);
        assert!(
            title.size.height < large.size.height,
            "{width} x {height}: the title didn't step down: {title:?}, {large:?} at 1280 x 800"
        );
        let bar_top = height as f32 - theme.get_bar_clear();
        assert!(
            album.max_y() <= bar_top - theme.get_s5() + 0.5,
            "{width} x {height}: the credits end at {}, the bar starts at {bar_top}",
            album.max_y()
        );
        assert!(
            cover >= 280.0,
            "{width} x {height}: the cover is only {cover} px"
        );
    }
}

#[test]
fn a_singles_album_named_as_the_song_says_single_once_its_page_says_so() {
    let scene = now_playing();
    let bar_top =
        scene.ui.window().size().height as f32 - scene.ui.global::<Theme>().get_bar_clear();
    // In the stage, above the bar (which shows the title too).
    let staged = |text: &str| {
        shown(&scene, text)
            .into_iter()
            .filter(|r| r.max_y() <= bar_top)
            .count()
    };
    let player = scene.ui.global::<Player>();
    let title = player.get_title();
    player.set_album(title.clone());
    let data = scene.ui.global::<NowPlayingData>();
    data.set_year("2024".into());
    data.set_single(false);
    scene.settle();
    // Not known as a single: the album's name would only repeat the title, so it goes.
    assert_eq!(
        staged("Single"),
        0,
        "Single before the album's page said so"
    );
    assert_eq!(staged(&title), 1, "the title shows twice: {title}");
    data.set_single(true);
    scene.settle();
    // A link: its text and its button both carry the label.
    assert!(staged("Single") > 0, "no Single under the title");
    assert_eq!(staged(&title), 1, "the title shows twice: {title}");
}
