//! Up next lights a row (its hover tile and ×) only while the pointer is on that row, and
//! only once the pointer has moved onto it: a row that slides under a resting pointer (the
//! list shifted when a row was played or removed) stays quiet, and a row that slid away
//! from the pointer doesn't keep its hover. On the offscreen platform (no window is shown).

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

fn rows(ids: &[&str]) -> Rc<VecModel<TrackRow>> {
    Rc::new(VecModel::from(
        ids.iter().map(|id| row(id)).collect::<Vec<_>>(),
    ))
}

/// The panel is 400 px on the right at 1280 px; its list starts below the tabs (64), the header
/// (Clear alone, 32 + 8) and the pinned row (56 + 8). Rows are 56 px.
const PANEL_X: f32 = 1280.0 - 400.0;
fn row_y(i: usize) -> f32 {
    64.0 + 40.0 + 56.0 + 8.0 + i as f32 * 56.0 + 28.0
}

fn at(x: f32, y: f32) -> LogicalPosition {
    LogicalPosition::new(x, y)
}

/// On row `i`'s title.
fn on_row(i: usize) -> LogicalPosition {
    at(PANEL_X + 120.0, row_y(i))
}

fn move_to(w: &slint::Window, p: LogicalPosition) {
    w.dispatch_event(WindowEvent::PointerMoved { position: p });
}

fn click(w: &slint::Window, p: LogicalPosition) {
    move_to(w, p);
    w.dispatch_event(WindowEvent::PointerPressed {
        position: p,
        button: PointerEventButton::Left,
    });
    w.dispatch_event(WindowEvent::PointerReleased {
        position: p,
        button: PointerEventButton::Left,
    });
}

/// Whether row `i` shows the hover tile: the pixel left of its cover differs from the same
/// pixel on row 6, which the pointer never visits.
fn lit(w: &slint::Window, i: usize) -> bool {
    slint::platform::update_timers_and_animations();
    let shot = w.take_snapshot().expect("snapshot");
    let px = |y: f32| shot.as_slice()[y as usize * shot.width() as usize + PANEL_X as usize + 20];
    px(row_y(i)) != px(row_y(6))
}

/// One test: the platform can be installed once per process.
#[test]
fn hover_follows_the_pointer_not_the_row() {
    let _pump = offscreen::install().expect("platform");
    let ui = AppWindow::new().expect("window");
    ui.window().set_size(slint::LogicalSize::new(1280.0, 800.0));
    ui.global::<Theme>().set_still(true);
    ui.show().expect("show");
    let w = ui.window();
    let data = ui.global::<QueueData>();
    data.set_current(row("now"));
    data.set_has_current(true);
    ui.global::<Frame>().set_queue_open(true);
    let ids = ["a", "b", "c", "d", "e", "f", "g", "h", "i"];

    // Plain hover lights the row under the pointer, and only that one.
    data.set_upcoming(ModelRc::from(rows(&ids)));
    move_to(w, on_row(1));
    assert!(lit(w, 1), "hovering a row lights it");
    assert!(!lit(w, 0), "the row above stays quiet");
    move_to(w, at(300.0, 400.0));
    assert!(!lit(w, 1), "leaving the row clears it");

    // A row is removed above the hovered one: the hovered row slides up, away from the
    // pointer, and the one below slides under it.
    let model = rows(&ids);
    data.set_upcoming(ModelRc::from(model.clone()));
    move_to(w, on_row(1));
    assert!(lit(w, 1));
    model.remove(0);
    assert!(
        !lit(w, 0),
        "the row that slid away from the pointer kept its hover"
    );
    assert!(
        !lit(w, 1),
        "the row that slid under the resting pointer lit up"
    );
    // Windows sends a move at the same place now and then (a window above changed).
    move_to(w, on_row(1));
    assert!(
        !lit(w, 1),
        "a move without movement lit the row that slid under the pointer"
    );
    move_to(w, at(PANEL_X + 124.0, row_y(1) + 1.0));
    assert!(lit(w, 1), "moving the pointer lights the row it's on");
    move_to(w, at(300.0, 400.0));

    // Clicking a row plays it: the controller's next state drops it from the list, so the
    // next row takes its place under the pointer.
    data.set_upcoming(ModelRc::from(rows(&ids)));
    click(w, on_row(0));
    data.set_upcoming(ModelRc::from(rows(&ids[1..])));
    move_to(w, on_row(0));
    assert!(
        !lit(w, 0),
        "the next row lit up under the pointer that played the row before it"
    );
    move_to(w, at(PANEL_X + 124.0, row_y(0) + 1.0));
    assert!(lit(w, 0), "moving the pointer lights the row it's on");
}
