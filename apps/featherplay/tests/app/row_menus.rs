//! The buttons track rows show on hover work, on the offscreen platform (no window is
//! shown): ⋯ opens the song menu and the heart likes an unliked row. `TrackListRow` on the
//! album page, `SongListRow` on a playlist.

use std::cell::Cell;
use std::rc::Rc;

use featherplay::offscreen;
use featherplay::ui::{
    AlbumData, AppWindow, Nav, PageKind, PlaylistData, SongMenu, Theme, TrackRow,
};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, ModelRc, VecModel};

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

fn move_to(w: &slint::Window, p: LogicalPosition) {
    w.dispatch_event(WindowEvent::PointerMoved { position: p });
}

fn click(w: &slint::Window, p: LogicalPosition, button: PointerEventButton) {
    move_to(w, p);
    w.dispatch_event(WindowEvent::PointerPressed {
        position: p,
        button,
    });
    w.dispatch_event(WindowEvent::PointerReleased {
        position: p,
        button,
    });
}

/// The middle of row `id`, found by right-clicking down the page (the headers' heights
/// depend on the page), at `x`.
fn row_centre(w: &slint::Window, menu: &SongMenu, x: f32, id: &str) -> f32 {
    let mut hits = Vec::new();
    for y in (80..700).step_by(4) {
        let y = y as f32;
        click(w, LogicalPosition::new(x, y), PointerEventButton::Right);
        if menu.get_open() && menu.get_track().id == id {
            hits.push(y);
        }
        menu.invoke_close();
    }
    assert!(!hits.is_empty(), "row {id} not found");
    (hits[0] + hits[hits.len() - 1]) / 2.0
}

/// Hovers the row, moves onto its ⋯ and clicks it: the menu opens for that row.
fn more_opens_the_menu(w: &slint::Window, menu: &SongMenu, page: &str) {
    // 1280 px wide without the panel: the content runs from 240 to 1280, the rows from
    // 32 px in on either side, and the ⋯ (36 px) ends 8 px before the row's right edge.
    let y = row_centre(w, menu, 700.0, "b");
    move_to(w, LogicalPosition::new(700.0, y));
    let more = LogicalPosition::new(1248.0 - 8.0 - 18.0, y);
    move_to(w, more);
    click(w, more, PointerEventButton::Left);
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
        (1248.0 - 8.0 - 36.0..=1248.0 - 8.0).contains(&x),
        "{page}: menu anchor x {x} isn't on the ⋯"
    );
    menu.invoke_close();
}

/// Hovers an unliked row, moves onto its heart and clicks it: `liked` gets the row's index.
fn heart_likes(w: &slint::Window, menu: &SongMenu, liked: &Cell<i32>, page: &str) {
    // The heart (36 px) starts 140 px before the row's right edge.
    let y = row_centre(w, menu, 700.0, "c");
    move_to(w, LogicalPosition::new(700.0, y));
    let heart = LogicalPosition::new(1248.0 - 140.0 + 18.0, y);
    move_to(w, heart);
    liked.set(-1);
    click(w, heart, PointerEventButton::Left);
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
    more_opens_the_menu(w, &menu, "album (TrackListRow)");
    heart_likes(w, &menu, &liked, "album (TrackListRow)");

    let playlist = ui.global::<PlaylistData>();
    playlist.set_title("Playlist".into());
    playlist.set_tracks(rows());
    ui.global::<Nav>().set_page(PageKind::Playlist);
    let l = liked.clone();
    playlist.on_like(move |i| l.set(i));
    more_opens_the_menu(w, &menu, "playlist (SongListRow)");
    heart_likes(w, &menu, &liked, "playlist (SongListRow)");
}
