//! Picking and moving songs on one of the account's own playlists, on the offscreen platform
//! with fixture data (no window is shown, nothing goes out): clicks pick rows (Ctrl adds,
//! Shift a range, Esc lets go), Alt+Down moves the picked rows together, and a write that
//! fails puts the list back with an error toast. Someone else's playlist picks nothing.

use featherplay::snapshot::Scene;
use featherplay::ui::{PlaylistData, Toast, ToastTone};
use featherplay_controller::catalog::Source;
use featherplay_controller::place::Place;
use featherplay_core::model::PlaylistId;
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, Model, SharedString};

use crate::support::label;

/// The fixtures' own playlist (the library's first), its keys in the window.
fn own_playlist() -> Scene {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("playlist").expect("playlist");
    scene.settle();
    scene
        .ui
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    assert!(scene.ui.global::<PlaylistData>().get_editable());
    scene
}

/// Row `i`'s accessible label.
fn row_label(scene: &Scene, i: usize) -> String {
    let row = scene
        .ui
        .global::<PlaylistData>()
        .get_tracks()
        .row_data(i)
        .expect("row");
    format!("{}, {}", row.title, row.artist)
}

/// Presses `keys` together, then lets them go.
fn keys(scene: &Scene, keys: &[SharedString]) {
    let w = scene.ui.window();
    for k in keys {
        w.dispatch_event(WindowEvent::KeyPressed { text: k.clone() });
    }
    for k in keys.iter().rev() {
        w.dispatch_event(WindowEvent::KeyReleased { text: k.clone() });
    }
}

/// Clicks row `i`, with `held` held.
fn click_row(scene: &Scene, i: usize, held: Option<Key>) {
    let at = label::centre(scene, &row_label(scene, i));
    let w = scene.ui.window();
    if let Some(k) = held {
        w.dispatch_event(WindowEvent::KeyPressed { text: k.into() });
    }
    crate::support::click(scene, at);
    if let Some(k) = held {
        w.dispatch_event(WindowEvent::KeyReleased { text: k.into() });
    }
}

/// The places of the picked rows.
fn picked(scene: &Scene) -> Vec<usize> {
    let tracks = scene.ui.global::<PlaylistData>().get_tracks();
    (0..tracks.row_count())
        .filter(|&i| tracks.row_data(i).is_some_and(|r| r.selected))
        .collect()
}

/// The ids of the rows, in order.
fn ids(scene: &Scene) -> Vec<String> {
    let tracks = scene.ui.global::<PlaylistData>().get_tracks();
    tracks.iter().map(|r| r.id.to_string()).collect()
}

#[test]
fn clicks_pick_rows_ctrl_adds_shift_picks_a_range_and_esc_lets_go() {
    let scene = own_playlist();
    click_row(&scene, 1, None);
    assert_eq!(picked(&scene), [1]);
    click_row(&scene, 3, Some(Key::Control));
    assert_eq!(picked(&scene), [1, 3]);
    click_row(&scene, 1, Some(Key::Control));
    assert_eq!(picked(&scene), [3], "Ctrl+click lets a picked row go");
    click_row(&scene, 5, Some(Key::Shift));
    assert_eq!(picked(&scene), [1, 2, 3, 4, 5], "from the row clicked last");
    assert!(
        label::find(&scene, &row_label(&scene, 4)).is_some_and(|r| r.accessible_string_property(
            i_slint_core::accessibility::AccessibleStringProperty::ItemSelected
        ) == Some("true".into())),
        "a picked row says it's selected"
    );
    keys(&scene, &[Key::Escape.into()]);
    assert_eq!(picked(&scene), Vec::<usize>::new());

    // Someone else's playlist (the recorded mix) picks nothing.
    scene.ctx.navigate(
        Place::Playlist(PlaylistId(
            "RDTMAK5uy_FIXTURE_MIX_100000000000000000000".into(),
        ))
        .into(),
    );
    scene.settle();
    assert!(!scene.ui.global::<PlaylistData>().get_editable());
    click_row(&scene, 1, None);
    assert_eq!(picked(&scene), Vec::<usize>::new());
}

#[test]
fn alt_down_moves_the_picked_rows_and_a_failed_write_puts_them_back() {
    let scene = own_playlist();
    let before = ids(&scene);
    click_row(&scene, 0, None);
    click_row(&scene, 2, Some(Key::Control));
    keys(&scene, &[Key::Alt.into(), Key::DownArrow.into()]);
    scene.settle();
    // Together, in their order, after the row that was below the last of them.
    let mut moved = before.clone();
    let (a, b) = (moved.remove(0), moved.remove(1));
    moved.splice(2..2, [a, b]);
    assert_eq!(ids(&scene), moved);
    assert_eq!(picked(&scene), [2, 3], "the rows stay picked");
    let toast = scene.ui.global::<Toast>();
    assert_eq!(toast.get_text().as_str(), "", "the fixtures took the write");

    // The service can't be reached: the list moves, then goes back, and a toast says why.
    scene.ctx.catalog.set(Source::Unreachable);
    keys(&scene, &[Key::Alt.into(), Key::Home.into()]);
    assert_ne!(ids(&scene), moved, "the list changes at once");
    scene.settle();
    assert_eq!(ids(&scene), moved);
    assert_eq!(picked(&scene), [2, 3]);
    assert_eq!(
        toast.get_text().as_str(),
        "Couldn't reorder the playlist. Check the connection, then try again."
    );
    assert_eq!(toast.get_tone(), ToastTone::Error);
}
