//! The buttons track rows show on hover work, on the offscreen platform (no window is
//! shown): ⋯ opens the song menu and the heart likes an unliked row. `TrackListRow` on the
//! album page, `SongListRow` on a playlist.

use std::cell::Cell;
use std::rc::Rc;

use featherplay::offscreen;
use featherplay::ui::{
    AlbumData, AppWindow, Nav, PageKind, PlaylistData, SongMenu, Theme, TrackRow,
};
use slint::platform::PointerEventButton;
use slint::{ComponentHandle, LogicalPosition, ModelRc, VecModel};

use crate::support::label::{self, LogicalRect};
use crate::support::{click_with, point};

fn rows() -> ModelRc<TrackRow> {
    let rows: Vec<TrackRow> = ["a", "b", "c"]
        .iter()
        .enumerate()
        .map(|(i, id)| TrackRow {
            id: (*id).into(),
            index: (i + 1).to_string().into(),
            title: format!("Title {id}").into(),
            artist: "Artist".into(),
            album: "Album".into(),
            duration: "3:00".into(),
            ..Default::default()
        })
        .collect();
    ModelRc::from(Rc::new(VecModel::from(rows)))
}

/// Where the row of song `id` is, found by its label: "Title <id>" on an album's page, and
/// with the artist after it (`artist`) on a playlist's.
fn row(w: &slint::Window, id: &str, artist: bool) -> LogicalRect {
    let name = if artist {
        format!("Title {id}, Artist")
    } else {
        format!("Title {id}")
    };
    label::rect(&label::find(w, &name).unwrap_or_else(|| panic!("no row {name:?}")))
}

/// Hovers `row` and finds its button labelled `name`, which shows on hover.
fn hover_button(w: &slint::Window, row: LogicalRect, name: &str) -> LogicalRect {
    point(w, LogicalPosition::new(row.center().x, row.center().y));
    let found = label::shown_where(w, |item, l| {
        let r = label::rect(item);
        l == name && r.min_y() >= row.min_y() && r.max_y() <= row.max_y()
    });
    label::rect(
        found
            .first()
            .unwrap_or_else(|| panic!("no {name:?} on the row")),
    )
}

/// Hovers the row, moves onto its ⋯ and clicks it: the menu opens for that row.
fn more_opens_the_menu(w: &slint::Window, menu: &SongMenu, page: &str, artist: bool) {
    let b = row(w, "b", artist);
    let y = b.center().y;
    let button = hover_button(w, b, "More");
    let more = LogicalPosition::new(button.center().x, button.center().y);
    point(w, more);
    click_with(w, more, PointerEventButton::Left);
    assert!(menu.get_open(), "{page}: the ⋯ didn't open the song menu");
    assert_eq!(menu.get_track().id, "b", "{page}");
    // Anchored on the row it came from: an upward menu ends at the row's top, a downward one
    // starts inside the row, at the ⋯ (rows are at most 56 px tall).
    let (top, anchor_y) = (menu.get_anchor_top(), menu.get_anchor_y());
    assert!(
        top <= y && y - top <= 56.0,
        "{page}: menu top {top} isn't row b's (centre {y})"
    );
    assert!(
        anchor_y > y && anchor_y - top <= 56.0,
        "{page}: menu anchor {anchor_y} isn't in row b (centre {y})"
    );
    let x = menu.get_anchor_x();
    assert!(
        (button.min_x()..=button.max_x()).contains(&x),
        "{page}: menu anchor x {x} isn't on the ⋯"
    );
    menu.invoke_close();
}

/// Hovers an unliked row, moves onto its heart and clicks it: `liked` gets the row's index.
fn heart_likes(w: &slint::Window, liked: &Cell<i32>, page: &str, artist: bool) {
    let button = hover_button(w, row(w, "c", artist), "Like");
    let heart = LogicalPosition::new(button.center().x, button.center().y);
    point(w, heart);
    liked.set(-1);
    click_with(w, heart, PointerEventButton::Left);
    assert_eq!(liked.get(), 2, "{page}: the heart didn't like the row");
}

/// One test: the platform can be installed once per process.
#[test]
fn more_and_heart_buttons_work() {
    let _pump = offscreen::install().expect("platform");
    let ui = AppWindow::new().expect("window");
    ui.window().set_size(slint::LogicalSize::new(1280.0, 800.0));
    ui.global::<Theme>().set_still(true);
    ui.show().expect("show");
    let w = ui.window();
    let menu = ui.global::<SongMenu>();
    let liked = Rc::new(Cell::new(-1));

    let album = ui.global::<AlbumData>();
    album.set_title("Album".into());
    album.set_tracks(rows());
    ui.global::<Nav>().set_page(PageKind::Album);
    let l = liked.clone();
    album.on_like(move |i| l.set(i));
    more_opens_the_menu(w, &menu, "album (TrackListRow)", false);
    heart_likes(w, &liked, "album (TrackListRow)", false);

    let playlist = ui.global::<PlaylistData>();
    playlist.set_title("Playlist".into());
    playlist.set_tracks(rows());
    ui.global::<Nav>().set_page(PageKind::Playlist);
    let l = liked.clone();
    playlist.on_like(move |i| l.set(i));
    more_opens_the_menu(w, &menu, "playlist (SongListRow)", true);
    heart_likes(w, &liked, "playlist (SongListRow)", true);
}
