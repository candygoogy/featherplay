//! The sidebar's library chips (Playlists, Albums, Artists) line up with the list below
//! them: they start at the rows' cover column (18 px in), not at the sidebar's edge.
//! Offscreen platform with fixture data; no window is shown.

use featherplay::snapshot::Scene;
use featherplay::ui::SidebarData;
use slint::{ComponentHandle, LogicalPosition};

use crate::support;

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;
/// The chips' middle: Liked songs ends at 268 px (under the row at the window's top), then
/// 12 px of room and the 28 px chips.
const CHIPS_Y: f32 = 294.0;

fn click(scene: &Scene, x: f32) {
    support::click(scene, LogicalPosition::new(x, CHIPS_Y));
    scene.settle();
}

fn chip(scene: &Scene) -> i32 {
    scene.ui.global::<SidebarData>().get_chip()
}

/// One test: the platform can be installed once per process.
#[test]
fn chips_start_at_the_lists_indent() {
    let scene = Scene::new(WIDTH, HEIGHT, true, false).expect("scene");
    scene.open("home").expect("home");
    scene.settle();
    assert_eq!(chip(&scene), 0, "Playlists first");

    // Each chip answers on its label.
    click(&scene, 190.0);
    assert_eq!(chip(&scene), 2, "Artists");
    click(&scene, 120.0);
    assert_eq!(chip(&scene), 1, "Albums");
    click(&scene, 50.0);
    assert_eq!(chip(&scene), 0, "Playlists");

    // Left of the covers' column there's no chip.
    click(&scene, 120.0);
    click(&scene, 14.0);
    assert_eq!(chip(&scene), 1, "a chip reaches past the list's indent");
}
