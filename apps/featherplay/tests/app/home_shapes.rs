//! Home's rails do what cards do, on the offscreen platform (no window is shown): the
//! Shortcuts row's Liked songs tile shuffles Liked songs from its cover, a mix's play button
//! on hover plays it without opening, and a 16:9 video tile plays its song.

use std::cell::RefCell;
use std::rc::Rc;

use featherplay::offscreen;
use featherplay::ui::{AppWindow, CardItem, HomeData, HomeShelfRow, ItemKind, ShelfShape, Theme};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::support::{label, point};

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

/// One test: the platform can be installed once per process.
#[test]
fn the_liked_tile_shuffles_and_mix_and_video_tiles_play() {
    let _pump = offscreen::install().expect("platform");
    let ui = AppWindow::new().expect("window");
    // Home's rows, as the app's wiring gives them.
    featherplay::pages::home::bind_rows(&ui);
    // Tall enough for the video shelf under the Shortcuts row and the mixes.
    ui.window()
        .set_size(slint::LogicalSize::new(1280.0, 1000.0));
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

    data.set_shortcuts_shown(true);
    data.set_shortcuts(ModelRc::new(VecModel::from(vec![card(
        "LM",
        ItemKind::Playlist,
    )])));
    data.set_shelves(ModelRc::new(VecModel::from(vec![
        shelf(
            "Your mixes",
            ShelfShape::Mixes,
            vec![card("RDTMAK5uy_a", ItemKind::Playlist)],
        ),
        shelf(
            "Listen together",
            ShelfShape::Videos,
            vec![card("v1", ItemKind::Song)],
        ),
    ])));

    // A frame lays the rails out (their tiles are made as they come into view).
    w.take_snapshot().expect("frame");
    label::click(w, "Shuffle Liked songs");
    // The play button shows on the art while the mix's tile is hovered.
    point(w, label::centre(w, "Title RDTMAK5uy_a"));
    label::click(w, "Play Title RDTMAK5uy_a");
    label::click(w, "Title v1");

    assert_eq!(
        *did.borrow(),
        ["shuffle liked", "play RDTMAK5uy_a", "song v1"],
        "what the clicks did"
    );
}
