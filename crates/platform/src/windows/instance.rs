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
use crate::session_end::{self, Ask};

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
    ask(key, Ask::Show)
}

/// Asks the featherplay of this key for `what` (its window, or the quick search), as
/// `ask_to_show` does.
pub fn ask(key: &str, what: Ask) -> Result<bool, String> {
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
                WPARAM(session_end::ask_wparam(what)),
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

#[cfg(test)]
mod tests {
    //! Nothing here touches the user's running featherplay: each test uses a home of its
    //! own, so its mutex and message are its own.

    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, mpsc};

    use super::*;

    /// A home no featherplay runs with, so the test's mutex and message are its own.
    fn test_key(test: &str) -> String {
        key(Some(OsStr::new(&format!(
            "D:/featherplay-test-{test}-{}",
            std::process::id()
        ))))
    }

    #[test]
    fn an_installed_run_uses_the_fixed_name() {
        assert_eq!(
            mutex_name(&key(None)),
            r"Local\featherplay-featherplay.featherplay"
        );
    }

    #[test]
    fn each_home_has_a_guard_of_its_own_whatever_its_spelling() {
        let home = Some(OsStr::new("D:/dev/featherplay-home"));
        // (case, one home, another, whether they share a guard)
        let cases: [(&str, Option<&OsStr>, Option<&OsStr>, bool); 4] = [
            (
                "an empty home counts as unset",
                Some(OsStr::new("")),
                None,
                true,
            ),
            ("a dev home has a guard of its own", home, None, false),
            (
                "two homes have two guards",
                home,
                Some(OsStr::new("D:/dev/featherplay-other")),
                false,
            ),
            (
                "one home spelled two ways has one guard",
                home,
                Some(OsStr::new(r"d:\dev\Featherplay-Home\")),
                true,
            ),
        ];
        for (case, a, b, same) in cases {
            assert_eq!(key(a) == key(b), same, "{case}: {} {}", key(a), key(b));
        }
        // A kernel object name may have one backslash, after its namespace.
        let k = key(Some(OsStr::new(r"D:\dev\cache\featherplay-dev")));
        assert!(
            !k.contains(['\\', '/', ':']),
            "the names hold no path separator: {k}"
        );
    }

    #[test]
    fn a_claim_after_the_first_has_gone_runs() {
        let name = mutex_name(&test_key("gone"));
        drop(claim(&name).expect("first claim"));
        assert!(matches!(
            claim(&name).expect("claim again"),
            Claim::First(_)
        ));
    }

    #[test]
    fn waiting_takes_over_once_the_first_lets_go() {
        let name = mutex_name(&test_key("wait"));
        let (held_tx, held) = mpsc::channel();
        let (release_tx, release) = mpsc::channel::<()>();
        // The first instance on a thread of its own: a mutex is owned by a thread, so this
        // one can't take it over while the other holds it.
        let thread_name = name.clone();
        let first = std::thread::spawn(move || {
            let guard = claim(&thread_name).expect("first claim");
            held_tx.send(()).expect("say held");
            let _ = release.recv();
            drop(guard);
        });
        held.recv().expect("the first holds it");
        let Claim::Running(running) = claim(&name).expect("second claim") else {
            panic!("the second claim should find the first running");
        };
        let Err(running) = running.wait(Duration::from_millis(50)) else {
            panic!("taken while the first still holds it");
        };
        release_tx.send(()).expect("let go");
        first.join().expect("first thread");
        assert!(
            running.wait(Duration::from_secs(5)).is_ok(),
            "not taken over once the first let go"
        );
    }

    #[test]
    fn asking_to_show_reaches_the_window_of_the_same_home() {
        let key = test_key("show");
        let shown = Arc::new(AtomicUsize::new(0));
        let count = shown.clone();
        session_end::listen(
            || {},
            Some(session_end::Show {
                message: show_message(&key).expect("register the message"),
                on_show: Box::new(move |ask| {
                    if ask == Ask::Show {
                        count.fetch_add(1, Ordering::SeqCst);
                    }
                    true
                }),
            }),
        )
        .expect("listen");

        assert!(ask_to_show(&key).expect("ask"), "no window answered");
        assert_eq!(shown.load(Ordering::SeqCst), 1);
        // `--quick-search` asks the same window for the box, not for the main window.
        assert!(ask(&key, Ask::QuickSearch).expect("ask"), "no answer");
        assert_eq!(shown.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn asking_to_show_passes_over_the_window_of_another_home() {
        let shown = Arc::new(AtomicUsize::new(0));
        let count = shown.clone();
        session_end::listen(
            || {},
            Some(session_end::Show {
                message: show_message(&test_key("mine")).expect("register the message"),
                on_show: Box::new(move |_| {
                    count.fetch_add(1, Ordering::SeqCst);
                    true
                }),
            }),
        )
        .expect("listen");

        assert!(!ask_to_show(&test_key("theirs")).expect("ask"));
        assert_eq!(shown.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_closing_instance_does_not_count_as_shown() {
        let key = test_key("closing");
        session_end::listen(
            || {},
            Some(session_end::Show {
                message: show_message(&key).expect("register the message"),
                on_show: Box::new(|_| false),
            }),
        )
        .expect("listen");

        assert!(!ask_to_show(&key).expect("ask"));
    }
}
