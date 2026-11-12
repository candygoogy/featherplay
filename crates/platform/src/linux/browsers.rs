//! Finding Chrome, Chromium, Edge, Brave and Vivaldi on a Linux PC (`PATH`, the folders their
//! packages use, their policy files, the default browser: `ThisPc`), and starting one with the
//! DevTools pipe on descriptors 3 and 4 in a process group of its own (`launch`). The
//! browsers, and the order they're offered in, are in `common`.
//!
//! # Where each fact comes from (checked 2026-11-12)
//! - **The pipe**: `--remote-debugging-pipe` reads CDP on descriptor 3 and writes on 4
//!   (`DevToolsAgentHost::StartRemoteDebuggingPipeHandler`, content/browser/devtools/
//!   devtools_agent_host_impl.cc, `kReadFD = 3`, `kWriteFD = 4`), as Puppeteer and Playwright
//!   hand them over on Linux (`stdio: [..., 'pipe', 'pipe']`).
//! - **Commands**: each package's launcher on `PATH`: Google's .deb and .rpm put
//!   `google-chrome-stable` (and `google-chrome`) in /usr/bin, a link to the wrapper script
//!   /opt/google/chrome/google-chrome, which `exec`s the browser, so the descriptors go
//!   through (seen in WSL, Chrome 157); Microsoft's `microsoft-edge-stable` (/opt/microsoft/
//!   msedge), Brave's `brave-browser-stable` (/opt/brave.com/brave) and Vivaldi's
//!   `vivaldi-stable` (/opt/vivaldi) are built the same way. Distributions call Chromium
//!   `chromium` (Debian, Fedora, Arch, openSUSE) or `chromium-browser` (Ubuntu, older Fedora).
//! - **Policies**: Chromium reads JSON files from `/etc/{chromium,opt/chrome}/policies/managed`
//!   (Chromium's docs/enterprise/linux.md, and Chrome Enterprise help); Edge from /etc/opt/edge/policies/managed (Microsoft's "Microsoft
//!   Edge for Linux" policy docs); Brave from /etc/brave/policies/managed (Brave's Group Policy
//!   help). **Not in its documentation**: Vivaldi's /etc/opt/vivaldi/policies/managed. A wrong
//!   folder only means a block is found late: the sign-in then ends after 15 s without an
//!   answer, as for any browser.
//! - **Snap**: a Snap's command is a link to /usr/bin/snap in /snap/bin, and Ubuntu's
//!   `chromium-browser` package is a script that runs /snap/bin/chromium. Snap confinement
//!   keeps a Snap out of hidden folders in the home (the `home` interface), so it can't open
//!   the temporary profile under `~/.cache` (snapcraft.io/docs/home-interface). Not tried.
//! - **Flatpak**: an installed app's command is `exports/bin/{app id}` under
//!   /var/lib/flatpak or ~/.local/share/flatpak, a script that runs `flatpak run`. Flatpak's
//!   sandbox shows the browser only the folders its manifest names (docs.flatpak.org,
//!   "Sandbox Permissions"), and descriptors beyond 0-2 aren't documented to reach it. Not
//!   tried.
//! - **Default browser**: the first `x-scheme-handler/https` entry under `[Default
//!   Applications]` in the `mimeapps.list` files, searched as the freedesktop.org "Association
//!   between MIME types and applications" spec lists them.
//!
//! Only Google Chrome has run here (in WSL); the platform's
//! `live_browser_answers_on_the_pipe_with_a_local_pages_cookies` runs on any of them.

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read};
use std::os::fd::{AsRawFd, OwnedFd, RawFd};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

pub use crate::common::browsers::*;
use crate::window::Handle;

/// Where a browser's package puts it, and its policies.
struct Place {
    id: &'static str,
    /// Its launchers, looked for on `PATH` (then in the usual bin folders), in this order.
    commands: &'static [&'static str],
    /// Its launcher where its package puts it, for a `PATH` without it.
    installed: &'static [&'static str],
    /// The folder its JSON policy files are read from.
    policies: &'static str,
    /// Its Flatpak app id.
    flatpak: &'static str,
    /// Its desktop entries, to tell it as the default browser.
    desktop: &'static [&'static str],
}

const PLACES: [Place; 5] = [
    Place {
        id: "chrome",
        commands: &["google-chrome-stable", "google-chrome"],
        installed: &["/opt/google/chrome/google-chrome"],
        policies: "/etc/opt/chrome/policies/managed",
        flatpak: "com.google.Chrome",
        desktop: &["google-chrome.desktop", "com.google.Chrome.desktop"],
    },
    Place {
        id: "chromium",
        commands: &["chromium", "chromium-browser"],
        installed: &[],
        policies: "/etc/chromium/policies/managed",
        flatpak: "org.chromium.Chromium",
        desktop: &[
            "chromium.desktop",
            "chromium-browser.desktop",
            "chromium_chromium.desktop",
            "org.chromium.Chromium.desktop",
        ],
    },
    Place {
        id: "edge",
        commands: &["microsoft-edge-stable", "microsoft-edge"],
        installed: &["/opt/microsoft/msedge/microsoft-edge"],
        policies: "/etc/opt/edge/policies/managed",
        flatpak: "com.microsoft.Edge",
        desktop: &["microsoft-edge.desktop", "com.microsoft.Edge.desktop"],
    },
    Place {
        id: "brave",
        commands: &["brave-browser-stable", "brave-browser", "brave"],
        installed: &["/opt/brave.com/brave/brave-browser"],
        policies: "/etc/brave/policies/managed",
        flatpak: "com.brave.Browser",
        desktop: &[
            "brave-browser.desktop",
            "brave_brave.desktop",
            "com.brave.Browser.desktop",
        ],
    },
    Place {
        id: "vivaldi",
        commands: &["vivaldi-stable", "vivaldi"],
        installed: &["/opt/vivaldi/vivaldi"],
        policies: "/etc/opt/vivaldi/policies/managed",
        flatpak: "com.vivaldi.Vivaldi",
        desktop: &["vivaldi-stable.desktop", "com.vivaldi.Vivaldi.desktop"],
    },
];

fn place(browser: &Browser) -> Option<&'static Place> {
    PLACES.iter().find(|p| p.id == browser.id)
}

/// Looked in after `PATH`, which a desktop session may set without some of them.
const BIN_DIRS: [&str; 4] = ["/usr/local/bin", "/usr/bin", "/bin", "/snap/bin"];

/// The policy that turns off `--remote-debugging-pipe` (and the port) when it's false.
const DEBUGGING_POLICY: &str = "RemoteDebuggingAllowed";

/// What finding a browser reads, so tests can stand in for the PC. Only ever reads.
pub trait Lookup {
    fn env(&self, var: &str) -> Option<OsString>;
    /// A file this user may run, following links.
    fn is_file(&self, path: &Path) -> bool;
    /// Where `path` leads, its links followed.
    fn resolve(&self, path: &Path) -> Option<PathBuf>;
    /// The first few kilobytes of the file at `path`: a script's lines.
    fn head(&self, path: &Path) -> Option<Vec<u8>>;
    /// A small text file (`mimeapps.list`).
    fn read(&self, path: &Path) -> Option<String>;
    /// The text of each `.json` file in `dir`.
    fn json_files(&self, dir: &Path) -> Vec<String>;

    /// The exe of the known browser https links open with: the user's default browser.
    fn https_handler(&self) -> Option<PathBuf>
    where
        Self: Sized,
    {
        default_browser(self)
    }
}

/// This PC: the environment, the disk.
pub struct ThisPc;

impl Lookup for ThisPc {
    fn env(&self, var: &str) -> Option<OsString> {
        std::env::var_os(var).filter(|v| !v.is_empty())
    }

    fn is_file(&self, path: &Path) -> bool {
        std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }

    fn resolve(&self, path: &Path) -> Option<PathBuf> {
        std::fs::canonicalize(path).ok()
    }

    fn head(&self, path: &Path) -> Option<Vec<u8>> {
        let mut head = Vec::new();
        File::open(path)
            .ok()?
            .take(4096)
            .read_to_end(&mut head)
            .ok()?;
        Some(head)
    }

    fn read(&self, path: &Path) -> Option<String> {
        let mut text = String::new();
        File::open(path)
            .ok()?
            .take(256 * 1024)
            .read_to_string(&mut text)
            .ok()?;
        Some(text)
    }

    fn json_files(&self, dir: &Path) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        entries
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .filter_map(|p| self.read(&p))
            .collect()
    }
}

impl Browser {
    /// The browser's launcher: the first of its commands on `PATH` (then in the usual bin
    /// folders), then where its package puts it, then its Flatpak. One that isn't a Snap or a
    /// Flatpak comes before one that is.
    pub fn find(&self, pc: &impl Lookup) -> Option<PathBuf> {
        self.locate(pc).map(|(exe, _)| exe)
    }

    /// Why the sign-in can't use this browser, if it can't: a Snap or a Flatpak, or a policy
    /// that turns off the pipe.
    pub fn blocked(&self, pc: &impl Lookup) -> Option<Block> {
        let (_, sandbox) = self.locate(pc)?;
        sandbox.or_else(|| self.pipe_blocked(pc).then_some(Block::Policy))
    }

    /// A policy file sets `RemoteDebuggingAllowed` to false: the browser would open and never
    /// answer.
    pub fn pipe_blocked(&self, pc: &impl Lookup) -> bool {
        let Some(place) = place(self) else {
            return false;
        };
        pc.json_files(Path::new(place.policies)).iter().any(|text| {
            serde_json::from_str::<serde_json::Value>(text)
                .is_ok_and(|v| v.get(DEBUGGING_POLICY) == Some(&serde_json::Value::Bool(false)))
        })
    }

    /// The launcher found, and the sandbox it runs in, if any.
    fn locate(&self, pc: &impl Lookup) -> Option<(PathBuf, Option<Block>)> {
        let place = place(self)?;
        let mut sandboxed = None;
        for exe in candidates(place, pc) {
            if !pc.is_file(&exe) {
                continue;
            }
            match sandbox(&exe, pc) {
                None => return Some((exe, None)),
                Some(kind) => {
                    sandboxed.get_or_insert((exe, Some(kind)));
                }
            }
        }
        sandboxed
    }
}

/// Where `place`'s launcher may be, in the order they're tried.
fn candidates(place: &Place, pc: &impl Lookup) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = pc
        .env("PATH")
        .map(|path| std::env::split_paths(&path).collect())
        .unwrap_or_default();
    dirs.retain(|d| d.is_absolute());
    for d in BIN_DIRS {
        if !dirs.iter().any(|x| x == Path::new(d)) {
            dirs.push(d.into());
        }
    }
    let mut out: Vec<PathBuf> = place
        .commands
        .iter()
        .flat_map(|c| dirs.iter().map(move |d| d.join(c)))
        .collect();
    out.extend(place.installed.iter().map(PathBuf::from));
    out.extend(flatpak_bins(pc).into_iter().map(|d| d.join(place.flatpak)));
    out
}

/// The folders Flatpak puts its apps' commands in: the system's, then the user's.
fn flatpak_bins(pc: &impl Lookup) -> Vec<PathBuf> {
    let mut out = vec![PathBuf::from("/var/lib/flatpak/exports/bin")];
    if let Some(data) = data_home(pc) {
        out.push(data.join("flatpak/exports/bin"));
    }
    out
}

/// `XDG_DATA_HOME`, else `~/.local/share`.
fn data_home(pc: &impl Lookup) -> Option<PathBuf> {
    pc.env("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            pc.env("HOME")
                .map(|h| PathBuf::from(h).join(".local/share"))
        })
}

/// The sandbox the launcher at `exe` runs its browser in, if any.
fn sandbox(exe: &Path, pc: &impl Lookup) -> Option<Block> {
    let resolved = pc.resolve(exe).unwrap_or_else(|| exe.to_path_buf());
    for p in [exe, resolved.as_path()] {
        if p.starts_with("/snap") || p.file_name().is_some_and(|n| n == "snap") {
            return Some(Block::Snap);
        }
        if p.components().any(|c| c.as_os_str() == "flatpak") {
            return Some(Block::Flatpak);
        }
    }
    // A wrapper script that hands over to one (Ubuntu's `chromium-browser`).
    let head = pc.head(&resolved)?;
    if !head.starts_with(b"#!") {
        return None;
    }
    let text = String::from_utf8_lossy(&head);
    if text.contains("/snap/bin/") {
        Some(Block::Snap)
    } else if text.contains("flatpak run") {
        Some(Block::Flatpak)
    } else {
        None
    }
}

/// The known browsers installed on this PC, in the order of `ALL`.
pub fn found(pc: &impl Lookup) -> Vec<Found> {
    ALL.into_iter()
        .filter_map(|browser| {
            let exe = browser.find(pc)?;
            let blocked = browser.blocked(pc);
            Some(Found {
                browser,
                exe,
                blocked,
            })
        })
        .collect()
}

/// The launcher of the known browser that `mimeapps.list` opens https links with.
fn default_browser(pc: &impl Lookup) -> Option<PathBuf> {
    let id = default_https_entry(pc)?;
    let place = PLACES.iter().find(|p| p.desktop.contains(&id.as_str()))?;
    by_id(place.id)?.find(pc)
}

/// The desktop entry https links open with: the first `mimeapps.list` that names one, in the
/// spec's order (the user's config, the system's config, the user's data, the system's data;
/// in each, the desktop's own `{desktop}-mimeapps.list` first).
fn default_https_entry(pc: &impl Lookup) -> Option<String> {
    let home = pc.env("HOME").map(PathBuf::from);
    let list = |var: &str, default: &str| -> Vec<PathBuf> {
        let value = pc.env(var).unwrap_or_else(|| default.into());
        std::env::split_paths(&value)
            .filter(|p| p.is_absolute())
            .collect()
    };
    let config_home = pc
        .env("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home.as_ref().map(|h| h.join(".config")));
    let mut dirs: Vec<PathBuf> = config_home.into_iter().collect();
    dirs.extend(list("XDG_CONFIG_DIRS", "/etc/xdg"));
    dirs.extend(data_home(pc).map(|d| d.join("applications")));
    dirs.extend(
        list("XDG_DATA_DIRS", "/usr/local/share:/usr/share")
            .into_iter()
            .map(|d| d.join("applications")),
    );
    let desktops: Vec<String> = pc
        .env("XDG_CURRENT_DESKTOP")
        .map(|d| {
            d.to_string_lossy()
                .split(':')
                .filter(|s| !s.is_empty())
                .map(|s| format!("{}-mimeapps.list", s.to_lowercase()))
                .collect()
        })
        .unwrap_or_default();
    let names: Vec<&str> = desktops
        .iter()
        .map(String::as_str)
        .chain(["mimeapps.list"])
        .collect();
    dirs.iter()
        .flat_map(|d| names.iter().map(move |n| d.join(n)))
        .find_map(|file| https_entry(&pc.read(&file)?))
}

/// The first desktop entry for `x-scheme-handler/https` under `[Default Applications]`.
fn https_entry(mimeapps: &str) -> Option<String> {
    let mut defaults = false;
    for line in mimeapps.lines().map(str::trim) {
        if line.starts_with('[') {
            defaults = line == "[Default Applications]";
        } else if defaults
            && let Some((key, value)) = line.split_once('=')
            && key.trim() == "x-scheme-handler/https"
        {
            return value
                .split(';')
                .map(str::trim)
                .find(|v| !v.is_empty())
                .map(str::to_owned);
        }
    }
    None
}

/// The descriptors Chromium's pipe handler uses.
const READ_FD: RawFd = 3;
const WRITE_FD: RawFd = 4;

/// The running browser, the leader of its own process group. Dropping it ends the group:
/// the browser and every helper it started.
pub struct Running {
    child: Mutex<Child>,
    pub pid: u32,
}

impl Running {
    /// The browser's own process has exited (its helpers may linger a moment).
    pub fn exited(&self) -> bool {
        let mut child = self.child.lock().unwrap_or_else(PoisonError::into_inner);
        !matches!(child.try_wait(), Ok(None))
    }

    /// Waits up to `limit` for the browser's process to exit; says whether it did.
    pub fn wait(&self, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        loop {
            if self.exited() {
                return true;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            std::thread::sleep(left.min(Duration::from_millis(20)));
        }
    }

    /// Ends the browser and every process in its group.
    pub fn kill(&self) {
        let pgid = libc::pid_t::try_from(self.pid).unwrap_or(0);
        if pgid > 0 {
            // SAFETY: a signal to the process group our child leads (`process_group(0)`); no
            // memory is touched. A group that's gone already is ESRCH, which is fine.
            unsafe { libc::killpg(pgid, libc::SIGKILL) };
        }
        let mut child = self.child.lock().unwrap_or_else(PoisonError::into_inner);
        // Reaps it, so no zombie stays; it has been sent SIGKILL, so this doesn't wait long.
        let _ = child.wait();
    }
}

/// Dropping a running browser ends it, as on Windows (whose job ends with its handle): callers
/// drop it on purpose.
impl Drop for Running {
    fn drop(&mut self) {
        self.kill();
    }
}

/// The app's ends of the two pipes.
pub struct Pipes {
    /// Commands go in here; the browser reads them on its descriptor 3.
    pub to_browser: File,
    /// The browser writes on its descriptor 4; replies come out here.
    pub from_browser: File,
}

/// Starts `exe` with `args` and the DevTools pipe: its descriptors 3 and 4 are the ends of two
/// pipes, and nothing else of the app's is inherited (every descriptor of the app's is
/// close-on-exec; its standard ones are /dev/null). It leads a process group of its own, which
/// `Running` ends, and it's killed if the thread that started it ends first (the app crashed
/// or was killed): the sign-in thread starts it, watches it and ends it.
pub fn launch(exe: &Path, args: &[OsString]) -> io::Result<(Running, Pipes)> {
    // Close-on-exec, both: no other child of the app's gets a copy.
    let (browser_read, to_browser) = io::pipe()?;
    let (from_browser, browser_write) = io::pipe()?;
    let (read, write) = (browser_read.as_raw_fd(), browser_write.as_raw_fd());
    let mut command = Command::new(exe);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    // SAFETY: `hand_over` only calls fcntl, dup2 and prctl, which are async-signal-safe, and
    // touches no memory of the parent's; `read` and `write` stay open in the parent until
    // `spawn` returns.
    unsafe {
        command.pre_exec(move || hand_over(read, write));
    }
    let started = command.spawn();
    // The browser has its own copies now; ours would keep the pipe open after it exits.
    drop(browser_read);
    drop(browser_write);
    let child = started?;
    let pid = child.id();
    Ok((
        Running {
            child: Mutex::new(child),
            pid,
        },
        Pipes {
            to_browser: File::from(OwnedFd::from(to_browser)),
            from_browser: File::from(OwnedFd::from(from_browser)),
        },
    ))
}

/// In the child, between fork and exec: puts the browser's pipe ends on 3 and 4.
fn hand_over(read: RawFd, write: RawFd) -> io::Result<()> {
    let check = |r: libc::c_int| {
        if r == -1 {
            Err(io::Error::last_os_error())
        } else {
            Ok(r)
        }
    };
    // Copies above 4 first, so placing one can't overwrite the other (either may be 3 or 4).
    // They're close-on-exec, so they go at the exec.
    // SAFETY: fcntl on descriptors that are open in this process.
    let read = check(unsafe { libc::fcntl(read, libc::F_DUPFD_CLOEXEC, 10) })?;
    // SAFETY: as above.
    let write = check(unsafe { libc::fcntl(write, libc::F_DUPFD_CLOEXEC, 10) })?;
    // dup2's copy isn't close-on-exec: 3 and 4 survive the exec.
    // SAFETY: dup2 onto 3 and 4, from descriptors that are open.
    check(unsafe { libc::dup2(read, READ_FD) })?;
    // SAFETY: as above.
    check(unsafe { libc::dup2(write, WRITE_FD) })?;
    // SAFETY: prctl with two integer arguments; it sets this process's own death signal.
    check(unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) })?;
    Ok(())
}

/// The visible top-level windows of the process `pid`: none are looked for on Linux, where a
/// window's process isn't the app's to ask (X11 only knows it from a hint, Wayland not at all).
pub fn visible_windows(_pid: u32) -> Vec<Handle> {
    Vec::new()
}

/// Would ask `window` to close; there is none (see [`visible_windows`]).
pub fn close_window(_window: Handle) {}

#[cfg(test)]
mod tests;
