//! The Up next panel's pointer handling, on the offscreen platform (no window is shown):
//! drag to reorder, click to play, the × on hover to remove.

use std::cell::RefCell;
use std::rc::Rc;

use featherplay::offscreen;
use featherplay::ui::{AppWindow, Frame, QueueData, Theme, TrackRow};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, ModelRc, VecModel};

fn row(id: &str) -> TrackRow {
    TrackRow {
        id: id.into(),
        title: format!("Title {id}").into(),
        artist: "Artist".into(),
        duration: "3:00".into(),
        ..Default::default()
    }
}

fn at(x: f32, y: f32) -> LogicalPosition {
    LogicalPosition::new(x, y)
}

fn press(w: &slint::Window, p: LogicalPosition) {
    w.dispatch_event(WindowEvent::PointerMoved { position: p });
    w.dispatch_event(WindowEvent::PointerPressed {
        position: p,
        button: PointerEventButton::Left,
    });
}

fn release(w: &slint::Window, p: LogicalPosition) {
    w.dispatch_event(WindowEvent::PointerReleased {
        position: p,
        button: PointerEventButton::Left,
    });
}

/// One test: the platform can be installed once per process.
#[test]
fn drag_reorders_click_plays_and_hover_removes() {
    let _pump = offscreen::install().expect("platform");
    let ui = AppWindow::new().expect("window");
    ui.window().set_size(slint::LogicalSize::new(1280.0, 800.0));
    ui.global::<Theme>().set_still(true);
    ui.show().expect("show");
    let w = ui.window();

    let data = ui.global::<QueueData>();
    data.set_current(row("now"));
    data.set_has_current(true);
    let rows: Vec<TrackRow> = ["a", "b", "c", "d", "e"].iter().map(|s| row(s)).collect();
    data.set_upcoming(ModelRc::from(Rc::new(VecModel::from(rows))));
    ui.global::<Frame>().set_queue_open(true);

    let log = Rc::new(RefCell::new(Vec::<String>::new()));
    let l = log.clone();
    data.on_move(move |from, to| l.borrow_mut().push(format!("move {from} {to}")));
    let l = log.clone();
    data.on_play(move |i| l.borrow_mut().push(format!("play {i}")));
    let l = log.clone();
    data.on_remove(move |i| l.borrow_mut().push(format!("remove {i}")));

    // The panel is 400 px on the right at 1280 px; its list starts below the tabs (64), the header
    // (Clear alone, 32 + 8: there's no "Playing from" here) and the pinned row (56 + 8).
    // Rows are 56 px.
    let x = 1280.0 - 400.0 + 120.0;
    let row_y = |i: usize| 64.0 + 40.0 + 56.0 + 8.0 + i as f32 * 56.0 + 28.0;

    // Drag the first row down two rows.
    press(w, at(x, row_y(0)));
    for step in 1..=10 {
        let y = row_y(0) + step as f32 * 11.2;
        w.dispatch_event(WindowEvent::PointerMoved { position: at(x, y) });
    }
    release(w, at(x, row_y(2)));
    assert_eq!(*log.borrow(), ["move 0 2"]);

    // A click (no movement) plays the row.
    log.borrow_mut().clear();
    press(w, at(x, row_y(3)));
    release(w, at(x, row_y(3)));
    assert_eq!(*log.borrow(), ["play 3"]);

    // A small wobble is still a click, not a drag.
    log.borrow_mut().clear();
    press(w, at(x, row_y(1)));
    w.dispatch_event(WindowEvent::PointerMoved {
        position: at(x, row_y(1) + 2.0),
    });
    release(w, at(x, row_y(1) + 2.0));
    assert_eq!(*log.borrow(), ["play 1"]);

    // Hovering shows the × at the row's right end; it removes the row.
    log.borrow_mut().clear();
    let close = at(1280.0 - 16.0 - 4.0 - 18.0, row_y(4));
    press(w, close);
    release(w, close);
    assert_eq!(*log.borrow(), ["remove 4"]);
}
