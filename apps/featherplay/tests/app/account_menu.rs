//! The sidebar's account row opens the account menu, on the offscreen platform (no window
//! is shown): its identities pick by key (the active one doesn't), its actions reach their
//! callbacks, and a click outside or Escape closes it. Signed out, the row asks to sign in.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use featherplay::offscreen;
use featherplay::ui::{AccountItem, AccountMenu, AppWindow, Nav, PageKind, SidebarData, Theme};
use slint::platform::{Key, PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, ModelRc, VecModel};

const W: f32 = 1280.0;
const H: f32 = 800.0;
/// The account row's top: 64 px above the sidebar rows' foot, the floating bar's top (98 px up).
const ROW_TOP: f32 = H - 98.0 - 64.0;

fn click(w: &slint::Window, p: LogicalPosition) {
    w.dispatch_event(WindowEvent::PointerMoved { position: p });
    for pressed in [true, false] {
        let button = PointerEventButton::Left;
        w.dispatch_event(if pressed {
            WindowEvent::PointerPressed {
                position: p,
                button,
            }
        } else {
            WindowEvent::PointerReleased {
                position: p,
                button,
            }
        });
    }
}

fn click_row(w: &slint::Window) {
    click(w, LogicalPosition::new(100.0, ROW_TOP + 28.0));
}

fn accounts() -> ModelRc<AccountItem> {
    let item = |key: &str, name: &str, detail: &str, active: bool| AccountItem {
        key: key.into(),
        name: name.into(),
        detail: detail.into(),
        initial: name[..1].into(),
        active,
        ..Default::default()
    };
    ModelRc::from(Rc::new(VecModel::from(vec![
        item("2", "Test Listener", "@testlistener", true),
        item("0", "Test Listener", "No channel", false),
        item(
            "0/100000000000000000002",
            "Test Channel",
            "12 subscribers",
            false,
        ),
    ])))
}

/// Where the menu's rows are, from its layout (ui/account_menu.slint): it ends 4 px above
/// the row; 8 px padding, a 32 px title, 52 px identity rows, a 9 px divider, 36 px actions.
fn layout(rows: usize) -> (f32, f32) {
    let h = 16.0 + 32.0 + rows as f32 * 52.0 + 9.0 + 72.0;
    let top = ROW_TOP - 4.0 - h;
    let first_row = top + 8.0 + 32.0;
    (first_row, first_row + rows as f32 * 52.0 + 9.0)
}

/// One test: the platform can be installed once per process.
#[test]
fn the_account_row_opens_the_account_menu() {
    let _pump = offscreen::install().expect("platform");
    let ui = AppWindow::new().expect("window");
    ui.window().set_size(slint::LogicalSize::new(W, H));
    ui.global::<Theme>().set_still(true);
    ui.show().expect("show");
    let w = ui.window();
    let menu = ui.global::<AccountMenu>();
    let sidebar = ui.global::<SidebarData>();

    // Signed out: the row goes to Sign in, no menu.
    let went = Rc::new(Cell::new(None));
    let g = went.clone();
    ui.global::<Nav>().on_go(move |k| g.set(Some(k)));
    click_row(w);
    assert!(!menu.get_open(), "signed out, the row opened the menu");
    assert_eq!(went.get(), Some(PageKind::SignIn));

    // Signed in: the row opens the menu, and it asks for its list.
    sidebar.set_signed_in(true);
    sidebar.set_display_name("Test Listener".into());
    let opened = Rc::new(Cell::new(0));
    let o = opened.clone();
    menu.on_opened(move || o.set(o.get() + 1));
    let picked = Rc::new(RefCell::new(Vec::<String>::new()));
    let p = picked.clone();
    menu.on_pick(move |key| p.borrow_mut().push(key.into()));
    let actions = Rc::new(RefCell::new(Vec::<&str>::new()));
    let a = actions.clone();
    menu.on_switch_google(move || a.borrow_mut().push("switch"));
    let a = actions.clone();
    menu.on_sign_out(move || a.borrow_mut().push("sign out"));
    menu.set_accounts(accounts());

    click_row(w);
    assert!(
        menu.get_open(),
        "the account row didn't open the account menu"
    );
    assert_eq!(opened.get(), 1);
    assert_eq!(menu.get_anchor_x(), 8.0);
    assert_eq!(menu.get_anchor_top(), ROW_TOP);

    // The active identity closes the menu without a pick; another one picks its key.
    let (first_row, actions_top) = layout(3);
    click(w, LogicalPosition::new(120.0, first_row + 26.0));
    assert!(!menu.get_open());
    assert!(
        picked.borrow().is_empty(),
        "the active identity was picked again"
    );
    click_row(w);
    click(
        w,
        LogicalPosition::new(120.0, first_row + 2.0 * 52.0 + 26.0),
    );
    assert_eq!(*picked.borrow(), ["0/100000000000000000002"]);
    assert!(!menu.get_open());

    // The two actions.
    click_row(w);
    click(w, LogicalPosition::new(120.0, actions_top + 18.0));
    click_row(w);
    click(w, LogicalPosition::new(120.0, actions_top + 36.0 + 18.0));
    assert_eq!(*actions.borrow(), ["switch", "sign out"]);

    // A click outside closes it, and so does Escape.
    click_row(w);
    click(w, LogicalPosition::new(800.0, 300.0));
    assert!(!menu.get_open(), "a click outside didn't close the menu");
    click_row(w);
    w.dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    assert!(!menu.get_open(), "Escape didn't close the menu");
    assert_eq!(opened.get(), 6);
}
