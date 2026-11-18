//! Home's Shortcuts row, on the offscreen platform with fixture data (no window is shown,
//! nothing goes out): Liked songs leads it; a right-click on a suggestion opens its menu, and
//! Pin makes it a pin, in the row and the sidebar alike; Remove from Shortcuts takes a
//! suggestion out, and the toast's Undo brings it back.

use featherplay::snapshot::Scene;
use featherplay::ui::{HomeData, ShortcutMenu, SidebarData, Theme, Toast};
use slint::platform::PointerEventButton;
use slint::{ComponentHandle, LogicalPosition, Model};

use crate::support::{click_with, label};

const WIDTH: u32 = 1280;
const HEIGHT: u32 = 800;

/// The row's tiles' titles, Liked songs first, a pin as `*title`.
fn tiles(scene: &Scene) -> Vec<String> {
    let data = scene.ui.global::<HomeData>();
    let pins = data.get_shortcut_pins();
    data.get_shortcuts()
        .iter()
        .enumerate()
        .map(|(i, it)| {
            let pinned = pins.row_data(i).unwrap_or(false);
            format!("{}{}", if pinned { "*" } else { "" }, it.title)
        })
        .collect()
}

/// Right-clicks the Shortcuts tile labelled `title` (not the sidebar's entry of that name),
/// on its text.
fn right_click(scene: &Scene, title: &str) {
    let side = scene.ui.global::<Theme>().get_sidebar_w();
    let found = label::shown_where(scene, |item, l| {
        l == title && label::rect(item).min_x() > side
    });
    let r = label::rect(
        found
            .first()
            .unwrap_or_else(|| panic!("no {title:?} tile in Shortcuts")),
    );
    let at = LogicalPosition::new(r.min_x() + 100.0, r.center().y);
    click_with(scene, at, PointerEventButton::Right);
}

/// One test: the platform can be installed once per process.
#[test]
fn a_suggestion_pins_and_is_removed_and_comes_back() {
    let scene = Scene::new(WIDTH, HEIGHT, true, false).expect("scene");
    scene.open("home").expect("home");
    scene.settle();
    let ui = &scene.ui;
    let data = ui.global::<HomeData>();
    assert!(data.get_shortcuts_shown(), "no Shortcuts row signed in");
    let shown = tiles(&scene);
    assert_eq!(shown.first().map(String::as_str), Some("Liked songs"));
    assert!(shown.len() > 2, "the library's playlists aren't suggested");
    let first = shown[1].clone();
    let second = shown[2].clone();

    // Right-click > Pin: the first suggestion is a pin now, here and in the sidebar.
    right_click(&scene, &first);
    let menu = ui.global::<ShortcutMenu>();
    assert!(menu.get_open(), "a right-click didn't open the tile's menu");
    assert!(!menu.get_pinned());
    label::click(&scene, "Pin");
    assert!(!menu.get_open());
    assert_eq!(tiles(&scene)[1], format!("*{first}"));
    assert_eq!(
        ui.global::<Toast>().get_text(),
        format!("Pinned \u{201C}{first}\u{201D}")
    );
    let sidebar = ui.global::<SidebarData>();
    assert_eq!(
        sidebar.get_items().row_data(0).expect("an entry").title,
        first
    );
    assert_eq!(sidebar.get_pinned().row_data(0), Some(true));

    // Remove from Shortcuts takes the next suggestion out; Undo puts it back.
    right_click(&scene, &second);
    assert!(menu.get_open());
    label::click(&scene, "Remove from Shortcuts");
    assert!(!tiles(&scene).contains(&second), "{second:?} still shows");
    assert_eq!(
        ui.global::<Toast>().get_text(),
        format!("Removed {second} from Shortcuts")
    );
    label::click(&scene, "Undo");
    assert_eq!(tiles(&scene)[2], second, "Undo didn't bring it back");
}
