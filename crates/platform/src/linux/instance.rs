//! One featherplay per home: a lock file that a second launch finds held, and a Unix socket
//! it asks the running one to show its window through. The app's `instance::start` runs them
//! first in a windowed run, before any window, tray icon, hotkey or store.
//!
//! Both live in `XDG_RUNTIME_DIR` (the user's own, mode 0700, emptied at logout); without it,
//! in a folder of the user's own under the temporary folder. One pair per home, as on Windows:
//! a run with `FEATHERPLAY_HOME` set derives its names from that home.
//!
//! The guard is an exclusive `flock` on `featherplay-<key>.lock`. The kernel lets go of it
//! when the process ends, however it ends, so a crashed first instance never blocks the next
//! launch (Windows' abandoned mutex). The socket, `featherplay-<key>.sock`, is made by
//! `session_end::listen` (the place Windows answers a second launch from); a file left by a
//! crash refuses connections, so `listen` replaces it.
//!
//! The second launch writes `show` and reads one byte: 1 once the first has asked its window
//! to show, 0 if it's closing. The window then shows and asks for the focus, which X11 gives;
//! Wayland lets the compositor decide, and most only mark the window as wanting attention.

use std::ffi::OsStr;
use std::fs::{DirBuilder, File, OpenOptions};
use std::io::{self, Read, Write};
use std::marker::PhantomData;
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::common::identity::AUMID;
use crate::common::instance::fnv1a;
use crate::common::session_end::Show;

/// How long one side waits for the other's request or answer.
const ASK_TIMEOUT: Duration = Duration::from_secs(2);
/// How often `Running::wait` tries the lock again.
const POLL: Duration = Duration::from_millis(25);
const REQUEST: &[u8] = b"show\n";

/// The guard's key: the app ID, plus a hash of the home when `FEATHERPLAY_HOME` is set (an
/// empty one counts as unset). It holds no path separator.
pub fn key(home: Option<&OsStr>) -> String {
    match home.filter(|h| !h.is_empty()) {
        None => AUMID.to_string(),
        Some(home) => format!("{AUMID}-home-{:016x}", fnv1a(normalised(home).as_bytes())),
    }
}

/// The guard's lock file for `key`.
pub fn mutex_name(key: &str) -> String {
    file_for(key, "lock").to_string_lossy().into_owned()
}

/// The socket a second launch of `key` asks through.
fn socket_for(key: &str) -> PathBuf {
    file_for(key, "sock")
}

fn file_for(key: &str, extension: &str) -> PathBuf {
    runtime_dir().join(format!("featherplay-{key}.{extension}"))
}

/// `XDG_RUNTIME_DIR` when it's set to an absolute path; else `featherplay-<uid>` in the
/// temporary folder, which [`private_dir`] makes the user's own.
fn runtime_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from) {
        Some(dir) if dir.is_absolute() => dir,
        _ => {
            // SAFETY: getuid has no preconditions and can't fail.
            let uid = unsafe { libc::getuid() };
            std::env::temp_dir().join(format!("featherplay-{uid}"))
        }
    }
}

/// Makes sure `dir` exists and only its user may enter it: made with mode 0700 if missing.
/// One that someone else owns, or that others may enter, is refused (another user could
/// plant a socket there). `XDG_RUNTIME_DIR` passes as it is.
fn private_dir(dir: &Path) -> io::Result<()> {
    if std::env::var_os("XDG_RUNTIME_DIR").is_some_and(|d| Path::new(&d) == dir) {
        return Ok(());
    }
    match DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => return Ok(()),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    let meta = std::fs::symlink_metadata(dir)?;
    // SAFETY: getuid has no preconditions and can't fail.
    let uid = unsafe { libc::getuid() };
    if !meta.is_dir() || meta.uid() != uid || meta.mode() & 0o077 != 0 {
        return Err(io::Error::other(format!(
            "{} isn't a private folder of this user's",
            dir.display()
        )));
    }
    Ok(())
}

/// The keys of the `show_message`s registered so far; a message is its key's index plus one.
static MESSAGES: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// What a second launch's request is known by in `session_end::Show`: a number for `key`,
/// which `session_end::listen` turns back into the key's socket.
pub fn show_message(key: &str) -> Result<u32, String> {
    let mut keys = MESSAGES
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let index = match keys.iter().position(|k| k == key) {
        Some(i) => i,
        None => {
            keys.push(key.to_string());
            keys.len() - 1
        }
    };
    u32::try_from(index + 1).map_err(|_| "too many show messages".into())
}

/// The absolute form of the path, without a trailing separator.
fn normalised(home: &OsStr) -> String {
    let path = std::path::absolute(home).unwrap_or_else(|_| PathBuf::from(home));
    let text = path.to_string_lossy().into_owned();
    text.trim_end_matches('/').to_string()
}

/// Takes the exclusive lock on `file` without waiting: true if taken, false if another holds it.
fn try_lock(file: &File) -> io::Result<bool> {
    // SAFETY: flock on a descriptor this File owns.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(true);
    }
    let e = io::Error::last_os_error();
    if e.raw_os_error() == Some(libc::EWOULDBLOCK) {
        Ok(false)
    } else {
        Err(e)
    }
}

/// Holding the lock: this run is the one featherplay of its home. Kept on the thread that
/// took it until the process ends, as on Windows (it isn't `Send` there either). Dropping it
/// removes the socket and lets go.
pub struct Guard {
    // Held for its lock, which closing it lets go of.
    _lock: File,
    socket: PathBuf,
    _owning_thread: PhantomData<*const ()>,
}

impl Guard {
    fn new(lock: File, name: &str) -> Guard {
        Guard {
            _lock: lock,
            socket: Path::new(name).with_extension("sock"),
            _owning_thread: PhantomData,
        }
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        // Before `_lock` closes (the fields drop after this), so it's never another run's.
        let _ = std::fs::remove_file(&self.socket);
    }
}

/// Another featherplay of this home holds the lock.
pub struct Running {
    lock: File,
    name: String,
}

impl Running {
    /// Waits up to `timeout` for the other to let go (it quit, or its process ended), and
    /// takes the lock over; gives itself back if the other still holds it.
    pub fn wait(self, timeout: Duration) -> Result<Guard, Running> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Ok(true) = try_lock(&self.lock) {
                return Ok(Guard::new(self.lock, &self.name));
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(self);
            }
            std::thread::sleep(left.min(POLL));
        }
    }
}

/// Whether this run is the first of its home.
pub enum Claim {
    First(Guard),
    Running(Running),
}

/// Opens the lock file `name` (made if missing) and takes its lock, or finds another
/// featherplay holding it.
pub fn claim(name: &str) -> Result<Claim, String> {
    let path = Path::new(name);
    if let Some(dir) = path.parent() {
        private_dir(dir).map_err(|e| format!("the lock's folder {}: {e}", dir.display()))?;
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
        .map_err(|e| format!("open the lock {name}: {e}"))?;
    match try_lock(&lock) {
        Ok(true) => Ok(Claim::First(Guard::new(lock, name))),
        Ok(false) => Ok(Claim::Running(Running {
            lock,
            name: name.to_string(),
        })),
        Err(e) => Err(format!("lock {name}: {e}")),
    }
}

/// Asks the featherplay of this key to show its window. True once one answered that it
/// did; false if none did (none is running, it is still starting, or it is closing).
pub fn ask_to_show(key: &str) -> Result<bool, String> {
    let path = socket_for(key);
    let mut stream = match UnixStream::connect(&path) {
        Ok(s) => s,
        // No socket yet (still starting), or one a crash left.
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Ok(false);
        }
        Err(e) => return Err(format!("connect to {}: {e}", path.display())),
    };
    let answer = (|| {
        stream.set_read_timeout(Some(ASK_TIMEOUT))?;
        stream.set_write_timeout(Some(ASK_TIMEOUT))?;
        stream.write_all(REQUEST)?;
        let mut byte = [0u8];
        stream.read_exact(&mut byte)?;
        io::Result::Ok(byte[0])
    })();
    // A first instance that hung or went away mid-answer didn't show anything.
    Ok(answer.is_ok_and(|b| b == 1))
}

/// Answers second launches on `show`'s socket, on a thread of its own, for the rest of the
/// run. Called by `session_end::listen`.
pub(crate) fn serve(show: Show) -> Result<(), String> {
    let key = {
        let keys = MESSAGES
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let index = (show.message as usize).checked_sub(1);
        index.and_then(|i| keys.get(i).cloned())
    }
    .ok_or("single instance: not a registered show message")?;
    let path = socket_for(&key);
    if let Some(dir) = path.parent() {
        private_dir(dir).map_err(|e| format!("single instance: {e}"))?;
    }
    let listener = bind(&path).map_err(|e| format!("single instance: {}: {e}", path.display()))?;
    std::thread::Builder::new()
        .name("featherplay-instance".into())
        .spawn(move || {
            for stream in listener.incoming() {
                match stream {
                    Ok(stream) => answer(stream, &show),
                    Err(e) => log::warn!("single instance: accept: {e}"),
                }
            }
        })
        .map(|_| ())
        .map_err(|e| format!("single instance: thread: {e}"))
}

/// Binds the socket at `path`, replacing a file a crashed run left (nothing answers on it).
fn bind(path: &Path) -> io::Result<UnixListener> {
    match UnixListener::bind(path) {
        Err(e) if e.kind() == io::ErrorKind::AddrInUse => {
            if UnixStream::connect(path).is_ok() {
                // A live one: another run without the guard (it couldn't be had) is serving.
                return Err(e);
            }
            std::fs::remove_file(path)?;
            UnixListener::bind(path)
        }
        bound => bound,
    }
}

/// Reads one request from `stream` and answers it.
fn answer(mut stream: UnixStream, show: &Show) {
    let _ = stream.set_read_timeout(Some(ASK_TIMEOUT));
    let _ = stream.set_write_timeout(Some(ASK_TIMEOUT));
    let mut request = [0u8; REQUEST.len()];
    // Anything else (a probe that connected and left, say) isn't a request.
    if stream.read_exact(&mut request).is_err() || request != REQUEST {
        return;
    }
    let shown = (show.on_show)();
    let _ = stream.write_all(&[u8::from(shown)]);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    /// A home no featherplay runs with, so the test's files are its own.
    fn test_key(test: &str) -> String {
        key(Some(OsStr::new(&format!(
            "/nonexistent/featherplay-test-{test}-{}",
            std::process::id()
        ))))
    }

    #[test]
    fn a_second_claim_finds_the_first_and_takes_over_once_it_lets_go() {
        let name = mutex_name(&test_key("claim"));
        let Ok(Claim::First(first)) = claim(&name) else {
            panic!("the first claim should run");
        };
        let Ok(Claim::Running(running)) = claim(&name) else {
            panic!("the second claim should find the first running");
        };
        let Err(running) = running.wait(Duration::from_millis(50)) else {
            panic!("taken while the first still holds it");
        };
        // As when the first ends, however it ends: the kernel lets go of its lock.
        drop(first);
        let guard = running.wait(Duration::from_secs(2));
        assert!(guard.is_ok(), "not taken over once the first let go");
        drop(guard);
        let _ = std::fs::remove_file(&name);
    }

    #[test]
    fn asking_to_show_reaches_the_first_past_a_crashed_runs_socket() {
        let key = test_key("show");
        let path = socket_for(&key);
        // What a crash leaves: the socket file, with nothing listening.
        drop(UnixListener::bind(&path).unwrap());
        assert!(path.exists());
        assert!(!ask_to_show(&key).unwrap(), "a stale socket answered");

        let shown = Arc::new(AtomicUsize::new(0));
        let count = shown.clone();
        serve(Show {
            message: show_message(&key).unwrap(),
            on_show: Box::new(move || {
                count.fetch_add(1, Ordering::SeqCst);
                true
            }),
        })
        .unwrap();
        assert!(ask_to_show(&key).unwrap(), "no answer");
        assert_eq!(shown.load(Ordering::SeqCst), 1);
        // Another home's launch doesn't reach it.
        assert!(!ask_to_show(&test_key("other")).unwrap());
        let _ = std::fs::remove_file(&path);
    }
}
