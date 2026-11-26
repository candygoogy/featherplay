//! Home's Shortcuts covers are decoded at the tile size the grid lays out (104 px beside the
//! card, 64 under it), on the offscreen platform with art on every cover. Across a change the
//! tiles keep their covers, drawn scaled, until the new size's replace them; a resize that
//! keeps the size decodes nothing.

use featherplay::offscreen;
use featherplay::snapshot::Scene;
use featherplay::ui::{HomeData, Theme};
use slint::{ComponentHandle, Model};

use crate::support::{fixture_covers, frames, label, resize, settle_in_frames};

const H: u32 = 1000;

/// The shown tiles' height: the grid's tile size.
fn tile(scene: &Scene) -> u32 {
    let data = scene.ui.global::<HomeData>();
    let title = data.get_shortcuts().row_data(1).expect("a shortcut").title;
    let side = scene.ui.global::<Theme>().get_sidebar_w();
    let found = label::shown_where(scene, |item, l| {
        l == title.as_str() && label::rect(item).min_x() > side
    });
    let r = label::rect(found.first().unwrap_or_else(|| panic!("no {title:?} tile")));
    r.height().round() as u32
}

/// The shortcuts' cover widths, Liked songs (drawn, not a cover) left out.
fn covers(scene: &Scene) -> Vec<u32> {
    let data = scene.ui.global::<HomeData>();
    data.get_shortcuts()
        .iter()
        .filter(|c| c.id != "LM")
        .map(|c| c.cover.size().width)
        .collect()
}

/// Frames until every shortcut's cover is `px` wide, checking none goes blank on the way.
fn swap_to(scene: &Scene, px: u32) {
    let w = scene.ui.window();
    for _ in 0..600 {
        let now = covers(scene);
        assert!(
            now.iter().all(|&c| c > 0),
            "a tile went blank across the step: {now:?}"
        );
        if now.iter().all(|&c| c == px) {
            return;
        }
        scene.settle_threads();
        frames(w, 1, |_| {});
    }
    panic!("the covers aren't {px} px: {:?}", covers(scene));
}

#[test]
fn shortcut_covers_follow_the_tile_size_and_never_go_blank() {
    fixture_covers();
    let scene = Scene::new(1280, 800, false, true).expect("scene");
    scene.ui.global::<Theme>().set_motion(true);
    let w = scene.ui.window();
    scene.open("home").expect("home");
    scene.theme(true);
    scene.ui.show().expect("show");
    scene.settle();
    offscreen::freeze_clock();
    resize(w, 1568, H);
    settle_in_frames(&scene);

    // Beside the card, the 104 px tiles (the fixtures' 21 shortcuts, most behind "N more").
    assert_eq!(tile(&scene), 104);
    assert!(
        covers(&scene).len() > 10,
        "the fixtures have shortcuts with art"
    );
    swap_to(&scene, 104);

    // A page too narrow for the row: the 64 px tiles under the card, and back.
    resize(w, 900, H);
    frames(w, 1, |_| {});
    assert_eq!(tile(&scene), 64);
    swap_to(&scene, 64);
    resize(w, 1568, H);
    frames(w, 1, |_| {});
    assert_eq!(tile(&scene), 104);
    swap_to(&scene, 104);

    // Wider at the same size: nothing is decoded again.
    resize(w, 1700, H);
    frames(w, 2, |_| {});
    assert_eq!(tile(&scene), 104);
    assert_eq!(
        featherplay::busy::pending(),
        0,
        "covers were asked for again"
    );
    assert!(covers(&scene).iter().all(|&c| c == 104));
}
