//! The artist page with the recorded artist (fixtures, offscreen): its rows in the page's
//! order, top songs to the related artists. The videos are song rows with their views; the
//! related artists are round.

use featherplay::snapshot::Scene;
use featherplay::ui::{ArtistData, CardItem, Theme};
use slint::{ComponentHandle, LogicalPosition, Model, ModelRc};

use crate::support::{fixture_covers, label, resize, settle_in_frames, wheel_run};

#[test]
fn the_artist_page_shows_its_rows_in_order() {
    let scene = Scene::new(1280, 800, true, false).expect("scene");
    scene.open("artist").expect("artist");
    scene.settle();
    let titles = [
        "Top songs",
        "Albums",
        "Singles & EPs",
        "Videos",
        "Featured on",
        "Fans might also like",
    ];
    // The page's, not the sidebar's chips ("Albums").
    let side = scene.ui.global::<Theme>().get_sidebar_w();
    let ys: Vec<f32> = titles
        .iter()
        .map(|t| {
            // Laid out, whether or not the page has scrolled to it.
            let found = label::any_where(&scene, |item, l| {
                l == *t && label::rect(item).min_x() > side
            });
            let r = found
                .first()
                .unwrap_or_else(|| panic!("the page shows no {t:?} row"));
            label::rect(r).min_y()
        })
        .collect();
    assert!(
        ys.windows(2).all(|w| w[0] < w[1]),
        "the rows are out of order: {titles:?} at {ys:?}"
    );

    let data = scene.ui.global::<ArtistData>();
    let video = data.get_videos().row_data(0).expect("a video row");
    assert_eq!(video.album, "907M views", "a video row shows its views");
    assert!(data.get_related().iter().all(|a| a.round));
    assert!(data.get_featured().row_count() > 0);
}

/// The card shelves' covers come as their cards come within a screen of the view: in a short
/// window the first shelf has its covers, and the last, far down the page, has none (none
/// asked for) until the page is scrolled to it.
#[test]
fn a_shelf_far_down_the_page_gets_its_covers_once_scrolled_near() {
    fixture_covers();
    let scene = Scene::new(1280, 800, true, true).expect("scene");
    resize(scene.ui.window(), 1280, 400);
    scene.ui.show().expect("show");
    scene.open("artist").expect("artist");
    settle_in_frames(&scene);
    scene.ctx.covers.flush();
    let data = scene.ui.global::<ArtistData>();
    let with_art =
        |cards: ModelRc<CardItem>| cards.iter().filter(|c| c.cover.size().width > 0).count();
    assert!(
        with_art(data.get_albums()) > 0,
        "the albums near the top have no covers"
    );
    assert!(data.get_related().row_count() > 0);
    assert_eq!(
        with_art(data.get_related()),
        0,
        "the related artists' covers came while far below the view"
    );

    let _ = wheel_run(
        scene.ui.window(),
        LogicalPosition::new(800.0, 200.0),
        20,
        -16.0,
    );
    settle_in_frames(&scene);
    scene.ctx.covers.flush();
    assert!(
        with_art(data.get_related()) > 0,
        "the related artists' covers didn't come once scrolled to"
    );
}
