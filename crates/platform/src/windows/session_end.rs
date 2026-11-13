//! Closing when Windows asks: at sign-out and shutdown, and when the installer (upgrading in
//! place) or the uninstaller needs the exe. Windows and the Restart Manager ask by sending
//! WM_QUERYENDSESSION, then WM_ENDSESSION, to every top-level window. winit's window doesn't
//! act on them, and it may be hidden in the tray, so featherplay keeps a hidden top-level
//! window of its own, on its own thread, which answers by quitting the way the tray's Quit
//! does: the session is saved, then the process exits.
//!
//! The uninstaller (`packaging/windows/featherplay.iss`) sends the same two messages to the
//! windows of class `CLASS` whose process runs the installed exe, so it never closes a
//! featherplay started from anywhere else.
//!
//! A second launch finds the running featherplay through the same window, which is there
//! and answering even while the main window is hidden in the tray (`instance`, `Show`).

use std::sync::{Condvar, Mutex, PoisonError};
use std::time::Duration;

use windows::Win32::Foundation::{
    ERROR_CLASS_ALREADY_EXISTS, GetLastError, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, MSG, RegisterClassW,
    WINDOW_EX_STYLE, WM_ENDSESSION, WM_QUERYENDSESSION, WNDCLASSW, WS_OVERLAPPED,
};
use windows::core::{HSTRING, PCWSTR};

pub use crate::common::session_end::*;

/// The hidden window's class. The uninstaller looks windows up by it: keep the two equal.
pub const CLASS: &str = "featherplay-endsession";
/// Its title, which the uninstaller matches too.
pub const TITLE: &str = "featherplay";

/// How long WM_ENDSESSION waits for the session to be saved before it returns. At
/// sign-out Windows may end the process any time after that.
const SAVE_WAIT: Duration = Duration::from_secs(5);

/// Set by `saved`, waited for by WM_ENDSESSION.
static SAVED: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

type OnEnd = Box<dyn FnOnce() + Send>;

/// The answer to `Show::message` once the window is on its way. Not 1, so a window that
/// answers any message with 1 isn't taken for featherplay.
pub const SHOWN: usize = 0x6670;

thread_local! {
    /// The window thread's `on_end`, taken by the first WM_ENDSESSION that ends the session.
    static ON_END: std::cell::RefCell<Option<OnEnd>> = const { std::cell::RefCell::new(None) };
    /// The window thread's answer to a second launch.
    static SHOW: std::cell::RefCell<Option<Show>> = const { std::cell::RefCell::new(None) };
}

/// Starts the window on its own thread and returns once it exists. When Windows ends the
/// session (or an installer asks), `on_end` runs once on that thread, and then
/// WM_ENDSESSION waits up to `SAVE_WAIT` for `saved`. `on_end` should only ask the app to
/// quit; the app calls `saved` once it has. Costs a blocked thread and no timers.
///
/// With `show`, the same window answers a second launch's request to show the main window
/// (`instance`).
pub fn listen(on_end: impl FnOnce() + Send + 'static, show: Option<Show>) -> Result<(), String> {
    let (ready_tx, ready) = std::sync::mpsc::sync_channel(1);
    let on_end: OnEnd = Box::new(on_end);
    std::thread::Builder::new()
        .name("featherplay-endsession".into())
        .spawn(move || {
            ON_END.with(|cell| *cell.borrow_mut() = Some(on_end));
            SHOW.with(|cell| *cell.borrow_mut() = show);
            match create_window() {
                Ok(_) => {
                    let _ = ready_tx.send(Ok(()));
                    pump();
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            }
        })
        .map_err(|e| format!("end-session thread: {e}"))?;
    ready
        .recv()
        .map_err(|_| "end-session thread ended early".to_string())?
}

/// Tells a waiting WM_ENDSESSION that the session is saved (or that the app is exiting
/// anyway), so it returns at once.
pub fn saved() {
    let (lock, cvar) = &SAVED;
    // A bool is valid whatever a panicking holder did, so a poisoned lock is taken over.
    *lock.lock().unwrap_or_else(PoisonError::into_inner) = true;
    cvar.notify_all();
}

fn wait_saved(timeout: Duration) {
    let (lock, cvar) = &SAVED;
    let done = lock.lock().unwrap_or_else(PoisonError::into_inner);
    let _ = cvar.wait_timeout_while(done, timeout, |done| !*done);
}

fn create_window() -> Result<HWND, String> {
    let class = HSTRING::from(CLASS);
    // SAFETY: the module handle of this exe; it isn't freed.
    let instance: HINSTANCE = unsafe { GetModuleHandleW(None) }
        .map_err(|e| format!("module handle: {}", e.message()))?
        .into();
    let wc = WNDCLASSW {
        lpfnWndProc: Some(window_proc),
        hInstance: instance,
        lpszClassName: PCWSTR(class.as_ptr()),
        ..Default::default()
    };
    // SAFETY: `wc` and the class name it points at outlive the call.
    if unsafe { RegisterClassW(&wc) } == 0 {
        // SAFETY: reads this thread's last error, set by RegisterClassW.
        let e = unsafe { GetLastError() };
        if e != ERROR_CLASS_ALREADY_EXISTS {
            return Err(format!("register {CLASS}: {}", e.to_hresult().message()));
        }
    }
    // A top-level window, never shown: message-only windows (HWND_MESSAGE) don't get
    // WM_QUERYENDSESSION. No WS_VISIBLE, so it has no taskbar button and never paints.
    // SAFETY: the class is registered above; the strings outlive the call.
    unsafe {
        CreateWindowExW(
            WINDOW_EX_STYLE(0),
            &class,
            &HSTRING::from(TITLE),
            WS_OVERLAPPED,
            0,
            0,
            0,
            0,
            None,
            None,
            Some(instance),
            None,
        )
    }
    .map_err(|e| format!("create the end-session window: {}", e.message()))
}

/// The window thread's message loop, until the process exits.
fn pump() {
    let mut msg = MSG::default();
    // SAFETY: `msg` is a valid MSG for each call; GetMessageW returns 0 on WM_QUIT and -1
    // on an error, which both end the loop.
    while unsafe { GetMessageW(&mut msg, None, 0, 0) }.0 > 0 {
        // SAFETY: a message GetMessageW just filled in.
        unsafe { DispatchMessageW(&msg) };
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        // Yes, the session may end. Nothing here would make the user lose work: the
        // session is saved on the way out.
        WM_QUERYENDSESSION => LRESULT(1),
        // wParam is FALSE when the end was cancelled (another app said no): keep running.
        WM_ENDSESSION if wparam.0 != 0 => {
            if let Some(on_end) = ON_END.with(|cell| cell.borrow_mut().take()) {
                on_end();
                wait_saved(SAVE_WAIT);
            }
            LRESULT(0)
        }
        _ => match show_answer(msg, wparam) {
            Some(answer) => answer,
            // SAFETY: the arguments this window procedure was called with.
            None => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
        },
    }
}

/// The answer to a second launch's `Show::message`, which asks in `wparam` (`ask_wparam`);
/// None for any other message.
fn show_answer(msg: u32, wparam: WPARAM) -> Option<LRESULT> {
    SHOW.with(|cell| {
        let show = cell.borrow();
        let show = show.as_ref().filter(|s| s.message == msg)?;
        let ask = if wparam.0 == ask_wparam(Ask::QuickSearch) {
            Ask::QuickSearch
        } else {
            Ask::Show
        };
        Some(LRESULT(if (show.on_show)(ask) {
            SHOWN as isize
        } else {
            0
        }))
    })
}

/// What a second launch puts in the show message's wParam for `ask`.
pub fn ask_wparam(ask: Ask) -> usize {
    match ask {
        Ask::Show => 0,
        Ask::QuickSearch => 1,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    use windows::Win32::UI::WindowsAndMessaging::{
        ENDSESSION_CLOSEAPP, FindWindowExW, GetWindowThreadProcessId, SMTO_ABORTIFHUNG,
        SendMessageTimeoutW,
    };

    /// The window of class `CLASS` that this process owns, looked up the way the
    /// uninstaller does it.
    fn own_window() -> Option<HWND> {
        let class = HSTRING::from(CLASS);
        let mut after: Option<HWND> = None;
        loop {
            // SAFETY: a NUL-terminated class name; `after` is a window found before, or None.
            let found = unsafe { FindWindowExW(None, after, &class, PCWSTR::null()) }.ok()?;
            let mut pid = 0u32;
            // SAFETY: a window handle FindWindowExW just returned.
            unsafe { GetWindowThreadProcessId(found, Some(&mut pid)) };
            if pid == std::process::id() {
                return Some(found);
            }
            after = Some(found);
        }
    }

    fn send(hwnd: HWND, msg: u32, wparam: usize) -> usize {
        let mut result = 0usize;
        // SAFETY: a live window of this process; the result pointer outlives the call.
        unsafe {
            SendMessageTimeoutW(
                hwnd,
                msg,
                WPARAM(wparam),
                LPARAM(ENDSESSION_CLOSEAPP as isize),
                SMTO_ABORTIFHUNG,
                10_000,
                Some(&mut result),
            )
        };
        result
    }

    #[test]
    fn the_end_of_the_session_quits_once_it_is_not_cancelled() {
        let (tx, quit) = mpsc::channel();
        listen(
            move || {
                let _ = tx.send(());
            },
            None,
        )
        .expect("listen");
        // The app has saved already, so WM_ENDSESSION returns at once.
        saved();
        let hwnd = own_window().expect("the end-session window");

        assert_eq!(send(hwnd, WM_QUERYENDSESSION, 0), 1, "agrees to end");
        send(hwnd, WM_ENDSESSION, 0);
        assert!(quit.try_recv().is_err(), "a cancelled end keeps running");
        send(hwnd, WM_ENDSESSION, 1);
        assert!(
            quit.recv_timeout(Duration::from_secs(5)).is_ok(),
            "quits when the session ends"
        );
    }
}
