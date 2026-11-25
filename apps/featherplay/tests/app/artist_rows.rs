//! The artist page with the recorded artist (fixtures, offscreen): its rows in the page's
//! order, top songs to the related artists. The videos are song rows with their views; the
//! related artists are round.

use featherplay::snapshot::Scene;
use featherplay::ui::{ArtistData, Theme};
use slint::{ComponentHandle, Model};

use crate::support::label;

/// One test: the platform can be installed once per process.
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
