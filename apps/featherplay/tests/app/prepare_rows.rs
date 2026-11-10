//! The song rows on screen go to the controller to be resolved ahead of a click
//! (`prepare.rs`): a list's top rows once it opens, the rows a scroll settles on, none when
//! another page shows or the window is minimised. Offscreen platform with fixture data.

use std::time::{Duration, Instant};

use featherplay::controller::Action;
use featherplay::snapshot::Scene;
use featherplay::ui::{AlbumData, Frame, HomeData, LibraryData, QueueData, ShelfShape, TrackRow};
use slint::{ComponentHandle, LogicalPosition, Model, ModelRc};

/// Runs timers for `ms`, rendering as the window would.
fn run_for(scene: &Scene, ms: u64) {
    let end = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < end {
        slint::platform::update_timers_and_animations();
        scene.ui.window().take_snapshot().expect("snapshot");
        std::thread::sleep(Duration::from_millis(4));
    }
}

/// The rows of each `Action::Prepare` sent since the last call.
fn prepared(scene: &Scene) -> Vec<Vec<String>> {
    scene
        .take_actions()
        .into_iter()
        .filter_map(|a| match a {
            Action::Prepare(rows) => Some(rows.into_iter().map(|(id, _)| id.0).collect()),
            _ => None,
        })
        .collect()
}

fn ids(rows: ModelRc<TrackRow>) -> Vec<String> {
    rows.iter().map(|r| r.id.to_string()).collect()
}

/// One test: the platform can be installed once per process.
#[test]
fn the_rows_on_screen_are_sent_once_a_list_settles_and_none_off_it() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("library").expect("library");
    scene.settle();
    run_for(&scene, 500);
    let liked = ids(scene.ui.global::<LibraryData>().get_liked());
    let sent = prepared(&scene);
    let top = sent.last().expect("the rows on screen weren't sent");
    assert!(top.len() > 4 && top.len() <= 12, "{top:?}");
    assert_eq!(top[..], liked[..top.len()], "the top first");

    // Scrolled down: once the list stops, the rows it shows now.
    featherplay::offscreen::wheel(scene.ui.window(), LogicalPosition::new(700.0, 500.0), -6.0);
    run_for(&scene, 800);
    let sent = prepared(&scene);
    let scrolled = sent.last().expect("nothing sent after the scroll");
    assert_ne!(scrolled[0], liked[0], "{scrolled:?}");
    assert!(scrolled.iter().all(|id| liked.contains(id)));

    // Another page: what Liked showed stops at once, then the album's rows come.
    scene.open("album").expect("album");
    assert_eq!(prepared(&scene), [Vec::<String>::new()]);
    scene.settle();
    run_for(&scene, 500);
    let album = ids(scene.ui.global::<AlbumData>().get_tracks());
    let sent = prepared(&scene);
    let rows = sent.last().expect("the album's rows weren't sent");
    assert_eq!(rows[..], album[..rows.len()]);

    // Minimised: none; shown again, the same rows.
    featherplay::memory::hidden(&scene.ctx, true);
    assert_eq!(prepared(&scene), [Vec::<String>::new()]);
    featherplay::memory::hidden(&scene.ctx, false);
    assert_eq!(prepared(&scene).last(), Some(rows));

    // Home: the songs of its song grids on screen, those of the grid's first page.
    scene.open("home").expect("home");
    scene.settle();
    run_for(&scene, 500);
    let grids: Vec<String> = scene
        .ui
        .global::<HomeData>()
        .get_shelves()
        .iter()
        .filter(|s| s.shape == ShelfShape::Songs)
        .flat_map(|s| ids(s.songs))
        .collect();
    let sent = prepared(&scene);
    let rows = sent.last().expect("Home's songs weren't sent");
    assert!(
        !rows.is_empty() && rows.iter().all(|id| grids.contains(id)),
        "{rows:?}"
    );
}

/// Up next shows other songs in the same rows as the queue moves on (a long queue): those
/// are sent once it settles, though the rows on screen are the same ones.
#[test]
fn up_next_is_sent_again_when_its_rows_show_other_songs() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("album").expect("album");
    scene.ui.global::<Frame>().set_queue_open(true);
    scene.play_track(false, 0).expect("play");
    scene.settle();
    run_for(&scene, 500);
    let upcoming = || ids(scene.ui.global::<QueueData>().get_upcoming());
    let before = upcoming();
    let sent = prepared(&scene);
    let first = sent.last().expect("nothing sent");
    assert!(first.contains(&before[0]), "{first:?}");

    // The next song plays: as many songs to come, each a row up.
    scene.play_track(false, 1).expect("next");
    scene.settle();
    run_for(&scene, 500);
    let after = upcoming();
    assert_eq!(after.len(), before.len());
    assert_ne!(after[0], before[0]);
    let sent = prepared(&scene);
    let rows = sent.last().expect("Up next's new rows weren't sent");
    assert!(rows.contains(&after[0]), "{rows:?}");
}
