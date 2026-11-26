//! Home's Now playing card (the band), on the offscreen platform (no window is shown): its
//! toggle resumes and pauses in place, without the row moving, and Play plays the pick on a
//! fresh start; the "From" line opens where the song plays from; its round actions act on the
//! card's song; a click on the card opens Now playing, on its artists their page; the "Next"
//! line opens the Up next panel, and from 760 px a column of two of the queue's songs, where
//! the row leaves the card one of three, show at its right end with "Show all"; the card's
//! queue follows the player's, the Up next panel open or not; "More like this" under a short
//! queue, asked for only where something would show it.

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
use slint::platform::PointerEventButton;
use slint::{ComponentHandle, LogicalPosition, Model, ModelRc, VecModel};

use crate::support::{self, label};

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
    support::click_with(w, LogicalPosition::new(x, y), button);
    // Away, so the next click's hover starts fresh.
    support::point(w, LogicalPosition::new(4.0, 4.0));
}

/// Clicks the middle of the band's element labelled `name` with `button`: the shown one
/// above the bar, which has a Play of its own.
fn band_click(ui: &AppWindow, name: &str, button: PointerEventButton) {
    let w = ui.window();
    let bar_top = w.size().height as f32 / w.scale_factor() - ui.global::<Theme>().get_bar_clear();
    let found = label::shown_where(w, |item, l| {
        l == name && label::rect(item).max_y() <= bar_top
    });
    let c = label::rect(
        found
            .first()
            .unwrap_or_else(|| panic!("no {name:?} in the band")),
    )
    .center();
    click(w, c.x, c.y, button);
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

#[test]
fn the_bands_pill_follows_the_session_and_its_actions_act_on_the_song() {
    let _pump = offscreen::install().expect("platform");
    let ui = window(1568.0);
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

    // Paused: Resume; playing: Pause, in its place. Playing from a playlist: the line under
    // the song opens it.
    data.set_resume(true);
    data.set_track(row("t1"));
    queue.set_playing_from("Daily mix".into());
    data.set_playing_kind(ItemKind::Playlist);
    data.set_playing_id("PL1".into());
    band_click(&ui, "Resume", PointerEventButton::Left);
    player.set_playing(true);
    band_click(&ui, "Pause", PointerEventButton::Left);
    band_click(&ui, "From Daily mix", PointerEventButton::Left);
    // A fresh start: Play on the featured song.
    data.set_resume(false);
    data.set_has_featured(true);
    data.set_featured(CardItem {
        id: "t1".into(),
        kind: ItemKind::Song,
        title: "Title t1".into(),
        ..Default::default()
    });
    band_click(&ui, "Play", PointerEventButton::Left);
    assert_eq!(
        *did.borrow(),
        ["resume", "resume", "open source", "play featured"],
        "the band's toggle, line and Play"
    );

    // Playing a radio (no page of its own): the line still says so, and opens Up next.
    did.borrow_mut().clear();
    data.set_resume(true);
    queue.set_playing_from("A radio".into());
    data.set_playing_kind(ItemKind::Song);
    data.set_playing_id("".into());
    band_click(&ui, "From A radio", PointerEventButton::Left);
    assert_eq!(*did.borrow(), ["open source"], "a radio's line");
    did.borrow_mut().clear();
    band_click(&ui, "Start radio", PointerEventButton::Left);
    band_click(&ui, "Go to artist", PointerEventButton::Left);
    assert_eq!(*did.borrow(), ["radio t1", "artist UCt1"], "the actions");
    // Add to playlist opens the song menu on its playlists.
    menu.set_signed_in(true);
    band_click(&ui, "Add to playlist", PointerEventButton::Left);
    assert!(
        menu.get_open() && menu.get_choosing(),
        "no playlist chooser"
    );
    assert_eq!(menu.get_track().id, "t1");
}

#[test]
fn the_next_line_and_the_up_next_column_open_the_panel_and_play() {
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
    let d = did.clone();
    data.on_play_related(move |i| d.borrow_mut().push(format!("related {i}")));
    data.set_resume(true);
    data.set_track(row("t1"));
    data.set_next(rows(3));
    queue.set_upcoming(rows(12));

    // At 800 px the card is under 760 px: one line of what's next, which opens the panel.
    support::resize(w, 800, 900);
    band_click(&ui, "Next Title q0 · Artist", PointerEventButton::Left);
    assert_eq!(*did.borrow(), ["show queue"], "the line");

    // At 1568 px (a 960 px card) two of them at its top right, and "Show all 12".
    did.borrow_mut().clear();
    support::resize(w, 1568, 900);
    assert!(
        label::shown_where(w, |_, l| l == "Title q2, Artist").is_empty(),
        "a third row in the small column"
    );
    band_click(&ui, "Title q1, Artist", PointerEventButton::Left);
    band_click(&ui, "Show all 12", PointerEventButton::Left);
    assert_eq!(*did.borrow(), ["play 1", "show queue"], "the small column");

    // At 1920 px it has its Up next column: the rows, and "Show all 12" at its top.
    did.borrow_mut().clear();
    support::resize(w, 1920, 900);
    band_click(&ui, "Title q1, Artist", PointerEventButton::Left);
    band_click(&ui, "Show all 12", PointerEventButton::Left);
    assert_eq!(*did.borrow(), ["play 1", "show queue"], "the column");
    // A right-click on a row opens the song menu on it.
    band_click(&ui, "Title q0, Artist", PointerEventButton::Right);
    assert!(menu.get_open());
    assert_eq!(menu.get_track().id, "q0");
    menu.invoke_close();

    // One song left: "More like this" fills the column under it.
    did.borrow_mut().clear();
    data.set_next(rows(1));
    queue.set_upcoming(rows(1));
    let related: Vec<TrackRow> = (0..2).map(|i| row(&format!("r{i}"))).collect();
    data.set_related(ModelRc::new(VecModel::from(related)));
    band_click(&ui, "Title r1, Artist", PointerEventButton::Left);
    assert_eq!(*did.borrow(), ["related 1"], "more like this");
}

/// The controller's state as a radio has it: `current` playing, `upcoming` after it.
fn radio_state(current: &Track, upcoming: &[Track]) -> PlayerState {
    PlayerState {
        current: Some(current.clone()),
        upcoming: upcoming.into(),
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
        crossfade: None,
    }
}

#[test]
fn the_bands_queue_follows_the_players_with_the_panel_open() {
    // 2000 px with the 400 px Up next panel open: the card shows no queue, but keeps the
    // player's for its "Next" line and its column.
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
    // The panel closed in a 2560 px window: the card's column, whose second row plays.
    ui.global::<Frame>().set_queue_open(false);
    support::resize(ui.window(), 2560, 1440);
    scene.settle();
    let _ = scene.take_actions();
    let second = data.get_next().row_data(1).expect("a second row");
    band_click(
        ui,
        &format!("{}, {}", second.title, second.artist),
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
    // 1100 px with the panel shut: the card has no room for its queue column, so nothing
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

    // The next song, the panel shut: nothing again, until the window widens to 2560 px.
    frame.set_queue_open(false);
    app::on_event(UiEvent::Player(Box::new(radio_state(&tracks[1], &[]))));
    settle();
    assert_eq!(
        data.get_related().row_count(),
        0,
        "asked with the panel shut"
    );
    support::resize(ui.window(), 2560, 1440);
    settle();
    assert!(data.get_related().row_count() > 0, "widening didn't ask");
}

#[test]
fn the_band_follows_the_player_only_while_home_shows_and_catches_up_on_opening() {
    // Every fixture cover has art.
    crate::support::fixture_covers();
    let scene = Scene::new(1600, 900, true, false).expect("scene");
    let ui = &scene.ui;
    let tracks = Fixtures::load().expect("fixtures").radio().expect("radio");
    let data = ui.global::<HomeData>();
    let play = |i: usize| {
        let state = radio_state(&tracks[i], &tracks[i + 1..i + 4]);
        app::on_event(UiEvent::Player(Box::new(state)));
    };
    play(0);
    scene.open("home").expect("home");
    ui.show().expect("show");
    scene.settle();
    assert_eq!(data.get_track().id.as_str(), tracks[0].id.0);

    // Songs change on another page: the band is left as it was, nothing loaded for it.
    scene.open("settings").expect("settings");
    scene.settle();
    play(1);
    play(2);
    scene.settle();
    assert_eq!(
        data.get_track().id.as_str(),
        tracks[0].id.0,
        "the band followed the player off Home"
    );

    // Home again: the band shows the song playing now and its queue at once, then its cover.
    scene.open("home").expect("home");
    assert_eq!(data.get_track().id.as_str(), tracks[2].id.0);
    assert_eq!(
        data.get_next().row_data(0).map(|r| r.id.to_string()),
        Some(tracks[3].id.0.clone())
    );
    scene.settle();
    assert!(
        tracks[2].thumbnail.is_none() || data.get_hero_cover().size().width > 0,
        "the band's cover didn't come"
    );
}

/// The left edges of the band's shown elements labelled `names`, in order.
fn band_xs(ui: &AppWindow, names: &[&str]) -> Vec<f32> {
    let w = ui.window();
    let bar_top = w.size().height as f32 / w.scale_factor() - ui.global::<Theme>().get_bar_clear();
    names
        .iter()
        .map(|name| {
            let found = label::shown_where(w, |item, l| {
                l == *name && label::rect(item).max_y() <= bar_top
            });
            label::rect(
                found
                    .first()
                    .unwrap_or_else(|| panic!("no {name:?} in the band")),
            )
            .min_x()
        })
        .collect()
}

#[test]
fn resume_turns_to_pause_in_place_and_the_row_stays() {
    let _pump = offscreen::install().expect("platform");
    let rounds = ["Like", "Add to playlist", "Start radio", "Go to artist"];
    // A 960 px card, one under 760 px and a slim one (the toggle a round there).
    for width in [1568.0, 800.0, 600.0] {
        let ui = window(width);
        let data = ui.global::<HomeData>();
        let player = ui.global::<Player>();
        data.set_resume(true);
        data.set_track(row("t1"));
        player.set_has_track(true);
        player.set_title("Title t1".into());
        player.set_total("2:41".into());
        player.set_elapsed("0:58".into());
        // The page settled at its width first.
        support::run_for(ui.window(), 400);
        let paused = band_xs(&ui, &[&["Resume"][..], &rounds[..]].concat());
        player.set_playing(true);
        support::run_for(ui.window(), 400);
        let playing = band_xs(&ui, &[&["Pause"][..], &rounds[..]].concat());
        assert_eq!(paused, playing, "the row moved at {width} px");
    }
}

#[test]
fn the_card_opens_now_playing_and_its_artists_the_artist_page() {
    let _pump = offscreen::install().expect("platform");
    let ui = window(1568.0);
    let did = Rc::new(RefCell::new(Vec::<String>::new()));
    let nav = ui.global::<Nav>();
    let d = did.clone();
    nav.on_toggle_now_playing(move || d.borrow_mut().push("now playing".into()));
    let d = did.clone();
    nav.on_open_artist(move |id| d.borrow_mut().push(format!("artist {id}")));
    let data = ui.global::<HomeData>();
    let player = ui.global::<Player>();
    data.set_resume(true);
    data.set_track(row("t1"));
    player.set_has_track(true);
    player.set_title("Title t1".into());
    player.set_artist("Isabel LaRosa".into());

    // The artists: their page. The card, outside its controls (here its art): Now playing.
    band_click(&ui, "Isabel LaRosa", PointerEventButton::Left);
    let w = ui.window();
    let bar_top = w.size().height as f32 / w.scale_factor() - ui.global::<Theme>().get_bar_clear();
    let card = label::shown_where(w, |item, l| {
        l == "Now playing: Title t1" && label::rect(item).max_y() <= bar_top
    });
    let r = label::rect(card.first().expect("the card is a button"));
    click(w, r.min_x() + 40.0, r.center().y, PointerEventButton::Left);
    assert_eq!(*did.borrow(), ["artist UCt1", "now playing"]);
}
