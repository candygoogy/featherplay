//! Home's resume card is as wide as a row of four shortcuts (996 px once they're 240 px each), its queue a column of rows at its right end, on
//! the offscreen platform (no window is shown): each row's play button plays the queue from
//! that row, and the card keeps its width whatever the shelves under it hold.

use std::cell::RefCell;
use std::rc::Rc;

use featherplay::offscreen;
use featherplay::ui::{
    AppWindow, CardItem, HomeData, HomeShelfRow, ItemKind, ShelfShape, Theme, TrackRow,
};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, ModelRc, VecModel};

fn mixes(n: usize) -> ModelRc<HomeShelfRow> {
    let items: Vec<CardItem> = (0..n)
        .map(|i| CardItem {
            id: format!("RDTMAK{i}").into(),
            kind: ItemKind::Playlist,
            title: format!("Mix {i}").into(),
            ..Default::default()
        })
        .collect();
    ModelRc::new(VecModel::from(vec![HomeShelfRow {
        title: "Your mixes".into(),
        items: ModelRc::new(VecModel::from(items)),
        shape: ShelfShape::Mixes,
        ..Default::default()
    }]))
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
fn the_card_keeps_a_steady_width_and_its_queue_rows_play() {
    let _pump = offscreen::install().expect("platform");
    let ui = AppWindow::new().expect("window");
    // Home's rows, as the app's wiring gives them.
    featherplay::pages::home::bind_rows(&ui);
    ui.window().set_size(slint::LogicalSize::new(1568.0, 900.0));
    ui.global::<Theme>().set_still(true);
    ui.show().expect("show");
    let w = ui.window();

    let played = Rc::new(RefCell::new(Vec::<i32>::new()));
    let data = ui.global::<HomeData>();
    let p = played.clone();
    data.on_play_next(move |i| p.borrow_mut().push(i));
    data.set_resume(true);
    let next: Vec<TrackRow> = ["a", "b", "c"]
        .iter()
        .map(|id| TrackRow {
            id: (*id).into(),
            title: format!("Title {id}").into(),
            artist: "Artist".into(),
            ..Default::default()
        })
        .collect();
    data.set_next(ModelRc::from(Rc::new(VecModel::from(next))));

    // At 1568 px the content is 1248 px wide from x 280: the card stops with four 240 px
    // shortcuts, at 996 px (from y 144); its rows are 30 % of it (298 px), ending 8 px from its right edge, 48 px tall from
    // 28 px down. A row's play button is on its 40 px cover, 8 px in.
    let play = |card_w: f32, rows_w: f32, i: usize| {
        LogicalPosition::new(
            280.0 + card_w - 8.0 - rows_w + 8.0 + 20.0,
            144.0 + 28.0 + 24.0 + i as f32 * 48.0,
        )
    };
    // However many shortcuts the row under it holds, the card stays 996 px wide.
    for n in [2, 4, 5] {
        played.borrow_mut().clear();
        data.set_shelves(mixes(n));
        for i in 0..3 {
            click(w, play(996.0, 298.0, i));
        }
        assert_eq!(
            *played.borrow(),
            [0, 1, 2],
            "the rows of the 996 px card, {n} shortcuts"
        );
    }
}
