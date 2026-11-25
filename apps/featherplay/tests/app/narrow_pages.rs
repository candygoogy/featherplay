//! The pages in a narrow window, offscreen (no window is shown). Below 800 px Now playing's
//! lyrics view stacks: the song as a header in the top left, the lines under it on the
//! cover's left edge; without lyrics the cover keeps the page's padding at its sides, so it
//! shrinks in place when they turn on. Home's band takes its narrow form and Quick picks one
//! column; Settings' rows put their controls under their labels where they'd crowd them;
//! the album header stacks. At 360 px (a window manager may force it) nothing runs past the
//! window's edge.

use featherplay::lyrics::{self, SnapshotLyrics};
use featherplay::snapshot::Scene;
use featherplay::ui::{LyricsData, Player, Theme};
use slint::{ComponentHandle, Model};

use crate::support;
use crate::support::label::{self, LogicalRect};

/// Where the shown elements named `label` are, in window coordinates.
fn shown(scene: &Scene, name: &str) -> Vec<LogicalRect> {
    let mut found = label::rects(scene, name);
    found.retain(|r| r.size.width > 0.0);
    found
}

/// The topmost shown element named `label`.
fn first(scene: &Scene, label: &str) -> LogicalRect {
    shown(scene, label)
        .into_iter()
        .min_by(|a, b| a.origin.y.total_cmp(&b.origin.y))
        .unwrap_or_else(|| panic!("no {label:?} shows"))
}

fn resize(scene: &Scene, width: u32, height: u32) {
    support::resize(scene.ui.window(), width, height);
    frame(scene);
}

/// A frame drawn, then what follows it: the change handlers that fit a page's chips and the
/// Accent row's colours to their width run after the frame that laid them out, as in the app.
fn frame(scene: &Scene) {
    scene.settle();
    scene.ui.window().take_snapshot().expect("frame");
    scene.settle();
}

fn page(width: u32, height: u32, name: &str) -> Scene {
    let scene = Scene::new(width, height, true, false).expect("scene");
    scene.play(false).expect("player");
    scene.open(name).expect("page");
    resize(&scene, width, height);
    scene
}

/// Whichever lyric line shows (the view glides to the one playing).
fn any_line(scene: &Scene, data: &LyricsData) -> LogicalRect {
    data.get_lines()
        .iter()
        .filter(|l| !l.trim().is_empty())
        .find_map(|l| shown(scene, &l).into_iter().next())
        .expect("no lyric line shows")
}

#[test]
fn now_playing_stacks_below_800_with_lyrics_and_keeps_the_cover_in_place_without() {
    let scene = page(480, 900, "now-playing");
    let data = scene.ui.global::<LyricsData>();
    data.set_on(true);
    lyrics::show_for_snapshot(&scene.ctx, SnapshotLyrics::Synced);
    scene.settle();
    let title = scene.ui.global::<Player>().get_title().to_string();

    // Without the lyrics view: the block under a cover the page's padding (20 px) from the
    // window's left edge, the title at its edge.
    let block = first(&scene, &title);
    assert!(
        (block.origin.x - 20.0).abs() < 0.5,
        "the cover view's title starts at {}, not the 20 px padding",
        block.origin.x
    );
    assert!(
        block.origin.y > 400.0,
        "the title isn't under the cover: {block:?}"
    );

    // The lyrics view: the title beside the small cover in the header (120 px from the same
    // left edge, then a 20 px gap), the lines under the header from that left edge.
    data.invoke_toggle_view();
    scene.settle();
    assert!(data.get_showing(), "the lyrics view didn't open");
    let header = first(&scene, &title);
    let foot = 196.0 + support::np_drop(&scene.ui);
    assert!(
        (header.origin.x - 160.0).abs() < 0.5 && header.max_y() < foot,
        "the title isn't in the header beside the cover: {header:?}"
    );
    let more = first(&scene, "More");
    let dislike = first(&scene, "Not for me");
    assert!(
        more.max_y() <= foot + 0.5 && (dislike.origin.x - 150.0).abs() < 0.5,
        "the song's buttons aren't at the header's foot: {more:?}"
    );
    // A line is its row, whose hover tile reaches 12 px past the text on either side. The
    // view follows the line playing as it opens, so its top row may be half scrolled away
    // under the view's top edge, which is below the header's foot.
    let lines = any_line(&scene, &data);
    assert!(
        (lines.origin.x + 12.0 - 20.0).abs() < 1.0 && lines.origin.y >= foot,
        "the lines don't run under the header on the cover's edge: {lines:?}"
    );

    // At 800 the lyrics view keeps its column: the lines beside the cover, right of the title.
    resize(&scene, 800, 900);
    let column = first(&scene, &title);
    let lines = any_line(&scene, &data);
    assert!(
        lines.origin.x > column.max_x(),
        "at 800 px the lines aren't beside the column: {column:?} {lines:?}"
    );
}

#[test]
fn home_at_480_has_the_narrow_band_one_column_of_quick_picks_and_no_up_next_link() {
    let scene = page(480, 900, "home");
    // The feed's Quick picks under the band, not the Shortcuts row.
    crate::support::no_shortcuts(&scene);
    frame(&scene);
    let width = scene.ui.window().size().width as f32;
    // The band's row (its actions) inside the page.
    let radio = first(&scene, "Start radio");
    assert!(
        radio.max_x() <= width,
        "the band's row runs past the window: {radio:?}"
    );
    // The bar's Up next button is the way to the queue: no link in the band.
    let link = label::shown_where(&scene, |_, l| l.starts_with("Up next ·"));
    assert!(link.is_empty(), "the band shows its Up next link at 480 px");
    // Quick picks: one column, its rows on one left edge, the next column out of view.
    let songs = scene.ui.global::<featherplay::ui::HomeData>();
    let quick = (0..songs.get_shelves().row_count())
        .filter_map(|i| songs.get_shelves().row_data(i))
        .find(|s| s.shape == featherplay::ui::ShelfShape::Songs)
        .expect("a Quick picks shelf");
    let titles: Vec<String> = (0..3)
        .filter_map(|i| quick.songs.row_data(i))
        .map(|t| t.title.to_string())
        .collect();
    assert!(titles.len() == 3, "fewer than three quick picks");
    let a = first(&scene, &titles[0]);
    let b = first(&scene, &titles[1]);
    assert!(
        (a.origin.x - b.origin.x).abs() < 0.5,
        "the first column's rows don't share an edge"
    );
    let third = shown(&scene, &titles[2]);
    assert!(
        third.iter().all(|r| r.origin.x >= width),
        "a second column shows at 480 px: {third:?}"
    );
}

#[test]
fn settings_rows_put_their_controls_under_their_labels_when_narrow() {
    // Tall enough that the row shows above the bar at either width.
    let scene = page(1100, 1600, "settings-sound");
    let beside = (
        first(&scene, "Skip silence"),
        first(&scene, "Start and end"),
    );
    assert!(
        beside.1.origin.x > beside.0.max_x(),
        "at 1100 px the chips aren't beside the label: {beside:?}"
    );
    resize(&scene, 480, 1600);
    let width = scene.ui.window().size().width as f32;
    let (label, chip) = (
        first(&scene, "Skip silence"),
        first(&scene, "Start and end"),
    );
    assert!(
        chip.origin.y >= label.max_y() && chip.max_x() <= width - 20.0,
        "at 480 px the chips aren't under the label, inside the page: {label:?} {chip:?}"
    );
}

#[test]
fn the_album_header_stacks_on_a_narrow_page() {
    let scene = page(1100, 900, "album");
    let wide = first(&scene, "Daft Punk");
    resize(&scene, 480, 900);
    let narrow = first(&scene, "Daft Punk");
    assert!(
        wide.origin.x > 300.0 && narrow.origin.x < 100.0,
        "the byline doesn't move under the cover: {wide:?} then {narrow:?}"
    );
}

/// Whatever a page shows with a name stays inside the window at 360 px, where a window
/// manager can force it below the minimum.
#[test]
fn nothing_runs_past_the_window_at_360() {
    // One scene (a process installs the offscreen platform once), the pages in turn.
    let scene = page(360, 700, "settings-sound");
    for name in ["settings-sound", "album", "about", "now-playing", "search"] {
        scene.open(name).expect("page");
        frame(&scene);
        let size = scene.ui.window().size();
        let width = size.width as f32;
        // The page's room: under the top bar's row, above the player bar (the frame's own
        // parts are checked by `bar_narrow`).
        let theme = scene.ui.global::<Theme>();
        let (top, bottom) = (
            theme.get_top_bar_h(),
            size.height as f32 - theme.get_bar_clear(),
        );
        let mut out = Vec::new();
        label::shown_where(&scene, |item, l| {
            let r = label::rect(item);
            let inside = r.min_y() >= top && r.max_y() <= bottom;
            if !l.is_empty() && r.size.width > 0.0 && inside && r.max_x() > width + 0.5 {
                out.push(format!("{l} ({} to {})", r.min_x(), r.max_x()));
            }
            false
        });
        assert!(out.is_empty(), "{name}: past the 360 px edge: {out:?}");
    }
}
