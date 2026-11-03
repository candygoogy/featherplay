//! Play on hover, on the offscreen platform (no window is shown): hovering a song row's cover,
//! or an album row's number, shows play, and clicking it plays the row as a double-click
//! does; on the current track it pauses and resumes instead. `TrackListRow` on the album
//! page, `SongListRow` on a playlist, `SongRow` as Up next's current track.

use std::cell::Cell;
use std::rc::Rc;

use featherplay::offscreen;
use featherplay::ui::{
    AlbumData, AppWindow, Frame, Nav, PageKind, Player, PlaylistData, QueueData, SongMenu, Theme,
    TrackRow,
};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, ModelRc, VecModel};

/// Three rows; `b` is the current track.
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
            playing: *id == "b",
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

/// The pixels of a 48 px square centred on `p`.
fn around(w: &slint::Window, p: LogicalPosition) -> Vec<slint::Rgba8Pixel> {
    slint::platform::update_timers_and_animations();
    let shot = w.take_snapshot().expect("snapshot");
    let width = shot.width() as usize;
    let (cx, cy) = (p.x as usize, p.y as usize);
    shot.as_slice()
        .chunks(width)
        .skip(cy - 24)
        .take(48)
        .flat_map(|row| row[cx - 24..cx + 24].iter().copied())
        .collect()
}

/// Whether hovering `p` (after resting on the row at `x = 700`) changes what's drawn there:
/// the play glyph shows.
fn hover_shows_play(w: &slint::Window, p: LogicalPosition) -> bool {
    move_to(w, LogicalPosition::new(700.0, p.y));
    let before = around(w, p);
    move_to(w, p);
    around(w, p) != before
}

struct Calls {
    played: Rc<Cell<i32>>,
    toggled: Rc<Cell<u32>>,
}

impl Calls {
    fn reset(&self) {
        self.played.set(-1);
        self.toggled.set(0);
    }
}

/// On the page shown: the play button at `x` plays row `c` (index 2), and on the current
/// row `b` it toggles the player without playing the row.
fn plays_and_toggles(w: &slint::Window, menu: &SongMenu, calls: &Calls, x: f32, page: &str) {
    let c = LogicalPosition::new(x, row_centre(w, menu, 700.0, "c"));
    assert!(hover_shows_play(w, c), "{page}: hovering didn't show play");
    calls.reset();
    click(w, c, PointerEventButton::Left);
    assert_eq!(calls.played.get(), 2, "{page}: play didn't play the row");
    assert_eq!(calls.toggled.get(), 0, "{page}: play toggled the player");
    // A right-click there still opens the row's song menu.
    click(w, c, PointerEventButton::Right);
    assert!(
        menu.get_open(),
        "{page}: right-click on play didn't open the menu"
    );
    assert_eq!(menu.get_track().id, "c", "{page}");
    menu.invoke_close();

    let b = LogicalPosition::new(x, row_centre(w, menu, 700.0, "b"));
    assert!(
        hover_shows_play(w, b),
        "{page}: hovering the current row didn't show pause"
    );
    calls.reset();
    click(w, b, PointerEventButton::Left);
    assert_eq!(
        calls.toggled.get(),
        1,
        "{page}: pause didn't toggle the player"
    );
    assert_eq!(calls.played.get(), -1, "{page}: pause restarted the row");
    move_to(w, LogicalPosition::new(700.0, 760.0));
}

/// One test: the platform can be installed once per process.
#[test]
fn hover_play_plays_the_row() {
    let _pump = offscreen::install().expect("platform");
    let ui = AppWindow::new().expect("window");
    ui.window().set_size(slint::LogicalSize::new(1280.0, 800.0));
    ui.global::<Theme>().set_still(true);
    ui.show().expect("show");
    let w = ui.window();
    let menu = ui.global::<SongMenu>();
    let calls = Calls {
        played: Rc::new(Cell::new(-1)),
        toggled: Rc::new(Cell::new(0)),
    };
    let player = ui.global::<Player>();
    player.set_has_track(true);
    player.set_playing(true);
    let t = calls.toggled.clone();
    player.on_toggle(move || t.set(t.get() + 1));

    // 1280 px wide without the panel: the rows run from 272 px (the content starts at 240,
    // the rows 32 px in). An album row's number cell is centred 36 px in, a song row's
    // 40 px cover 28 px in.
    let album = ui.global::<AlbumData>();
    album.set_title("Album".into());
    album.set_tracks(rows());
    let p = calls.played.clone();
    album.on_play(move |i| p.set(i));
    ui.global::<Nav>().set_page(PageKind::Album);
    plays_and_toggles(w, &menu, &calls, 272.0 + 36.0, "album (TrackListRow)");

    let playlist = ui.global::<PlaylistData>();
    playlist.set_title("Playlist".into());
    playlist.set_tracks(rows());
    let p = calls.played.clone();
    playlist.on_play(move |i| p.set(i));
    ui.global::<Nav>().set_page(PageKind::Playlist);
    plays_and_toggles(w, &menu, &calls, 272.0 + 28.0, "playlist (SongListRow)");

    // Up next's current track (a `SongRow` that isn't a button itself): its cover pauses.
    // The panel starts at 880 px (400 wide at 1280); the row 16 px in, its cover 8 px further. Without a
    // "Playing from" label the row sits at 64 + 40 px.
    let queue = ui.global::<QueueData>();
    queue.set_current(TrackRow {
        id: "b".into(),
        title: "Title b".into(),
        playing: true,
        ..Default::default()
    });
    queue.set_has_current(true);
    ui.global::<Frame>().set_queue_open(true);
    let cover = LogicalPosition::new(880.0 + 16.0 + 8.0 + 20.0, 64.0 + 40.0 + 28.0);
    calls.reset();
    move_to(w, LogicalPosition::new(1200.0, cover.y));
    let before = around(w, cover);
    move_to(w, cover);
    assert!(
        around(w, cover) != before,
        "Up next: hovering didn't show pause"
    );
    click(w, cover, PointerEventButton::Left);
    assert_eq!(
        calls.toggled.get(),
        1,
        "Up next: pause didn't toggle the player"
    );
}
