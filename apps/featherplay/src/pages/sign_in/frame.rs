//! The sign-in window's frame (`ui/sign_in_window.slint`), on the UI thread: a window of its
//! own with back, reload, the read-only address and the status strip, round the room where
//! the sign-in thread's WebView2 page sits as a child window (`webview`). The frame shows
//! what the page thread tells it (`webview::Event`) and passes back, reload, close, resizes
//! and moves on to it (`webview::Remote`). Once the page thread has ended, the frame goes.

use std::cell::RefCell;
use std::ffi::c_void;
use std::path::PathBuf;
use std::rc::Rc;

use slint::{CloseRequestResponse, ComponentHandle, LogicalSize};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::UI::Shell::{DefSubclassProc, RemoveWindowSubclass, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    WA_ACTIVE, WM_ACTIVATE, WM_MOVE, WM_NCDESTROY, WM_SIZE,
};

use super::address::Address;
use super::webview::{self, Event, Outcome, Remote, Room};
use crate::app::Ctx;
use crate::native;
use crate::popup;
use crate::shell;
use crate::ui::{SettingsData, SignInWindow, Theme};

/// Logical size the window opens at.
pub const SIZE: (f32, f32) = (540.0, 780.0);

thread_local! {
    /// The frame while a sign-in runs.
    static FRAME: RefCell<Option<SignInWindow>> = const { RefCell::new(None) };
}

/// Builds the frame in the main window's theme, without showing it: `open` shows it, the
/// snapshot renders it as it is.
pub fn create(ctx: &Rc<Ctx>) -> Result<SignInWindow, String> {
    let main = ctx.window().ok_or("no main window")?;
    let ui = SignInWindow::new().map_err(|e| format!("window: {e}"))?;
    popup::sync_theme(
        &main.global::<Theme>(),
        &ui.global::<Theme>(),
        main.global::<SettingsData>().get_appearance(),
    );
    ui.window().set_size(LogicalSize::new(SIZE.0, SIZE.1));
    Ok(ui)
}

/// Shows `address` in the frame's pill.
pub fn show_address(ui: &SignInWindow, address: &Address) {
    ui.set_host(address.host.as_str().into());
    ui.set_path(address.path.as_str().into());
    ui.set_secure(address.secure);
    ui.set_google(address.google);
}

/// Opens the frame and Google's page in it at `start`; `done` runs on the sign-in thread
/// once the page has closed (the frame is gone by the time it's called back on the UI
/// thread through `closed`).
pub fn open(
    ctx: &Rc<Ctx>,
    profile: PathBuf,
    start: &'static str,
    done: impl FnOnce(Outcome) + Send + 'static,
) -> Result<(), String> {
    if FRAME.with_borrow(Option::is_some) {
        return Err("the sign-in window is already open".into());
    }
    let ui = create(ctx)?;
    ui.show().map_err(|e| format!("show: {e}"))?;
    let Some(hwnd) = shell::hwnd(ui.window()) else {
        let _ = ui.hide();
        return Err("the sign-in window has no handle".into());
    };
    dress(&ui, hwnd);
    let room = Room {
        top: ui.get_page_top(),
        bottom: ui.get_page_bottom(),
    };
    let remote = match webview::open(hwnd, room, profile, start, tell, done) {
        Ok(r) => r,
        Err(e) => {
            let _ = ui.hide();
            return Err(e);
        }
    };
    ui.on_back(move || remote.back());
    ui.on_reload(move || remote.reload());
    ui.window().on_close_requested(move || {
        if remote.is_alive() {
            // The page thread reads the cookies first; the frame goes once it has ended.
            remote.close();
            return CloseRequestResponse::KeepWindowShown;
        }
        // The page thread is gone without saying how it ended: end the sign-in here, so the
        // window can still be closed and the page stops waiting.
        let _ = slint::invoke_from_event_loop(|| {
            super::window_closed(Outcome::Failed("the sign-in window stopped".into()));
        });
        CloseRequestResponse::HideWindow
    });
    if let Err(e) = follow(hwnd, remote) {
        log!("sign-in window: can't follow its size: {e}");
    }
    FRAME.with_borrow_mut(|f| *f = Some(ui));
    Ok(())
}

/// On the UI thread, once the page thread has ended: the frame goes.
pub fn closed() {
    if let Some(ui) = FRAME.with_borrow_mut(Option::take) {
        let _ = ui.hide();
    }
}

/// The title bar in the frame's colours, an opaque client area, the app's icon.
fn dress(ui: &SignInWindow, hwnd: isize) {
    let theme = ui.global::<Theme>();
    if let Err(e) = native::opaque(hwnd) {
        log!("sign-in window: opaque: {e}");
    }
    if let Err(e) = native::title_bar(hwnd, theme.get_dark(), theme.get_panel(), theme.get_text1())
    {
        log!("sign-in window: title bar: {e}");
    }
    let _ = native::set_icon(hwnd, ui.window().scale_factor());
}

/// From the sign-in thread: shown on the UI thread.
fn tell(event: Event) {
    let _ = slint::invoke_from_event_loop(move || {
        FRAME.with_borrow(|f| {
            let Some(ui) = f else { return };
            match event {
                Event::Address(a) => show_address(ui, &a),
                Event::CanGoBack(b) => ui.set_can_back(b),
                // Hidden at once; it goes once the profile is cleared (`closed`).
                Event::Closing => {
                    let _ = ui.hide();
                }
            }
        });
    });
}

/// The subclass that passes the frame's size and position on to the page.
const FOLLOW_SUBCLASS: usize = 0xfea7_0020;

/// Tells the page thread when the frame is resized (a drag, a maximize, another screen's
/// scale), moved, or brought back from the keyboard.
fn follow(h: isize, remote: Remote) -> Result<(), String> {
    let data = Box::into_raw(Box::new(remote));
    // SAFETY: a live HWND of this thread; `data` stays valid until WM_NCDESTROY frees it.
    let ok = unsafe {
        SetWindowSubclass(
            HWND(h as *mut c_void),
            Some(follow_proc),
            FOLLOW_SUBCLASS,
            data as usize,
        )
    };
    if !ok.as_bool() {
        // SAFETY: never shared, the subclass wasn't installed.
        drop(unsafe { Box::from_raw(data) });
        return Err("SetWindowSubclass failed".into());
    }
    Ok(())
}

unsafe extern "system" fn follow_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
    _id: usize,
    data: usize,
) -> LRESULT {
    if msg == WM_NCDESTROY {
        // SAFETY: the window is going; nothing calls this subclass with `data` again.
        unsafe {
            let _ = RemoveWindowSubclass(hwnd, Some(follow_proc), FOLLOW_SUBCLASS);
            drop(Box::from_raw(data as *mut Remote));
        }
        // SAFETY: the arguments this subclass was called with.
        return unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
    }
    // SAFETY: the arguments this subclass was called with; winit lays the frame out first.
    let result = unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) };
    // SAFETY: `data` is the box from `follow`, alive until WM_NCDESTROY.
    let remote = unsafe { *(data as *const Remote) };
    match msg {
        WM_SIZE => remote.relayout(),
        WM_MOVE => remote.moved(),
        // Back from Alt+Tab (not a click, which goes where it lands): the page takes the
        // keyboard again, as it did when the window opened.
        WM_ACTIVATE if (wparam.0 & 0xffff) as u32 == WA_ACTIVE => remote.focus_page(),
        _ => {}
    }
    result
}
