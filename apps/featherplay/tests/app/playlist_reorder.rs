//! Picking and moving songs on one of the account's own playlists, on the offscreen platform
//! with fixture data (no window is shown, nothing goes out): clicks pick rows (Ctrl adds,
//! Shift a range, Esc lets go), Alt+Down moves the picked rows together, and a write that
//! fails puts the list back with an error toast. Someone else's playlist picks nothing.
//! Dragging a row moves it, or the rows picked with it, to the gap it's dropped in; Esc ends
//! a drag where it started, and held near the list's bottom a drag scrolls the list.

use featherplay::snapshot::Scene;
use featherplay::ui::{PlaylistData, Theme, Toast, ToastTone};
use featherplay_controller::catalog::Source;
use featherplay_controller::place::Place;
use featherplay_core::model::PlaylistId;
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model, SharedString};

use crate::support::{self, label};

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

/// What the page has of the rows picked: how many, the first and the last (-1 for none).
fn told(scene: &Scene) -> (i32, i32, i32) {
    let data = scene.ui.global::<PlaylistData>();
    (
        data.get_picked_count(),
        data.get_picked_first(),
        data.get_picked_last(),
    )
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
    assert_eq!(told(&scene), (2, 0, 2));
    keys(&scene, &[Key::Alt.into(), Key::DownArrow.into()]);
    scene.settle();
    assert_eq!(told(&scene), (2, 2, 3), "the page follows the rows moved");
    let data = scene.ui.global::<PlaylistData>();
    assert_eq!((data.get_reveal_first(), data.get_reveal_last()), (2, 3));
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

/// Row `i`'s middle, in the window.
fn row_at(scene: &Scene, i: usize) -> LogicalPosition {
    label::centre(scene, &row_label(scene, i))
}

/// Presses at `from`, drags to `to` in two moves (the first past the 4 px start).
fn drag_to(scene: &Scene, from: LogicalPosition, to: LogicalPosition) {
    support::press(scene, from);
    support::point(scene, LogicalPosition::new(from.x, from.y + 8.0));
    support::point(scene, to);
}

#[test]
fn dragging_a_row_or_the_rows_picked_moves_them_to_the_gap_and_esc_cancels() {
    let scene = own_playlist();
    let data = scene.ui.global::<PlaylistData>();
    let before = ids(&scene);

    // One row, not picked, dropped on row 3: it's picked alone and lands below it (it came
    // from above, as in Up next).
    let on = row_at(&scene, 3);
    drag_to(&scene, row_at(&scene, 0), on);
    assert!(data.get_dragging());
    support::release(&scene, on);
    scene.settle();
    let mut one = before.clone();
    let first = one.remove(0);
    one.insert(3, first);
    assert_eq!(ids(&scene), one);
    assert_eq!(picked(&scene), [3]);
    assert!(!data.get_dragging());

    // Two picked, dragged by the second onto the first row: both go above it, in their
    // order, and the raised copy says how many.
    click_row(&scene, 1, None);
    click_row(&scene, 2, Some(Key::Control));
    let on = row_at(&scene, 0);
    drag_to(&scene, row_at(&scene, 2), on);
    assert!(label::shows(&scene, "2 songs"), "the dragged copy's badge");
    support::release(&scene, on);
    scene.settle();
    let mut two = one.clone();
    let moved: Vec<String> = two.drain(1..3).collect();
    two.splice(0..0, moved);
    assert_eq!(ids(&scene), two);
    assert_eq!(picked(&scene), [0, 1]);

    // Esc during a drag: it ends, and the release moves nothing or picks nothing new.
    let on = row_at(&scene, 6);
    drag_to(&scene, row_at(&scene, 4), on);
    keys(&scene, &[Key::Escape.into()]);
    assert!(!data.get_dragging());
    support::release(&scene, on);
    scene.settle();
    assert_eq!(ids(&scene), two);
    assert_eq!(picked(&scene), [4]);
}

#[test]
fn a_drag_held_in_the_bottom_band_scrolls_the_list() {
    let scene = own_playlist();
    let w = scene.ui.window();
    let height = w.size().to_logical(w.scale_factor()).height;
    let t = scene.ui.global::<Theme>();
    let top = row_at(&scene, 3).y;
    // In the band above the floating bar.
    let band = LogicalPosition::new(
        row_at(&scene, 0).x,
        height - t.get_bar_h() - t.get_bar_inset() - 20.0,
    );
    drag_to(&scene, row_at(&scene, 0), band);
    support::run_for(w, 300);
    let scrolled = top - row_at(&scene, 3).y;
    assert!(scrolled > 20.0, "the list scrolled {scrolled} px");
    // Held on, it stops at the end of the list.
    support::run_for(w, 3000);
    let end = row_at(&scene, 3).y;
    support::run_for(w, 300);
    assert_eq!(row_at(&scene, 3).y, end, "it went past the end");
    support::release(&scene, band);
    scene.settle();
    assert_eq!(picked(&scene).len(), 1);
}

#[test]
fn ctrl_a_picks_every_song_and_a_click_on_empty_space_lets_them_go() {
    let scene = own_playlist();
    keys(&scene, &[Key::Control.into(), "a".into()]);
    let n = ids(&scene).len();
    assert_eq!(picked(&scene), (0..n).collect::<Vec<_>>());
    assert_eq!(told(&scene), (n as i32, 0, n as i32 - 1));
    // Beside the rows, in the page's margin.
    let row = label::find(&scene, &row_label(&scene, 1)).expect("row");
    let left = label::rect(&row).origin.x;
    support::click(
        &scene,
        LogicalPosition::new(left - 8.0, row_at(&scene, 1).y),
    );
    assert_eq!(picked(&scene), Vec::<usize>::new());
    assert_eq!(told(&scene), (0, -1, -1));
}
