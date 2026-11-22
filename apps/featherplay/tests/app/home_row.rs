//! Home's first row (the board of 2026-11-22), on the offscreen platform with fixture data (no
//! window is shown): the Now playing card takes 52 % of the row beside the Shortcuts, 536 px at
//! least, and the Shortcuts go under it below an 800 px page; the tiles step down from 104 to
//! 64 to 50 px before any is hidden, and what still doesn't fit is behind "N more", which opens
//! the rest as a menu.

use featherplay::snapshot::Scene;
use featherplay::ui::{CardItem, Frame, HomeData, ItemKind, ShortcutMore, Theme};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::support::{self, label};

/// The page's left edge at these widths: the 240 px sidebar and the 40 px padding.
const PAGE_X: f32 = 280.0;
/// The card's top: the 48 px top bar and 32 px of page top.
const CARD_Y: f32 = 80.0;

/// Where the Shortcuts grid is.
fn grid(scene: &Scene) -> label::LogicalRect {
    let found = label::all(scene, "Shortcuts");
    label::rect(found.last().expect("no Shortcuts grid"))
}

/// The shown tiles' heights, by title.
fn tile_height(scene: &Scene, title: &str) -> f32 {
    let side = scene.ui.global::<Theme>().get_sidebar_w();
    let found = label::shown_where(scene, |item, l| {
        l == title && label::rect(item).min_x() > side
    });
    label::rect(found.first().unwrap_or_else(|| panic!("no {title:?} tile"))).height()
}

/// The "N more" tile's label, if one shows.
fn more(scene: &Scene) -> Option<String> {
    let is_more = |l: &str| {
        l.strip_suffix(" more")
            .is_some_and(|n| n.parse::<u32>().is_ok())
    };
    let mut out = None;
    label::shown_where(scene, |_, l| {
        if is_more(l) {
            out = Some(l.to_string());
        }
        false
    });
    out
}

fn tiles(n: usize) -> ModelRc<CardItem> {
    let mut v = vec![CardItem {
        id: "LM".into(),
        kind: ItemKind::Playlist,
        title: "Liked songs".into(),
        ..Default::default()
    }];
    v.extend((1..n).map(|i| CardItem {
        id: format!("PL{i}").into(),
        kind: ItemKind::Playlist,
        title: format!("Tile {i}").into(),
        subtitle: "Playlist".into(),
        ..Default::default()
    }));
    ModelRc::new(VecModel::from(v))
}

/// One test: the platform can be installed once per process.
#[test]
fn the_card_and_the_shortcuts_share_the_row_and_step_down_before_n_more() {
    let scene = Scene::new(1568, 900, true, false).expect("scene");
    support::resize(scene.ui.window(), 1568, 900);
    scene.play_track(false, 0).expect("a song");
    scene.open("home").expect("home");
    scene.settle();
    let ui = &scene.ui;
    let data = ui.global::<HomeData>();
    let frame = ui.global::<Frame>();
    assert!(data.get_resume(), "no session on the card");
    assert!(data.get_shortcuts_shown(), "no Shortcuts signed in");

    // Three shortcuts and the hint. 1568 px, the panel closed: a 1,248 px row, the card 52 %
    // of it (649 px), the tiles 104 px beside it, as high as the card.
    frame.set_queue_open(false);
    data.set_shortcuts(tiles(3));
    data.set_shortcut_hint(true);
    scene.settle();
    let g = grid(&scene);
    assert_eq!(
        g.min_x(),
        PAGE_X + 649.0 + 24.0,
        "the card beside the tiles"
    );
    assert_eq!((g.min_y(), g.height()), (CARD_Y, 224.0));
    assert_eq!(tile_height(&scene, "Tile 1"), 104.0);
    assert!(label::shows(&scene, "Pin what you play most"));

    // The panel open: an 848 px row, the card at its least (536 px), one column of three 64
    // px tiles; the hint has no room.
    frame.set_queue_open(true);
    scene.settle();
    let g = grid(&scene);
    assert_eq!(g.min_x(), PAGE_X + 536.0 + 24.0);
    assert_eq!(tile_height(&scene, "Tile 2"), 64.0);
    assert!(!label::shows(&scene, "Pin what you play most"));

    // Four: 50 px tiles. Five: three of them and "2 more", which opens the other two.
    data.set_shortcut_hint(false);
    data.set_shortcuts(tiles(4));
    scene.settle();
    assert_eq!(tile_height(&scene, "Tile 3"), 50.0);
    assert_eq!(more(&scene), None);
    data.set_shortcuts(tiles(5));
    scene.settle();
    assert_eq!(more(&scene).as_deref(), Some("2 more"));
    assert!(!label::shows(&scene, "Tile 4"), "a hidden tile shows");
    label::click(&scene, "2 more");
    scene.settle();
    let menu = ui.global::<ShortcutMore>();
    assert!(menu.get_open(), "\"2 more\" didn't open its menu");
    assert_eq!(menu.get_from(), 3);
    assert!(
        label::shows(&scene, "Tile 4"),
        "the menu doesn't list the rest"
    );
    label::click(&scene, "Tile 4");
    assert!(!menu.get_open(), "a pick didn't close the menu");
    // It opened the playlist; back to Home.
    scene.open("home").expect("home");

    // Under an 800 px page (1100 px with the panel shut, a 780 px page): the tiles go under
    // the card, which takes the page's width.
    frame.set_queue_open(false);
    support::resize(ui.window(), 1100, 900);
    scene.settle();
    let g = grid(&scene);
    assert_eq!(g.min_x(), PAGE_X);
    assert_eq!(g.min_y(), CARD_Y + 224.0 + 24.0);
}
