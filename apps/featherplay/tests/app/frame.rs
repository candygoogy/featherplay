//! The frame's keyboard routing, on the offscreen platform (no window is shown).

use std::cell::RefCell;
use std::rc::Rc;

use featherplay::offscreen;
use featherplay::ui::{AppWindow, Nav};
use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, SharedString};

fn tap(w: &slint::Window, text: SharedString) {
    w.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    w.dispatch_event(WindowEvent::KeyReleased { text });
}

/// One test: the platform can be installed once per process.
#[test]
fn ctrl_k_search_enter_and_alt_left() {
    let _pump = offscreen::install().expect("platform");
    let ui = AppWindow::new().expect("window");
    ui.show().expect("show");
    let w = ui.window();
    w.dispatch_event(WindowEvent::WindowActiveChanged(true));

    let searched = Rc::new(RefCell::new(None::<String>));
    let s = searched.clone();
    ui.global::<Nav>()
        .on_search(move |q| *s.borrow_mut() = Some(q.to_string()));
    let backs = Rc::new(RefCell::new(0));
    let b = backs.clone();
    ui.global::<Nav>().on_back(move || *b.borrow_mut() += 1);

    // Typing without Ctrl+K reaches no field.
    tap(w, "x".into());
    tap(w, Key::Return.into());
    assert_eq!(*searched.borrow(), None);

    w.dispatch_event(WindowEvent::KeyPressed {
        text: Key::Control.into(),
    });
    tap(w, "k".into());
    w.dispatch_event(WindowEvent::KeyReleased {
        text: Key::Control.into(),
    });
    for c in ["d", "a", "f", "t"] {
        tap(w, c.into());
    }
    tap(w, Key::Return.into());
    assert_eq!(searched.borrow().as_deref(), Some("daft"));

    // After a search, focus is back on the window, so Alt+Left goes back.
    w.dispatch_event(WindowEvent::KeyPressed {
        text: Key::Alt.into(),
    });
    tap(w, Key::LeftArrow.into());
    w.dispatch_event(WindowEvent::KeyReleased {
        text: Key::Alt.into(),
    });
    assert_eq!(*backs.borrow(), 1);
}
