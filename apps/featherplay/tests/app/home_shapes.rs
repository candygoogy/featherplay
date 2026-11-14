//! Home's rails do what cards do, on the offscreen platform (no window is shown): the first
//! rail's "Shuffle Liked songs" tile shuffles Liked songs, a mix's play button on hover plays
//! it without opening, and a 16:9 video tile plays its song.

use std::cell::RefCell;
use std::rc::Rc;

use featherplay::offscreen;
use featherplay::ui::{AppWindow, CardItem, HomeData, HomeShelfRow, ItemKind, ShelfShape, Theme};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, ModelRc, VecModel};

fn card(id: &str, kind: ItemKind) -> CardItem {
    CardItem {
        id: id.into(),
        kind,
        title: format!("Title {id}").into(),
        subtitle: "Subtitle".into(),
        ..Default::default()
    }
}

fn shelf(title: &str, shape: ShelfShape, items: Vec<CardItem>) -> HomeShelfRow {
    HomeShelfRow {
        title: title.into(),
        items: ModelRc::new(VecModel::from(items)),
        shape,
        ..Default::default()
    }
}

fn click(w: &slint::Window, p: LogicalPosition) {
    w.dispatch_event(WindowEvent::PointerMoved { position: p });
    w.dispatch_event(WindowEvent::PointerPressed {
        position: p,
        button: PointerEventButton::Left,
    });
    w.dispatch_event(WindowEvent::PointerReleased {
        position: p,
        button: PointerEventButton::Left,
    });
}

/// One test: the platform can be installed once per process.
#[test]
fn the_liked_tile_shuffles_and_mix_and_video_tiles_play() {
    let _pump = offscreen::install().expect("platform");
    let ui = AppWindow::new().expect("window");
    // Home's rows, as the app's wiring gives them.
    featherplay::pages::home::bind_rows(&ui);
    ui.window().set_size(slint::LogicalSize::new(1280.0, 800.0));
    ui.global::<Theme>().set_still(true);
    ui.show().expect("show");
    let w = ui.window();

    let did = Rc::new(RefCell::new(Vec::<String>::new()));
    let data = ui.global::<HomeData>();
    let d = did.clone();
    data.on_shuffle_liked(move || d.borrow_mut().push("shuffle liked".into()));
    let d = did.clone();
    data.on_play_card(move |it| d.borrow_mut().push(format!("play {}", it.id)));
    let d = did.clone();
    data.on_play_song(move |id| d.borrow_mut().push(format!("song {id}")));

    data.set_shelves(ModelRc::new(VecModel::from(vec![
        shelf(
            "Your mixes",
            ShelfShape::Mixes,
            vec![
                card("LM", ItemKind::Playlist),
                card("RDTMAK5uy_a", ItemKind::Playlist),
            ],
        ),
        shelf(
            "Listen together",
            ShelfShape::Videos,
            vec![card("v1", ItemKind::Song)],
        ),
    ])));

    // No band: the feed starts 32 px below the 64 px top bar, at x 280. The mixes' 200 px
    // tiles sit under their 48 px heading, 20 px apart.
    let art_y = 64.0 + 32.0 + 48.0;
    click(w, LogicalPosition::new(280.0 + 100.0, art_y + 100.0));
    // The play button shows on hover at the art's bottom right: 40 px, 8 px in.
    click(
        w,
        LogicalPosition::new(280.0 + 220.0 + 200.0 - 28.0, art_y + 200.0 - 28.0),
    );

    // The videos 32 px below the mixes' rail (its art, 8 px, two 20 px lines): 300 px 16:9
    // tiles, art 169 px tall.
    let video_y = art_y + 200.0 + 8.0 + 40.0 + 32.0 + 48.0 + 84.0;
    click(w, LogicalPosition::new(280.0 + 150.0, video_y));

    assert_eq!(
        *did.borrow(),
        ["shuffle liked", "play RDTMAK5uy_a", "song v1"],
        "what the clicks did"
    );
}
