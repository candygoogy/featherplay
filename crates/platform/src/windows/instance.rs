//! One featherplay per home: the named mutex that a second launch finds held, and the
//! message it asks the running one to show its window with. The app's `instance::start`
//! runs them first in a windowed run, before any window, tray icon, hotkey or store.
//!
//! One guard per home: a run with `FEATHERPLAY_HOME` set derives its names from that home,
//! so a dev build and the installed app don't stop each other, and tests don't meet the
//! user's app.
//!
//! How the second finds the first: every featherplay keeps the hidden end-session window
//! (`session_end`), on a thread that is always pumping messages, even while the main window
//! is hidden in the tray. The second looks those windows up by class and sends each one a
//! window message registered under the guard's key. Only a featherplay of the same home
//! registered that message, so only it answers `session_end::SHOWN`, after posting the tray's
//! Show to its event loop. Windows lets only the foreground app bring a window to the
//! front; the launch the user just made may, so it hands that right to each candidate
//! (`AllowSetForegroundWindow`) before asking.

use std::ffi::OsStr;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::time::Duration;

use windows::Win32::Foundation::{
    CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, HWND, LPARAM, SetLastError,
    WAIT_ABANDONED, WAIT_OBJECT_0, WIN32_ERROR, WPARAM,
};
use windows::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};
use windows::Win32::UI::WindowsAndMessaging::{
    AllowSetForegroundWindow, FindWindowExW, GetWindowThreadProcessId, RegisterWindowMessageW,
    SMTO_ABORTIFHUNG, SendMessageTimeoutW,
};
use windows::core::HSTRING;

use crate::common::identity::AUMID;
use crate::common::instance::fnv1a;
use crate::session_end;

/// How long one window may take to answer.
const ASK_TIMEOUT_MS: u32 = 2000;

/// The guard's key: the AppUserModelID, plus a hash of the home when `FEATHERPLAY_HOME` is
/// set (an empty one counts as unset, as in `Paths::resolve`). One home spelled two ways
/// (slashes, case, a trailing separator) gives one key. It holds no path separator.
pub fn key(home: Option<&OsStr>) -> String {
    match home.filter(|h| !h.is_empty()) {
        None => AUMID.to_string(),
        Some(home) => format!("{AUMID}-home-{:016x}", fnv1a(normalised(home).as_bytes())),
    }
}

/// The named mutex, in the session's namespace: one featherplay per user session and home.
pub fn mutex_name(key: &str) -> String {
    format!(r"Local\featherplay-{key}")
}

/// The window message a second launch asks with, registered under the key.
pub fn show_message(key: &str) -> Result<u32, String> {
    // SAFETY: a NUL-terminated string that outlives the call.
    let message =
        unsafe { RegisterWindowMessageW(&HSTRING::from(format!("featherplay-show-{key}"))) };
    if message == 0 {
        // SAFETY: reads this thread's last error, set by RegisterWindowMessageW.
        let e = unsafe { GetLastError() };
        return Err(format!(
            "register the show message: {}",
            e.to_hresult().message()
        ));
    }
    Ok(message)
}

/// Windows' absolute form of the path, lower case, with backslashes and no trailing one.
fn normalised(home: &OsStr) -> String {
    let path = std::path::absolute(home).unwrap_or_else(|_| PathBuf::from(home));
    let text = path.to_string_lossy().replace('/', "\\").to_lowercase();
    text.trim_end_matches('\\').to_string()
}

/// A kernel handle, closed on drop.
struct Handle(HANDLE);

impl Drop for Handle {
    fn drop(&mut self) {
        // SAFETY: a handle this process opened and closes once, here.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// Holding the mutex: this run is the one featherplay of its home. A mutex belongs to the
/// thread that took it, so keep the guard on that thread (it isn't `Send`) until the process
/// ends; if that thread ended first, another launch could take over.
pub struct Guard {
    mutex: Handle,
    _owning_thread: PhantomData<*const ()>,
}

impl Guard {
    fn new(mutex: Handle) -> Guard {
        Guard {
            mutex,
            _owning_thread: PhantomData,
        }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        // SAFETY: a mutex this thread owns; the handle closes after this.
        let _ = unsafe { ReleaseMutex(self.mutex.0) };
    }
}

/// Another featherplay of this home holds the mutex.
pub struct Running(Handle);

impl Running {
    /// Waits up to `timeout` for the other to let go (it quit, or its process ended), and
    /// takes the mutex over; gives itself back if the other still holds it.
    pub fn wait(self, timeout: Duration) -> Result<Guard, Running> {
        // Below INFINITE (u32::MAX), which would never time out.
        let ms = u32::try_from(timeout.as_millis())
            .unwrap_or(u32::MAX)
            .min(u32::MAX - 1);
        // SAFETY: a mutex handle this process holds open.
        let event = unsafe { WaitForSingleObject(self.0.0, ms) };
        // Abandoned: the other process ended without letting go; the mutex is ours now.
        if event == WAIT_OBJECT_0 || event == WAIT_ABANDONED {
            Ok(Guard::new(self.0))
        } else {
            Err(self)
        }
    }
}

/// Whether this run is the first of its home.
pub enum Claim {
    First(Guard),
    Running(Running),
}

/// Creates the named mutex, owned by this thread, or opens it if another featherplay
/// already did.
pub fn claim(name: &str) -> Result<Claim, String> {
    // SAFETY: clears this thread's last error, so the check below reads CreateMutexW's.
    unsafe { SetLastError(WIN32_ERROR(0)) };
    // SAFETY: default security; a NUL-terminated name that outlives the call.
    let handle = unsafe { CreateMutexW(None, true, &HSTRING::from(name)) }
        .map_err(|e| format!("create the mutex {name}: {}", e.message()))?;
    // SAFETY: reads this thread's last error, which CreateMutexW just set.
    let existed = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
    let mutex = Handle(handle);
    // An existing mutex opens without being owned, whatever `bInitialOwner` says.
    Ok(if existed {
        Claim::Running(Running(mutex))
    } else {
        Claim::First(Guard::new(mutex))
    })
}

/// Asks the featherplay of this key to show its window. True once one answered that it
/// did; false if none did (none is running, it is still starting, or it is closing). An
/// error if the message to ask with couldn't be registered.
pub fn ask_to_show(key: &str) -> Result<bool, String> {
    let message = show_message(key)?;
    let class = HSTRING::from(session_end::CLASS);
    let title = HSTRING::from(session_end::TITLE);
    let mut after: Option<HWND> = None;
    // SAFETY: NUL-terminated strings; `after` is a window found before, or None.
    while let Ok(hwnd) = unsafe { FindWindowExW(None, after, &class, &title) } {
        after = Some(hwnd);
        let mut pid = 0u32;
        // SAFETY: a window handle FindWindowExW just returned.
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        // Fails if this process may not take the foreground itself; then the window still
        // shows, only perhaps behind the current one.
        // SAFETY: no preconditions.
        let _ = unsafe { AllowSetForegroundWindow(pid) };
        let mut answer = 0usize;
        // SAFETY: a window found above (a stale one fails the call); the result pointer
        // outlives the call.
        let sent = unsafe {
            SendMessageTimeoutW(
                hwnd,
                message,
                WPARAM(0),
                LPARAM(0),
                SMTO_ABORTIFHUNG,
                ASK_TIMEOUT_MS,
                Some(&mut answer),
            )
        };
        if sent.0 != 0 && answer == session_end::SHOWN {
            return Ok(true);
        }
    }
    Ok(false)
}
