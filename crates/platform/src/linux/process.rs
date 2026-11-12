//! Child processes, ended with the app. A helper (yt-dlp, deno, featherplay's own
//! preprocessing child) is tied to the app as it starts: [`end_with_app`] has the kernel send
//! it `SIGTERM` when its parent goes (`PR_SET_PDEATHSIG`), however the app ends: a quit, a
//! crash, or `SIGKILL`. A process group killed at exit would miss the last two.
//!
//! The kernel's "parent" there is the thread that started the child, not the process: a
//! child started on a thread that ends first (a blocking-pool thread, idle for a few
//! seconds) would be ended with it, mid-run. So tied children are started through
//! [`spawn_tied`], on one thread that lives as long as the app.
//!
//! `SIGTERM` rather than `SIGKILL`: yt-dlp's Linux build is a PyInstaller bundle, whose
//! launcher passes `SIGTERM` on to the Python process it runs, which a `SIGKILL` would leave
//! behind. What a helper starts in turn (yt-dlp's deno) isn't tied: it ends when its parent's
//! pipes close.
//!
//! The sign-in browser is tied the same way, by `browsers::launch`, and ended with its
//! process group.

use std::io;
use std::os::unix::process::CommandExt;
use std::panic::{self, AssertUnwindSafe};
use std::process::Command;
use std::sync::OnceLock;
use std::sync::mpsc::{self, Sender};

/// Starts `cmd`'s process without a console window. Linux opens none, so nothing to do.
pub fn no_console_window(_cmd: &mut Command) {}

/// Ties `cmd`'s process to the app: it gets `SIGTERM` when the app ends, however it ends.
/// Start it inside [`spawn_tied`], or it ends with the thread that started it.
pub fn end_with_app(cmd: &mut Command) {
    let parent = std::process::id();
    // SAFETY: the closure only calls prctl and getppid, which are async-signal-safe, and
    // touches no memory of the parent's.
    unsafe {
        cmd.pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM) == -1 {
                return Err(io::Error::last_os_error());
            }
            // The app ended between the fork and the prctl: nothing will send the signal.
            if u32::try_from(libc::getppid()).ok() != Some(parent) {
                return Err(io::Error::from_raw_os_error(libc::ESRCH));
            }
            Ok(())
        });
    }
}

type Work = Box<dyn FnOnce() + Send>;

/// The thread tied children start on, started on first use; None if it couldn't be.
fn spawner() -> Option<&'static Sender<Work>> {
    static SPAWNER: OnceLock<Option<Sender<Work>>> = OnceLock::new();
    SPAWNER
        .get_or_init(|| {
            let (tx, rx) = mpsc::channel::<Work>();
            std::thread::Builder::new()
                .name("featherplay-spawner".into())
                .spawn(move || {
                    // Never returns while the app runs: the sender lives in the static.
                    for job in rx {
                        job();
                    }
                })
                .inspect_err(|e| log::warn!("the spawner thread: {e}"))
                .ok()
                .map(|_| tx)
        })
        .as_ref()
}

/// Runs `spawn`, which starts a child made with [`end_with_app`], on a thread that lives as
/// long as the app, and returns what it returned. Blocks until it has run. A panic in `spawn`
/// comes back to the caller.
pub fn spawn_tied<T: Send + 'static>(spawn: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::sync_channel(1);
    let job: Work = Box::new(move || {
        let _ = tx.send(panic::catch_unwind(AssertUnwindSafe(spawn)));
    });
    match spawner() {
        Some(spawner) => {
            if let Err(mpsc::SendError(job)) = spawner.send(job) {
                job();
            }
        }
        // No thread for it: here, which is right as long as this thread outlives the child.
        None => job(),
    }
    match rx.recv() {
        Ok(Ok(value)) => value,
        Ok(Err(panicked)) => panic::resume_unwind(panicked),
        Err(_) => unreachable!("the job always answers"),
    }
}

/// What names a started child to [`kill_on_close`]: its process id, as
/// `tokio::process::Child::id` gives it.
pub type ChildHandle = u32;

/// What ends the child with the app on Windows (a job object). Holds nothing here: the
/// child was tied as it started ([`end_with_app`]).
pub struct Job(());

/// Nothing more to do here: a child made with [`end_with_app`] is tied already.
pub fn kill_on_close(_process: ChildHandle) -> io::Result<Job> {
    Ok(Job(()))
}

#[cfg(test)]
mod tests;
