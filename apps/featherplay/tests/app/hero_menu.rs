//! The Home hero's "Next in queue" cells open the song menu, on the offscreen platform (no
//! window is shown): right-click on a cell, or its ⋯ on hover.

use std::rc::Rc;

use featherplay::offscreen;
use featherplay::ui::{AppWindow, HomeData, SongMenu, Theme, TrackRow};
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

fn click(w: &slint::Window, p: LogicalPosition, button: PointerEventButton) {
    w.dispatch_event(WindowEvent::PointerMoved { position: p });
    w.dispatch_event(WindowEvent::PointerPressed {
        position: p,
        button,
    });
    w.dispatch_event(WindowEvent::PointerReleased {
        position: p,
        button,
    });
}

/// One test: the platform can be installed once per process.
#[test]
fn right_click_and_more_open_the_song_menu() {
    let _pump = offscreen::install().expect("platform");
    let ui = AppWindow::new().expect("window");
    // Home's rows, as the app's wiring gives them.
    featherplay::pages::home::bind_rows(&ui);
    ui.window().set_size(slint::LogicalSize::new(1280.0, 800.0));
    ui.global::<Theme>().set_still(true);
    ui.show().expect("show");
    let w = ui.window();

    let data = ui.global::<HomeData>();
    data.set_resume(true);
    let rows: Vec<TrackRow> = ["a", "b", "c", "d"].iter().map(|s| row(s)).collect();
    data.set_next(ModelRc::from(Rc::new(VecModel::from(rows))));
    let menu = ui.global::<SongMenu>();
    assert!(!menu.get_open());

    // The resume card: at 1280 px without the Up next panel it's 960 px wide, from x 280
    // and y 144 (32 + the 48 px heading, below the 64 px top bar). Under the 152 px cover
    // (16 px in), a hairline, then the queue strip: two 376 px cells, 48 px tall, from the
    // text column (x 192 in the card), 12 px under the hairline.
    let cell_x = |i: usize| 280.0 + 192.0 + i as f32 * 384.0;
    let cell_y = 144.0 + 16.0 + 152.0 + 16.0 + 1.0 + 12.0 + 24.0;

    // Right-click on the first cell.
    click(
        w,
        LogicalPosition::new(cell_x(0) + 120.0, cell_y),
        PointerEventButton::Right,
    );
    assert!(menu.get_open());
    assert_eq!(menu.get_track().id, "a");
    menu.invoke_close();

    // A left click doesn't open it.
    click(
        w,
        LogicalPosition::new(cell_x(1) + 120.0, cell_y),
        PointerEventButton::Left,
    );
    assert!(!menu.get_open());

    // Hovering shows the ⋯ at the cell's right end.
    let more = LogicalPosition::new(cell_x(1) + 376.0 - 4.0 - 18.0, cell_y);
    click(w, more, PointerEventButton::Left);
    assert!(menu.get_open());
    assert_eq!(menu.get_track().id, "b");
}
