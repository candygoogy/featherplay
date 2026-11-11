//! One featherplay at a time. It hides in the notification area, so starting it again from
//! the Start menu would otherwise run a second copy: two tray icons, two sets of hotkeys,
//! two writers of the queue and settings, double scrobbles. `start` runs first in a windowed
//! run, before any window, tray icon, hotkey or store: it claims a named mutex, and if
//! another featherplay holds it, asks that one to show its window and exits.
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
use std::time::{Duration, Instant};

use featherplay_platform::identity::AUMID;
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

use featherplay_platform::session_end;

/// How long a second launch keeps trying: the first may still be starting (its end-session
/// window not made yet) or closing (saving its session).
const HANDOFF: Duration = Duration::from_secs(10);
/// How long it waits for the first to let go between two asks.
const RETRY: Duration = Duration::from_millis(250);
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

/// The key for this run's environment.
pub fn key_here() -> String {
    key(std::env::var_os("FEATHERPLAY_HOME").as_deref())
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

/// 64-bit FNV-1a: stable across builds and Rust versions, unlike `DefaultHasher`, so two
/// featherplay builds agree on a home's key.
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, &b| {
        (hash ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
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
/// did; false if none did (none is running, it is still starting, or it is closing).
pub fn ask_to_show(key: &str) -> bool {
    let message = match show_message(key) {
        Ok(m) => m,
        Err(e) => {
            log!("single instance: {e}");
            return false;
        }
    };
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
            return true;
        }
    }
    false
}

/// What `main` does after `start`.
pub enum Start {
    /// The first featherplay of this home: run, keeping the guard (if the mutex could be
    /// made at all) on the main thread until the end.
    Run(Option<Guard>),
    /// Another one showed its window: exit without creating anything.
    Exit,
}

/// Call on the main thread, first thing in a windowed run.
pub fn start() -> Start {
    let key = key_here();
    let mut running = match claim(&mutex_name(&key)) {
        Ok(Claim::First(guard)) => return Start::Run(Some(guard)),
        Ok(Claim::Running(running)) => running,
        Err(e) => {
            log!("single instance: {e}; starting anyway");
            return Start::Run(None);
        }
    };
    let deadline = Instant::now() + HANDOFF;
    loop {
        if ask_to_show(&key) {
            log!("featherplay is already running; it showed its window");
            return Start::Exit;
        }
        // Starting (no window yet) or closing (it said no): if it ends, this run takes over.
        running = match running.wait(RETRY) {
            Ok(guard) => return Start::Run(Some(guard)),
            Err(running) => running,
        };
        if Instant::now() >= deadline {
            log!("featherplay is already running but didn't answer; not starting another");
            return Start::Exit;
        }
    }
}
