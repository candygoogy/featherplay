//! A link pasted into the top bar's search field, with fixture data on the offscreen
//! platform (no window is shown, no request goes out): a song link shows that one song on
//! the Search page and plays nothing until it's asked to; a playlist link opens the
//! playlist page. Tracking parameters never reach the page or the field.

use featherplay::controller::Action;
use featherplay::router::Page;
use featherplay::snapshot::Scene;
use featherplay::ui::{Nav, PageKind, SearchData, TopBarData};
use featherplay_core::model::{PlaylistId, TrackId};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, Model, SharedString};

fn tap(w: &slint::Window, text: SharedString) {
    w.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    w.dispatch_event(WindowEvent::KeyReleased { text });
}

/// Ctrl+K into the field, the link in it as a paste leaves it, then Enter.
fn paste_and_enter(scene: &Scene, link: &str) {
    let w = scene.ui.window();
    w.dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    tap(w, "k".into());
    w.dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
    scene.ui.global::<TopBarData>().set_query(link.into());
    tap(w, Key::Return.into());
    scene.settle();
}

fn plays(actions: &[Action]) -> Vec<&Action> {
    actions
        .iter()
        .filter(|a| matches!(a, Action::Play { .. } | Action::Radio(_)))
        .collect()
}

/// One test: the platform can be installed once per process.
#[test]
fn a_song_link_shows_one_song_and_a_playlist_link_opens_it() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("home").expect("home");
    scene.settle();
    scene.take_actions();

    // "Around the World", recorded in the radio fixture.
    paste_and_enter(&scene, "https://youtu.be/Jb6gcoR266U?si=Tr4ck1ngT0ken&t=42");
    let bare = "https://music.youtube.com/watch?v=Jb6gcoR266U";
    assert_eq!(
        scene.ctx.current_page(),
        Page::Search { query: bare.into() }
    );
    assert_eq!(scene.ui.global::<TopBarData>().get_query(), bare);
    let data = scene.ui.global::<SearchData>();
    let songs = data.get_songs();
    assert_eq!(songs.row_count(), 1, "one result for a song link");
    let row = songs.row_data(0).expect("row");
    assert_eq!(row.id, "Jb6gcoR266U");
    assert_eq!(row.title, "Around the World");
    assert!(!row.artist.is_empty() && !row.duration.is_empty());
    assert!(
        plays(&scene.take_actions()).is_empty(),
        "a pasted link started playback"
    );

    // Enter or a click on the row plays it, with its radio, as any search result.
    data.invoke_play(0);
    let sent = scene.take_actions();
    match plays(&sent).as_slice() {
        [
            Action::Play {
                tracks,
                start: 0,
                radio: true,
            },
        ] => assert_eq!(tracks[0].id, TrackId("Jb6gcoR266U".into())),
        other => panic!("expected one Play with radio, got {other:?}"),
    }

    // A playlist link (an album's list here) opens the playlist page.
    let list = "OLAK5uy_nMr9h2VlS-2PULNz3M3XVXQj_P3C2bqaY";
    paste_and_enter(
        &scene,
        &format!("https://music.youtube.com/playlist?list={list}&si=abc"),
    );
    assert_eq!(
        scene.ctx.current_page(),
        Page::Playlist {
            id: PlaylistId(list.into())
        }
    );
    assert_eq!(scene.ui.global::<Nav>().get_page(), PageKind::Playlist);
    assert!(plays(&scene.take_actions()).is_empty());

    // Text that isn't a link searches as typed.
    paste_and_enter(&scene, "daft punk");
    assert_eq!(
        scene.ctx.current_page(),
        Page::Search {
            query: "daft punk".into()
        }
    );
    assert!(scene.ui.global::<SearchData>().get_songs().row_count() > 1);
}
