//! The song menu stays above the bottom bar, on the offscreen platform (no window is shown):
//! opened from a row too low for it to fit below, it flips upward, ending above the row.

use featherplay::offscreen;
use featherplay::ui::{AppWindow, SongMenu, Theme, TrackRow};
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition};

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

/// The panel swallows clicks on its padding; a click anywhere else closes the menu. So a
/// click 3 px inside the panel's left edge, at `y`, tells whether the panel covers `y`.
/// The menu is opened at 440 (the anchor's bottom), its anchor's top at `top`.
fn panel_covers(w: &slint::Window, menu: &SongMenu, row: &TrackRow, top: f32, y: f32) -> bool {
    menu.invoke_show(row.clone(), 1000.0, 440.0, top);
    click(w, LogicalPosition::new(1003.0, y));
    let open = menu.get_open();
    menu.invoke_close();
    open
}

/// One test: the platform can be installed once per process.
#[test]
fn the_menu_flips_up_rather_than_run_into_the_bar() {
    let _pump = offscreen::install().expect("platform");
    let ui = AppWindow::new().expect("window");
    ui.window().set_size(slint::LogicalSize::new(1280.0, 800.0));
    ui.global::<Theme>().set_still(true);
    ui.show().expect("show");
    let w = ui.window();
    let menu = ui.global::<SongMenu>();
    let row = TrackRow {
        id: "a".into(),
        title: "Title".into(),
        artist: "Artist".into(),
        ..Default::default()
    };

    // At 800 px the floating bar starts at 702. Opened at 440, the 315 px menu would end at 755, so it
    // goes above the anchor instead (125 to 440) and leaves the bar alone.
    assert!(
        !panel_covers(w, &menu, &row, 440.0, 745.0),
        "the menu runs into the bottom bar"
    );
    assert!(
        panel_covers(w, &menu, &row, 440.0, 300.0),
        "the menu didn't flip above its anchor"
    );

    // Opened from a row's ⋯ (the row from 384 to 440), it ends at the row's top (69 to 384)
    // and leaves the row, its ⋯ and its duration uncovered.
    assert!(
        !panel_covers(w, &menu, &row, 384.0, 410.0),
        "the flipped menu covers its own row"
    );
    assert!(
        panel_covers(w, &menu, &row, 384.0, 300.0),
        "the flipped menu isn't above its row"
    );
}
