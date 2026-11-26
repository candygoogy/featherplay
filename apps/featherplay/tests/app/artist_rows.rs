//! The artist page with the recorded artist (fixtures, offscreen): its rows in the page's
//! order, top songs to the related artists. The videos are song rows with their views; the
//! related artists are round.

use std::ops::ControlFlow;

use featherplay::snapshot::Scene;
use featherplay::ui::{ArtistData, CardItem, Theme};
use i_slint_core::item_tree::ItemRc;
use i_slint_core::items::Layer;
use i_slint_core::window::WindowInner;
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

/// Only the cards on a shelf's page take a cover layer, as on Explore: Slint makes a layer even
/// where the rail's clip hides it, so every card of the artist page's rails held one (~0.4 MB
/// at 150 %) and drew it as its shelf came into view. At 1700 px the cards are wider than the
/// 200 px their covers are decoded at, so the covers on the page draw through layers.
#[test]
fn only_the_cards_on_a_shelfs_page_take_a_cover_layer() {
    fixture_covers();
    let (w, h) = (1700, 2400);
    let scene = Scene::new(w, h, true, true).expect("scene");
    resize(scene.ui.window(), w, h);
    scene.ui.show().expect("show");
    scene.open("artist").expect("artist");
    settle_in_frames(&scene);
    scene.ctx.covers.flush();
    settle_in_frames(&scene);
    let data = scene.ui.global::<ArtistData>();
    assert!(
        data.get_albums().row_count() > 5,
        "the fixture's albums fit on one page: nothing to check"
    );
    let (mut on, mut off) = (0, Vec::new());
    let root = ItemRc::new_root(WindowInner::from_pub(scene.ui.window()).component());
    root.visit_descendants(|item| {
        if let Some(layer) = item.downcast::<Layer>()
            && layer.as_pin_ref().cache_rendering_hint()
        {
            let r = label::rect(item);
            if r.max_x() <= w as f32 + 0.5 {
                on += 1;
            } else {
                off.push(r.min_x());
            }
        }
        ControlFlow::<()>::Continue(())
    });
    assert!(on > 0, "no cover on a shelf's page took a layer");
    assert!(
        off.is_empty(),
        "{} covers off their shelf's page took a layer, at x {off:?}",
        off.len()
    );
}

/// The banner's photo is made in Rust for the banner's physical size, the gradient baked in,
/// and made again when a resize changes that size: it's drawn as it is, never scaled.
#[test]
fn the_banner_photo_is_made_for_its_size_and_again_after_a_resize() {
    fixture_covers();
    let scene = Scene::new(1600, 1000, true, true).expect("scene");
    let w = scene.ui.window();
    resize(w, 1600, 1000);
    scene.ui.show().expect("show");
    scene.open("artist").expect("artist");
    settle_in_frames(&scene);
    let data = scene.ui.global::<ArtistData>();
    assert!(data.get_has_photo(), "the fixture artist has no photo");
    let before = data.get_banner().size();
    assert!(before.width > 0, "no banner photo");

    // The page, and so the banner, 300 px narrower (at 1.0, logical is physical).
    resize(w, 1300, 900);
    settle_in_frames(&scene);
    let after = data.get_banner().size();
    assert_eq!(
        before.width - after.width,
        300,
        "the photo wasn't made again for the narrower banner: {before:?} → {after:?}"
    );
    assert!(after.height <= before.height);
}
