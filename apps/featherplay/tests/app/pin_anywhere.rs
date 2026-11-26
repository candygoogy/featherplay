//! Pin from wherever a playlist, album or artist shows, on the offscreen platform with
//! fixture data (no window is shown, nothing goes out): a right-click on a Home album tile
//! opens its menu and Pin puts the album in Shortcuts; a song tile opens the song menu; the
//! playlist page's ⋯ (the account's own: the sidebar's menu) pins the playlist; the hint
//! tile names what to right-click.

use featherplay::snapshot::Scene;
use featherplay::ui::{HomeData, ItemKind, ItemMenu, PlaylistMenu, ShelfShape, SongMenu, Toast};
use i_slint_core::items::AccessibleRole;
use slint::platform::PointerEventButton;
use slint::{ComponentHandle, LogicalPosition, Model, ModelRc, VecModel};

use crate::support::{click_with, frames, home_at, label, pixels};

const WIDTH: u32 = 1280;
// Tall enough for the feed's last rail, its albums, under the band, Shortcuts and the rest.
const HEIGHT: u32 = 2600;

/// The row's tiles' titles, a pin as `*title`.
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

/// The first tile of `kind` on Home's rails whose card shows whole in the window (not its
/// 64 px Shortcuts tile), with where its art's middle is.
fn shelf_tile(scene: &Scene, kind: ItemKind) -> (String, LogicalPosition) {
    let shelves = scene.ui.global::<HomeData>().get_shelves();
    shelves
        .iter()
        .filter(|s| s.shape != ShelfShape::Songs)
        .flat_map(|s| s.items.iter().collect::<Vec<_>>())
        .filter(|it| it.kind == kind)
        .find_map(|it| {
            let found = label::shown_where(scene, |item, l| {
                let r = label::rect(item);
                l == it.title.as_str()
                    && item.accessible_role() == AccessibleRole::Button
                    && r.height() > 100.0
                    && r.min_y() > 0.0
                    && r.max_y() < HEIGHT as f32 - 120.0
                    && r.max_x() < WIDTH as f32 - 400.0
            });
            let r = label::rect(found.first()?);
            let at = LogicalPosition::new(r.center().x, r.min_y() + r.width() / 2.0);
            Some((it.title.to_string(), at))
        })
        .unwrap_or_else(|| panic!("no whole {kind:?} tile on Home"))
}

/// One test: the platform can be installed once per process.
#[test]
fn a_tile_and_the_playlist_page_pin_and_the_hint_says_how() {
    let scene = home_at(WIDTH, HEIGHT, 1.0);
    let ui = &scene.ui;
    let w = ui.window();
    let menu = ui.global::<ItemMenu>();

    // A song tile: the song menu, as song rows have it.
    let (song, at) = shelf_tile(&scene, ItemKind::Song);
    click_with(&scene, at, PointerEventButton::Right);
    let songs = ui.global::<SongMenu>();
    assert!(
        songs.get_open(),
        "a song tile's right-click didn't open the song menu"
    );
    assert_eq!(songs.get_track().title, song);
    assert!(!menu.get_open());
    songs.invoke_close();
    let _ = frames(w, 30, |_| {});

    // An album tile: its menu, then Pin, and it's a pin in Shortcuts at once.
    let (album, at) = shelf_tile(&scene, ItemKind::Album);
    click_with(&scene, at, PointerEventButton::Right);
    assert!(
        menu.get_open(),
        "an album tile's right-click didn't open its menu"
    );
    assert_eq!(menu.get_item().title, album);
    assert!(!menu.get_pinned());
    let _ = frames(w, 30, |_| {});
    pixels::save(
        "menu-dark",
        &featherplay::offscreen::shown(w).expect("shown"),
    );
    scene.theme(false);
    let _ = frames(w, 30, |_| {});
    pixels::save(
        "menu-light",
        &featherplay::offscreen::shown(w).expect("shown"),
    );
    label::click(&scene, "Pin");
    assert!(!menu.get_open());
    assert_eq!(tiles(&scene).get(1), Some(&format!("*{album}")));
    assert_eq!(
        ui.global::<Toast>().get_text(),
        format!("Pinned \u{201C}{album}\u{201D}")
    );

    // The playlist page's ⋯: the account's own playlist, so the sidebar's menu, and Pin.
    scene.open("playlist").expect("playlist");
    scene.settle();
    let _ = frames(w, 30, |_| {});
    label::click(&scene, "More");
    let own = ui.global::<PlaylistMenu>();
    assert!(own.get_open(), "the playlist page's ⋯ didn't open its menu");
    let playlist = own.get_item().title.to_string();
    let _ = frames(w, 30, |_| {});
    assert!(label::shows(&scene, "Play next"));
    pixels::save(
        "page-light",
        &featherplay::offscreen::shown(w).expect("shown"),
    );
    label::click(&scene, "Pin");
    assert_eq!(tiles(&scene).get(1), Some(&format!("*{playlist}")));

    // A new account's row, the hint ending it.
    scene.open("home").expect("home");
    scene.settle();
    let data = ui.global::<HomeData>();
    data.set_shortcuts(ModelRc::new(VecModel::from(vec![
        data.get_shortcuts().row_data(0).expect("Liked songs"),
    ])));
    data.set_shortcut_pins(ModelRc::new(VecModel::from(vec![false])));
    data.set_shortcut_hint(true);
    let _ = frames(w, 30, |_| {});
    assert!(label::shows(&scene, "Pin what you play most"));
    assert!(label::shows(
        &scene,
        "Right-click a playlist, album or artist"
    ));
}
