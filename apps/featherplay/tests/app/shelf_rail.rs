//! The shelves move sideways (`ui/parts/rail.slint`, every rail's one way), on the offscreen
//! platform (no window is shown), motion off so each move lands at once: a sideways scroll
//! moves a rail and a vertical one scrolls the page instead; Shift and the vertical wheel move
//! the rail; the arrows show while the shelf is hovered and stay under the pointer, and two
//! quick clicks move two pages, from wherever a scroll left the rail.

use featherplay::snapshot::Scene;
use featherplay::ui::{CardItem, HomeData, HomeShelfRow, ItemKind, ShelfShape};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, ModelRc, SharedString, VecModel};

use crate::support::{self, label};

/// The same place, give or take the rounding of an eased position.
fn near(got: f32, want: f32, what: &str) {
    assert!((got - want).abs() < 0.5, "{what}: {got}, not {want}");
}

/// Where the tile `title` is across, if it shows.
fn x(scene: &Scene, title: &str) -> f32 {
    label::rect(&label::find(scene, title).unwrap_or_else(|| panic!("no tile {title:?}"))).min_x()
}

fn scroll(scene: &Scene, at: LogicalPosition, dx: f32, dy: f32) {
    scene
        .ui
        .window()
        .dispatch_event(WindowEvent::PointerScrolled {
            position: at,
            delta_x: dx,
            delta_y: dy,
        });
    scene.settle();
}

fn shift(scene: &Scene, down: bool) {
    let text = SharedString::from(Key::Shift);
    scene.ui.window().dispatch_event(if down {
        WindowEvent::KeyPressed { text }
    } else {
        WindowEvent::KeyReleased { text }
    });
}

/// One test: the platform can be installed once per process.
#[test]
fn a_rail_scrolls_sideways_and_pages_by_its_arrows() {
    let scene = Scene::new(1568, 900, true, false).expect("scene");
    support::resize(scene.ui.window(), 1568, 900);
    scene.open("home").expect("home");
    scene.settle();
    support::no_shortcuts(&scene);
    // A shelf of 30 tiles with names of their own, and more under it, so the page scrolls.
    let shelf = |name: &str, tile: &str| {
        let items: Vec<CardItem> = (0..30)
            .map(|i| CardItem {
                id: format!("MPREb_{tile}{i}").into(),
                kind: ItemKind::Album,
                title: format!("{tile} {i}").into(),
                subtitle: "Album".into(),
                ..Default::default()
            })
            .collect();
        HomeShelfRow {
            title: name.into(),
            items: ModelRc::new(VecModel::from(items)),
            shape: ShelfShape::Cards,
            ..Default::default()
        }
    };
    let shelves: Vec<HomeShelfRow> = std::iter::once(shelf("A long shelf", "Tile"))
        .chain((0..5).map(|k| shelf(&format!("More {k}"), &format!("More {k} tile"))))
        .collect();
    let data = scene.ui.global::<HomeData>();
    data.set_shelves(ModelRc::new(VecModel::from(shelves)));
    scene.settle();
    let start = x(&scene, "Tile 2");
    let head = label::rect(&label::find(&scene, "A long shelf").expect("the shelf's title"));
    let on = LogicalPosition::new(600.0, head.max_y() + 120.0);

    // A vertical wheel over the shelf is the page's: the rail stays.
    scroll(&scene, on, 0.0, -60.0);
    near(
        x(&scene, "Tile 2"),
        start,
        "a vertical wheel moved the rail",
    );
    let moved = label::rect(&label::find(&scene, "A long shelf").expect("title")).min_y();
    assert!(
        moved < head.min_y(),
        "a vertical wheel didn't scroll the page"
    );
    scroll(&scene, on, 0.0, 60.0);
    let head = label::rect(&label::find(&scene, "A long shelf").expect("the shelf's title"));
    let on = LogicalPosition::new(600.0, head.max_y() + 120.0);

    // Sideways: the rail follows (a notch's 60 px, then a touchpad's 15), and stops at the start.
    scroll(&scene, on, -60.0, 0.0);
    near(x(&scene, "Tile 2"), start - 60.0, "a sideways notch");
    scroll(&scene, on, -15.0, 2.0);
    near(
        x(&scene, "Tile 2"),
        start - 75.0,
        "a touchpad's sideways step",
    );
    scroll(&scene, on, 400.0, 0.0);
    near(x(&scene, "Tile 2"), start, "past the start");

    // Shift and the vertical wheel: sideways too.
    shift(&scene, true);
    scroll(&scene, on, 0.0, -60.0);
    shift(&scene, false);
    near(x(&scene, "Tile 2"), start - 60.0, "Shift and the wheel");

    // The arrows: there while the shelf is hovered, and still under the pointer on them.
    support::point(&scene, on);
    scene.settle();
    let right = label::centre(&scene, "Scroll right");
    support::point(&scene, right);
    scene.settle();
    assert!(
        label::shows(&scene, "Scroll right"),
        "the arrows went under the pointer"
    );
    // Two quick clicks from 60 px in: to the next whole tile and a page, then another page.
    support::click(&scene, right);
    support::click(&scene, right);
    scene.settle();
    // From 60 px in: to the next whole tile and a page (Tile 5 first), then a page more
    // (Tile 10 first), so Tile 12 is where Tile 2 was.
    near(x(&scene, "Tile 12"), start, "two clicks, two pages");
    // A sideways scroll then pages from where it left the rail: back to the whole tile before.
    scroll(&scene, on, 30.0, 0.0);
    support::click(&scene, label::centre(&scene, "Scroll left"));
    scene.settle();
    near(
        x(&scene, "Tile 7"),
        start,
        "back from part-way: Tile 5 first again",
    );
}
