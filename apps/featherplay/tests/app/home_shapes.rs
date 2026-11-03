//! Home's shaped shelves do what cards do, on the offscreen platform (no window is shown): a
//! mix shortcut opens its playlist and its play button on hover plays it without opening; a
//! 16:9 video tile plays its song.

use std::cell::RefCell;
use std::rc::Rc;

use featherplay::offscreen;
use featherplay::ui::{
    AppWindow, CardItem, HomeData, HomeShelfRow, ItemKind, Nav, ShelfShape, Theme,
};
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
fn mix_shortcuts_and_video_tiles_open_and_play() {
    let _pump = offscreen::install().expect("platform");
    let ui = AppWindow::new().expect("window");
    // Home's rows, as the app's wiring gives them.
    featherplay::pages::home::bind_rows(&ui);
    ui.window().set_size(slint::LogicalSize::new(1280.0, 800.0));
    ui.global::<Theme>().set_still(true);
    ui.show().expect("show");
    let w = ui.window();

    let opened = Rc::new(RefCell::new(Vec::<String>::new()));
    let o = opened.clone();
    ui.global::<Nav>()
        .on_open_playlist(move |id| o.borrow_mut().push(format!("open {id}")));
    let data = ui.global::<HomeData>();
    let o = opened.clone();
    data.on_play_card(move |it| o.borrow_mut().push(format!("play {}", it.id)));
    let o = opened.clone();
    data.on_play_song(move |id| o.borrow_mut().push(format!("song {id}")));

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

    // No hero: the feed starts 32 px below the 64 px top bar, at x 280. The mixes' 64 px
    // shortcuts sit under their 48 px heading, four across 960 px (231 px wide, 12 px apart).
    let mix_y = 64.0 + 32.0 + 48.0 + 32.0;
    click(w, LogicalPosition::new(280.0 + 100.0, mix_y));
    // The play button shows on hover at the tile's right end: 36 px, 12 px in.
    click(
        w,
        LogicalPosition::new(280.0 + 231.0 + 12.0 + 231.0 - 12.0 - 18.0, mix_y),
    );

    // The videos 40 px below the mixes, three 306 px 16:9 tiles across, art 172 px tall.
    let video_y = 64.0 + 32.0 + 48.0 + 64.0 + 40.0 + 48.0 + 86.0;
    click(w, LogicalPosition::new(280.0 + 150.0, video_y));

    assert_eq!(
        *opened.borrow(),
        ["open LM", "play RDTMAK5uy_a", "song v1"],
        "what the clicks did"
    );
}
