//! The Home hero's "Next in queue" rows open the song menu, on the offscreen platform (no
//! window is shown): right-click on a row, or its ⋯ on hover.

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

    // Direction A's hero: at 1280 px without the Up next panel it's 960 px wide, from x 280
    // and y 144 (32 + the 48 px heading, below the 64 px top bar), 16 px in. Its right
    // column is 428 px wide: a 2 x 2 grid of 210 px rows, 56 px tall, at its bottom.
    let list_x = 280.0 + 960.0 - 428.0 - 16.0;
    let col_x = |i: usize| list_x + (i % 2) as f32 * 218.0;
    let row_y = |i: usize| 144.0 + 16.0 + 152.0 - 2.0 * 56.0 + (i / 2) as f32 * 56.0 + 28.0;

    // Right-click on the first row.
    click(
        w,
        LogicalPosition::new(col_x(0) + 120.0, row_y(0)),
        PointerEventButton::Right,
    );
    assert!(menu.get_open());
    assert_eq!(menu.get_track().id, "a");
    menu.invoke_close();

    // A left click doesn't open it.
    click(
        w,
        LogicalPosition::new(col_x(1) + 120.0, row_y(1)),
        PointerEventButton::Left,
    );
    assert!(!menu.get_open());

    // Hovering shows the ⋯ at the row's right end.
    let more = LogicalPosition::new(col_x(2) + 210.0 - 4.0 - 18.0, row_y(2));
    click(w, more, PointerEventButton::Left);
    assert!(menu.get_open());
    assert_eq!(menu.get_track().id, "c");
}
