//! Pasting into the top bar's search field, on the offscreen platform (no window is shown):
//! whatever the clipboard holds, the field stays one line. Windows' clipboard ends lines with
//! "\r\n" and Slint's single-line input only turns the "\n" into a space, so the "\r" broke
//! the text onto a second line, which sat above the field's middle.

use featherplay::offscreen;
use featherplay::top_bar;
use featherplay::ui::{AppWindow, TopBarData};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, SharedString};

fn tap(w: &slint::Window, text: SharedString) {
    w.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    w.dispatch_event(WindowEvent::KeyReleased { text });
}

fn with_ctrl(w: &slint::Window, key: &str) {
    w.dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    tap(w, key.into());
    w.dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
}

fn type_text(w: &slint::Window, text: &str) {
    for c in text.chars() {
        tap(w, c.to_string().into());
    }
}

/// Clears the field, puts `text` on the clipboard and pastes it with Ctrl+V.
fn paste(ui: &AppWindow, text: &str) -> String {
    ui.global::<TopBarData>().set_query("".into());
    offscreen::set_clipboard(text);
    with_ctrl(ui.window(), "v");
    ui.global::<TopBarData>().get_query().to_string()
}

/// One test: the platform can be installed once per process.
#[test]
fn pasted_text_stays_on_one_line() {
    let _pump = offscreen::install().expect("platform");
    let ui = AppWindow::new().expect("window");
    top_bar::init(&ui);
    ui.window().set_size(slint::LogicalSize::new(1280.0, 800.0));
    ui.show().expect("show");
    let w = ui.window();
    w.dispatch_event(WindowEvent::WindowActiveChanged(true));
    with_ctrl(w, "k");

    assert_eq!(
        paste(&ui, "daft punk"),
        "daft punk",
        "a plain paste lands as it is"
    );
    assert_eq!(
        paste(&ui, "daft\r\npunk\tlive\r\n"),
        "daft punk live",
        "line breaks and tabs become spaces, the ends are trimmed"
    );
    assert_eq!(
        paste(&ui, "\r\n\t "),
        "",
        "nothing but breaks leaves it empty"
    );
    let long = "around the world ".repeat(30);
    let pasted = paste(&ui, &format!("{long}\r\n{long}"));
    assert!(!pasted.contains(char::is_control), "{pasted:?}");
    assert_eq!(pasted.len(), 2 * long.trim_end().len() + 1);

    // Typing is left alone: a space typed at the end stays for the next word.
    ui.global::<TopBarData>().set_query("".into());
    type_text(w, "air ");
    assert_eq!(ui.global::<TopBarData>().get_query(), "air ");
    // A paste after it lands at the end, the cursor after it.
    offscreen::set_clipboard("moon\r\nsafari\r\n");
    with_ctrl(w, "v");
    type_text(w, "!");
    assert_eq!(ui.global::<TopBarData>().get_query(), "air moon safari!");

    // Each paste is noted for the log first (`top_bar::pasting`), by key or by the menu.
    let noted = std::rc::Rc::new(std::cell::Cell::new(0));
    let n = noted.clone();
    ui.global::<TopBarData>()
        .on_pasting(move || n.set(n.get() + 1));
    with_ctrl(w, "v");
    assert_eq!(noted.get(), 1, "Ctrl+V");
    with_ctrl(w, "c");
    type_text(w, "v");
    assert_eq!(noted.get(), 1, "Ctrl+C and a typed v aren't pastes");

    // A right-click on the field opens its menu; Paste there pastes.
    ui.global::<TopBarData>().set_query("".into());
    offscreen::set_clipboard("homework");
    let field = LogicalPosition::new(400.0, 32.0);
    press(w, field, PointerEventButton::Right);
    // The menu opens at the pointer: Cut, Copy, Paste (the third, 32 px rows from 4 px
    // down), a separator, Select all.
    press(
        w,
        LogicalPosition::new(field.x + 50.0, field.y + 84.0),
        PointerEventButton::Left,
    );
    assert_eq!(
        ui.global::<TopBarData>().get_query(),
        "homework",
        "the menu's Paste"
    );
    assert_eq!(noted.get(), 2, "the menu's Paste is noted too");
}

fn press(w: &slint::Window, position: LogicalPosition, button: PointerEventButton) {
    w.dispatch_event(WindowEvent::PointerMoved { position });
    w.dispatch_event(WindowEvent::PointerPressed { position, button });
    w.dispatch_event(WindowEvent::PointerReleased { position, button });
}
