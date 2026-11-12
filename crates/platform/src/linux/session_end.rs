//! The end of the session: `SIGTERM` (what the desktop, systemd or `kill` sends to end a
//! program) and `SIGHUP` (its terminal or session went away) ask the app to close, as
//! Windows' end-of-session messages do, so the session is saved first. A second signal
//! ends the process at once. Ctrl+C (`SIGINT`) is the controller's own.
//!
//! Second launches aren't heard here yet: `listen` drops its `Show`.

use std::sync::atomic::{AtomicI32, Ordering};

pub use crate::common::session_end::*;

/// The pipe's write end the signal handler writes to; -1 before `listen`.
static WAKE: AtomicI32 = AtomicI32::new(-1);

const SIGNALS: [libc::c_int; 2] = [libc::SIGTERM, libc::SIGHUP];

extern "C" fn on_signal(_signal: libc::c_int) {
    let fd = WAKE.load(Ordering::Relaxed);
    if fd >= 0 {
        let byte = 1u8;
        // SAFETY: `write` is async-signal-safe; the buffer is a local byte. A full pipe
        // drops the byte, and one is enough.
        let _ = unsafe { libc::write(fd, (&raw const byte).cast(), 1) };
    }
}

/// Calls `on_end` (once, on a thread of its own) when the process gets `SIGTERM` or
/// `SIGHUP`. `show` isn't used yet.
pub fn listen(on_end: impl FnOnce() + Send + 'static, _show: Option<Show>) -> Result<(), String> {
    let mut fds = [0 as libc::c_int; 2];
    // SAFETY: a valid out array of two descriptors.
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(format!(
            "session end: pipe: {}",
            std::io::Error::last_os_error()
        ));
    }
    let [read, write] = fds;
    WAKE.store(write, Ordering::Relaxed);
    for signal in SIGNALS {
        set_handler(signal, on_signal as *const () as libc::sighandler_t)
            .map_err(|e| format!("session end: signal {signal}: {e}"))?;
    }
    std::thread::Builder::new()
        .name("featherplay-session-end".into())
        .spawn(move || {
            let mut byte = 0u8;
            loop {
                // SAFETY: our pipe's read end and a local one-byte buffer.
                let n = unsafe { libc::read(read, (&raw mut byte).cast(), 1) };
                if n == 1 {
                    break;
                }
                if n == 0
                    || std::io::Error::last_os_error().kind() != std::io::ErrorKind::Interrupted
                {
                    return;
                }
            }
            // From here a second signal ends the process, as it would without featherplay's
            // handler.
            for signal in SIGNALS {
                let _ = set_handler(signal, libc::SIG_DFL);
            }
            on_end();
        })
        .map(|_| ())
        .map_err(|e| format!("session end: thread: {e}"))
}

fn set_handler(signal: libc::c_int, handler: libc::sighandler_t) -> std::io::Result<()> {
    // SAFETY: an all-zero `sigaction` is a valid value (no flags, an empty mask) before the
    // fields are set.
    let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
    action.sa_sigaction = handler;
    action.sa_flags = libc::SA_RESTART;
    // SAFETY: a valid action; the handler only does async-signal-safe work.
    if unsafe { libc::sigaction(signal, &action, std::ptr::null_mut()) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Nothing waits for it: the process ends when `main` returns.
pub fn saved() {}
