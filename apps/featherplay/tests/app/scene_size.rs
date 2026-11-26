//! A scene made at a size lays out at it from the start, shown or not: the offscreen window
//! opens at 1280 × 800, and `set_size` alone kept the page at that until a resize arrived.

use featherplay::snapshot::Scene;
use i_slint_core::window::WindowInner;
use slint::ComponentHandle;

use crate::support::label::ItemRc;

#[test]
fn a_scene_lays_out_at_the_size_it_was_made_at() {
    let scene = Scene::new(1600, 900, true, false).expect("scene");
    scene.open("home").expect("home");
    scene.settle();
    let w = scene.ui.window();
    assert_eq!(w.scale_factor(), 1.0);
    let root = ItemRc::new_root(WindowInner::from_pub(w).component()).geometry();
    assert_eq!((root.width(), root.height()), (1600.0, 900.0));
    // The page beside the sidebar is wider than a 1280 px window could hold.
    let page = scene.ui.get_page_width();
    assert!(page > 1280.0, "the page is {page} px wide at 1600");
}
