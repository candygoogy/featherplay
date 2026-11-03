//! The top bar's search field, on the offscreen platform (no window is shown): Ctrl+K
//! focuses it, Enter goes to Search, Escape clears it then leaves it, × clears it, and a
//! search page shown (back and forward too) puts its query in it.

use std::cell::RefCell;
use std::rc::Rc;

use featherplay::offscreen;
use featherplay::router::Page;
use featherplay::top_bar;
use featherplay::ui::{AppWindow, Nav, PageKind, TopBarData};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, SharedString};

fn tap(w: &slint::Window, text: SharedString) {
    w.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    w.dispatch_event(WindowEvent::KeyReleased { text });
}

fn type_text(w: &slint::Window, text: &str) {
    for c in text.chars() {
        tap(w, c.to_string().into());
    }
}

fn ctrl_k(w: &slint::Window) {
    w.dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    tap(w, "k".into());
    w.dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
}

fn click(w: &slint::Window, position: LogicalPosition) {
    w.dispatch_event(WindowEvent::PointerMoved { position });
    w.dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    w.dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
}

fn query(ui: &AppWindow) -> String {
    ui.global::<TopBarData>().get_query().to_string()
}

/// One test: the platform can be installed once per process.
#[test]
fn top_field_takes_ctrl_k_enter_escape_and_clear() {
    let _pump = offscreen::install().expect("platform");
    let ui = AppWindow::new().expect("window");
    ui.window().set_size(slint::LogicalSize::new(1280.0, 800.0));
    ui.show().expect("show");
    let w = ui.window();
    w.dispatch_event(WindowEvent::WindowActiveChanged(true));

    // As the router does: a search shows the Search page.
    let searched = Rc::new(RefCell::new(Vec::<String>::new()));
    let s = searched.clone();
    let weak = ui.as_weak();
    ui.global::<Nav>().on_search(move |q| {
        s.borrow_mut().push(q.to_string());
        if let Some(ui) = weak.upgrade() {
            ui.global::<Nav>().set_page(PageKind::Search);
        }
    });

    // Ctrl+K puts the keys in the top field; Enter searches and goes to Search.
    ctrl_k(w);
    type_text(w, "daft");
    assert_eq!(query(&ui), "daft");
    tap(w, Key::Return.into());
    assert_eq!(*searched.borrow(), ["daft"]);
    assert_eq!(ui.global::<Nav>().get_page(), PageKind::Search);

    // After Enter the field is left: typing reaches nothing, the query stays.
    type_text(w, "x");
    assert_eq!(query(&ui), "daft");

    // Ctrl+K selects what's there, so typing replaces it.
    ctrl_k(w);
    type_text(w, "moby");
    assert_eq!(query(&ui), "moby");

    // Escape clears the field and keeps it focused; the next Escape leaves it.
    tap(w, Key::Escape.into());
    assert_eq!(query(&ui), "");
    type_text(w, "a");
    assert_eq!(query(&ui), "a");
    tap(w, Key::Escape.into());
    tap(w, Key::Escape.into());
    type_text(w, "z");
    assert_eq!(query(&ui), "");

    // A search page shown, by back or forward too, puts its query in the field.
    let air = Page::Search {
        query: "air".into(),
    };
    top_bar::page_shown(&ui, &air, Some(&Page::Home));
    assert_eq!(query(&ui), "air");
    // Going to any other page clears it ("ccc" stayed on Settings)...
    top_bar::page_shown(&ui, &Page::Settings, Some(&air));
    assert_eq!(query(&ui), "");
    // ...and back to the search brings its query back.
    top_bar::page_shown(&ui, &air, Some(&Page::Settings));
    assert_eq!(query(&ui), "air");
    // The same page shown again (a rename, a Try again) leaves what's being typed.
    ui.global::<TopBarData>().set_query("half typ".into());
    top_bar::page_shown(&ui, &Page::Home, Some(&Page::Home));
    assert_eq!(query(&ui), "half typ");
    top_bar::page_shown(&ui, &Page::Home, None);
    assert_eq!(query(&ui), "", "the first page shown clears it too");

    // × clears it and puts the cursor in it. The field starts at the content's 40 px pad
    // (240 + 40) and is 600 px wide; × sits 4 px in from its right end, 32 px wide.
    top_bar::page_shown(&ui, &air, Some(&Page::Home));
    click(w, LogicalPosition::new(280.0 + 600.0 - 4.0 - 16.0, 32.0));
    assert_eq!(query(&ui), "");
    type_text(w, "b");
    assert_eq!(query(&ui), "b");
}
