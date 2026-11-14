//! Home's band, on the offscreen platform (no window is shown): its main pill resumes a
//! paused session, opens the source while it plays and plays the pick on a fresh start; its
//! round actions act on the band's song; Up next shows its rows and "Show all" beside the
//! song in a wide window, and an "Up next · N" link in a narrow one, both opening the panel;
//! the band's queue follows the player's, the Up next panel open or not; a second column
//! only for six songs and room for it; "More like this" under a short queue, asked for only
//! where something would show it.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use featherplay::app;
use featherplay::controller::{Action, PlayerState, PlayingFrom, QueueAction, QueueInfo, UiEvent};
use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{
    AppWindow, CardItem, Frame, HomeData, ItemKind, Nav, Player, QueueData, SongMenu, Theme,
    TrackRow,
};
use featherplay_controller::fixtures::Fixtures;
use featherplay_core::model::{RepeatMode, Track};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model, ModelRc, VecModel};

fn row(id: &str) -> TrackRow {
    TrackRow {
        id: id.into(),
        title: format!("Title {id}").into(),
        artist: "Artist".into(),
        artist_id: format!("UC{id}").into(),
        duration: "3:00".into(),
        ..Default::default()
    }
}

fn rows(n: usize) -> ModelRc<TrackRow> {
    let rows: Vec<TrackRow> = (0..n).map(|i| row(&format!("q{i}"))).collect();
    ModelRc::new(VecModel::from(rows))
}

fn click(w: &slint::Window, x: f32, y: f32, button: PointerEventButton) {
    let p = LogicalPosition::new(x, y);
    w.dispatch_event(WindowEvent::PointerMoved { position: p });
    w.dispatch_event(WindowEvent::PointerPressed {
        position: p,
        button,
    });
    w.dispatch_event(WindowEvent::PointerReleased {
        position: p,
        button,
    });
    // Away, so the next click's hover starts fresh.
    w.dispatch_event(WindowEvent::PointerMoved {
        position: LogicalPosition::new(4.0, 4.0),
    });
}

fn window(width: f32) -> AppWindow {
    let ui = AppWindow::new().expect("window");
    // Home's rows, as the app's wiring gives them.
    featherplay::pages::home::bind_rows(&ui);
    ui.window().set_size(slint::LogicalSize::new(width, 900.0));
    ui.global::<Theme>().set_still(true);
    ui.show().expect("show");
    ui
}

// At 1568 px the band runs from x 280 (the 240 px sidebar, the 40 px padding) for 1248 px,
// from y 96 (the 64 px top bar, 32 px of page top); its text column from 228 px in, its row
// of actions at the foot of the 176 px cover, 24 px in.
const BAND_X: f32 = 280.0;
const BAND_Y: f32 = 96.0;
const TEXT_X: f32 = BAND_X + 228.0;
const ROW_Y: f32 = BAND_Y + 24.0 + 136.0 + 20.0;

#[test]
fn the_bands_pill_follows_the_session_and_its_actions_act_on_the_song() {
    let _pump = offscreen::install().expect("platform");
    let ui = window(1568.0);
    let w = ui.window();
    let did = Rc::new(RefCell::new(Vec::<String>::new()));
    let data = ui.global::<HomeData>();
    let queue = ui.global::<QueueData>();
    let player = ui.global::<Player>();
    let menu = ui.global::<SongMenu>();
    let d = did.clone();
    data.on_resume_clicked(move || d.borrow_mut().push("resume".into()));
    let d = did.clone();
    data.on_play_featured(move || d.borrow_mut().push("play featured".into()));
    let d = did.clone();
    queue.on_open_source(move || d.borrow_mut().push("open source".into()));
    let d = did.clone();
    menu.on_start_radio(move |id| d.borrow_mut().push(format!("radio {id}")));
    let d = did.clone();
    ui.global::<Nav>()
        .on_open_artist(move |id| d.borrow_mut().push(format!("artist {id}")));

    // Paused: Resume. Playing: Open <source>, in the same place.
    data.set_resume(true);
    data.set_track(row("t1"));
    queue.set_playing_from("Daily mix".into());
    click(w, TEXT_X + 40.0, ROW_Y, PointerEventButton::Left);
    player.set_playing(true);
    click(w, TEXT_X + 40.0, ROW_Y, PointerEventButton::Left);
    // A fresh start: Play on the featured song.
    data.set_resume(false);
    data.set_has_featured(true);
    data.set_featured(CardItem {
        id: "t1".into(),
        kind: ItemKind::Song,
        title: "Title t1".into(),
        ..Default::default()
    });
    click(w, TEXT_X + 40.0, ROW_Y, PointerEventButton::Left);
    assert_eq!(
        *did.borrow(),
        ["resume", "open source", "play featured"],
        "the band's pill"
    );

    // Playing from a source without a name, there's no pill: the four actions start the row
    // (Like, Add to playlist, Start radio, Go to artist; 40 px, 8 apart).
    did.borrow_mut().clear();
    data.set_resume(true);
    queue.set_playing_from("".into());
    click(w, TEXT_X + 96.0 + 20.0, ROW_Y, PointerEventButton::Left);
    click(w, TEXT_X + 144.0 + 20.0, ROW_Y, PointerEventButton::Left);
    assert_eq!(*did.borrow(), ["radio t1", "artist UCt1"], "the actions");
    // Add to playlist opens the song menu on its playlists.
    menu.set_signed_in(true);
    click(w, TEXT_X + 48.0 + 20.0, ROW_Y, PointerEventButton::Left);
    assert!(
        menu.get_open() && menu.get_choosing(),
        "no playlist chooser"
    );
    assert_eq!(menu.get_track().id, "t1");
}

#[test]
fn up_next_shows_beside_the_song_or_as_a_link_and_opens_the_panel() {
    let _pump = offscreen::install().expect("platform");
    let ui = window(1568.0);
    let w = ui.window();
    let did = Rc::new(RefCell::new(Vec::<String>::new()));
    let data = ui.global::<HomeData>();
    let queue = ui.global::<QueueData>();
    let menu = ui.global::<SongMenu>();
    let d = did.clone();
    data.on_show_queue(move || d.borrow_mut().push("show queue".into()));
    let d = did.clone();
    queue.on_play(move |i| d.borrow_mut().push(format!("play {i}")));
    data.set_resume(true);
    data.set_track(row("t1"));
    data.set_next(rows(3));
    queue.set_upcoming(rows(12));

    // The column from 732 px in: its heading 20 px tall, then 52 px rows; "Show all 12" at
    // its right end, 24 + 8 px from the band's.
    let col_x = BAND_X + 732.0;
    let rows_y = BAND_Y + 24.0 + 20.0;
    click(
        w,
        col_x + 120.0,
        rows_y + 52.0 + 26.0,
        PointerEventButton::Left,
    );
    click(
        w,
        BAND_X + 1248.0 - 32.0 - 20.0,
        BAND_Y + 24.0 + 10.0,
        PointerEventButton::Left,
    );
    assert_eq!(*did.borrow(), ["play 1", "show queue"], "the column");
    // A right-click on a row opens the song menu on it.
    click(w, col_x + 120.0, rows_y + 26.0, PointerEventButton::Right);
    assert!(menu.get_open());
    assert_eq!(menu.get_track().id, "q0");
    menu.invoke_close();

    // At 1100 px the band is 780 px: no column, the link at its top right instead.
    did.borrow_mut().clear();
    w.set_size(slint::LogicalSize::new(1100.0, 900.0));
    w.dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(1100.0, 900.0),
    });
    let band_w = 1100.0 - 240.0 - 80.0;
    click(
        w,
        BAND_X + band_w - 24.0 - 40.0,
        BAND_Y + 24.0 + 10.0,
        PointerEventButton::Left,
    );
    assert_eq!(*did.borrow(), ["show queue"], "the link");
}

#[test]
fn a_second_column_needs_six_songs_and_room_and_a_short_queue_gets_more_like_this() {
    let _pump = offscreen::install().expect("platform");
    let ui = window(1920.0);
    let w = ui.window();
    let did = Rc::new(RefCell::new(Vec::<String>::new()));
    let data = ui.global::<HomeData>();
    let queue = ui.global::<QueueData>();
    let d = did.clone();
    queue.on_play(move |i| d.borrow_mut().push(format!("play {i}")));
    let d = did.clone();
    data.on_play_related(move |i| d.borrow_mut().push(format!("related {i}")));
    data.set_resume(true);
    data.set_track(row("t1"));
    // Row `r` of a column, 26 px into it: under the 20 px heading, 52 px rows.
    let row_y = |r: f32| BAND_Y + 24.0 + 20.0 + r * 52.0 + 26.0;
    let clicks = |x: f32, ys: &[f32]| {
        did.borrow_mut().clear();
        for &y in ys {
            click(w, BAND_X + x, y, PointerEventButton::Left);
        }
        std::mem::take(&mut *did.borrow_mut())
    };

    // At 1920 the band is 1600 px. Four songs: one 520 px column at its right end, from
    // 1056 px in, the song's side taking the rest; no second column for the fourth song.
    data.set_next(rows(4));
    queue.set_upcoming(rows(4));
    assert_eq!(clicks(1056.0 + 120.0, &[row_y(0.0)]), ["play 0"]);
    assert!(
        clicks(732.0 + 120.0, &[row_y(0.0)]).is_empty(),
        "the song's side"
    );
    // Six: two 410 px columns from 732 px, three rows each.
    data.set_next(rows(6));
    queue.set_upcoming(rows(6));
    assert_eq!(
        clicks(732.0 + 434.0 + 120.0, &[row_y(0.0), row_y(2.0)]),
        ["play 3", "play 5"]
    );

    // At 1568 (a 1248 px band) six don't fit in two: one column, from 732 px.
    w.set_size(slint::LogicalSize::new(1568.0, 900.0));
    w.dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(1568.0, 900.0),
    });
    assert_eq!(clicks(732.0 + 120.0, &[row_y(2.0)]), ["play 2"]);

    // One song left: it, then "More like this" with two related songs, in 44 px rows (the
    // second heading 4 px under the queue's row).
    data.set_next(rows(1));
    queue.set_upcoming(rows(1));
    data.set_related(rows(2));
    let more_y = BAND_Y + 24.0 + 20.0 + 44.0 + 4.0 + 20.0;
    assert_eq!(
        clicks(732.0 + 120.0, &[more_y + 22.0, more_y + 44.0 + 22.0]),
        ["related 0", "related 1"]
    );
}

/// The controller's state as a radio has it: `current` playing, `upcoming` after it.
fn radio_state(current: &Track, upcoming: &[Track]) -> PlayerState {
    PlayerState {
        current: Some(current.clone()),
        upcoming: upcoming.to_vec(),
        playing: true,
        audible: true,
        loading: false,
        position: Duration::from_secs(8),
        shuffle: false,
        repeat: RepeatMode::Off,
        radio: true,
        volume: 1.0,
        queue: QueueInfo {
            playing_from: PlayingFrom::named("A radio"),
            autoplay_at: None,
        },
        speed: 1.0,
    }
}

#[test]
fn the_bands_queue_follows_the_players_with_the_panel_open() {
    // 2000 px with the 400 px Up next panel open: the band is 1280 px, room for its queue.
    let scene = Scene::new(2000, 900, true, false).expect("scene");
    let ui = &scene.ui;
    let mut tracks = Fixtures::load().expect("fixtures").radio().expect("radio");
    let current = tracks.remove(0);
    ui.global::<Frame>().set_queue_open(true);
    // A radio just started: nothing after the song until its continuation comes.
    app::on_event(UiEvent::Player(Box::new(radio_state(&current, &[]))));
    scene.open("home").expect("home");
    ui.show().expect("show");
    scene.settle();
    let data = ui.global::<HomeData>();
    assert_eq!(data.get_next().row_count(), 0);
    // Nothing next: "More like this" fills the column without the Related tab being opened.
    assert!(
        data.get_related().row_count() > 0,
        "the band didn't ask for related songs"
    );

    // The continuation: the band shows the panel's first songs, beside the open panel.
    app::on_event(UiEvent::Player(Box::new(radio_state(&current, &tracks))));
    scene.settle();
    let panel = ui.global::<QueueData>().get_upcoming();
    assert!(
        data.get_next().row_count() >= 3,
        "the band's queue didn't follow"
    );
    for i in 0..3 {
        assert_eq!(
            data.get_next().row_data(i).map(|r| r.id),
            panel.row_data(i).map(|r| r.id),
            "the band and the panel differ at {i}"
        );
    }
    let _ = scene.take_actions();
    click(
        ui.window(),
        BAND_X + 732.0 + 120.0,
        BAND_Y + 24.0 + 20.0 + 52.0 + 26.0,
        PointerEventButton::Left,
    );
    assert!(
        scene
            .take_actions()
            .iter()
            .any(|a| matches!(a, Action::Queue(QueueAction::SkipTo(1)))),
        "the band's second row didn't play"
    );
}

#[test]
fn more_like_this_is_asked_for_only_where_it_would_show() {
    // 1100 px with the panel shut: the band has no room for its queue column, so nothing
    // would show a short queue's related songs, and nothing asks for them.
    let scene = Scene::new(1100, 900, true, false).expect("scene");
    let ui = &scene.ui;
    let tracks = Fixtures::load().expect("fixtures").radio().expect("radio");
    let data = ui.global::<HomeData>();
    let frame = ui.global::<Frame>();
    let settle = || {
        // A frame, as the window draws one, then the queued work.
        let _ = ui.window().take_snapshot();
        scene.settle();
    };
    app::on_event(UiEvent::Player(Box::new(radio_state(&tracks[0], &[]))));
    scene.open("home").expect("home");
    ui.show().expect("show");
    settle();
    assert_eq!(
        data.get_related().row_count(),
        0,
        "asked in a narrow window"
    );

    // The Up next panel opens: asked for then.
    frame.set_queue_open(true);
    frame.invoke_queue_toggled(true);
    settle();
    assert!(data.get_related().row_count() > 0, "the panel didn't ask");

    // The next song, the panel shut: nothing again, until the window widens to 1568 px.
    frame.set_queue_open(false);
    app::on_event(UiEvent::Player(Box::new(radio_state(&tracks[1], &[]))));
    settle();
    assert_eq!(
        data.get_related().row_count(),
        0,
        "asked with the panel shut"
    );
    let size = slint::LogicalSize::new(1568.0, 900.0);
    ui.window().set_size(size);
    ui.window().dispatch_event(WindowEvent::Resized { size });
    settle();
    assert!(data.get_related().row_count() > 0, "widening didn't ask");
}
