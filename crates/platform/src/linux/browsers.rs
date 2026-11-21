//! Finding Chrome, Chromium, Firefox, Edge, Brave and Vivaldi on a Linux PC (`PATH`, the
//! folders their packages use, their Snap or Flatpak, the default browser: `ThisPc`), and
//! starting one in a process group of its own, plainly, for the user to sign in
//! (`launch_plain`). Once the user has closed it, the app reads the cookies it kept from its
//! profile on disk (`cookies`); nothing talks to the running browser. The browsers, and the
//! order they're offered in, are in `common`.
//!
//! # Where each fact comes from (checked 2026-11-12, Firefox and the sandboxes 2026-11-21)
//! - **Commands**: each package's launcher on `PATH`: Google's .deb and .rpm put
//!   `google-chrome-stable` (and `google-chrome`) in /usr/bin, a link to the wrapper script
//!   /opt/google/chrome/google-chrome, which `exec`s the browser; Microsoft's
//!   `microsoft-edge-stable` (/opt/microsoft/msedge), Brave's `brave-browser-stable`
//!   (/opt/brave.com/brave) and Vivaldi's `vivaldi-stable` (/opt/vivaldi) are built the same
//!   way. Distributions call Chromium `chromium` (Debian, Fedora, Arch, openSUSE) or
//!   `chromium-browser` (Ubuntu, older Fedora). Firefox is `firefox` (Mozilla's .deb, Fedora,
//!   Arch) or `firefox-esr` (Debian, Mozilla's ESR .deb); with both of Mozilla's installed,
//!   /usr/bin/firefox is a script that `exec`s one of them (seen in WSL, Ubuntu 26.04).
//! - **One browser process per profile**: Chromium keeps one per `--user-data-dir`
//!   (`chrome/browser/process_singleton_posix.cc`), and Firefox's `--new-instance` starts its
//!   own on `--profile`, so a start on the temporary profile never hands over to a browser the
//!   user has open. Each locks the profile while it runs (`ProfileLock`).
//! - **Snap**: a Snap's command is a link to /usr/bin/snap in /snap/bin, and Ubuntu's
//!   `chromium-browser` package is a script that runs /snap/bin/chromium. Snap confinement
//!   keeps a Snap out of hidden folders in the home (the `home` interface,
//!   snapcraft.io/docs/home-interface), but every Snap may write its `$SNAP_USER_COMMON`,
//!   ~/snap/{name}/common (snapcraft.io/docs/environment-variables): the temporary profile goes
//!   there (`Sandbox::profiles`). It runs in the host's process namespace, so its lock names
//!   its real pid.
//! - **Flatpak**: an installed app's command is `exports/bin/{app id}` under
//!   /var/lib/flatpak or ~/.local/share/flatpak, a script that runs `flatpak run`. The sandbox
//!   shows the browser only the folders its manifest names (docs.flatpak.org, "Sandbox
//!   Permissions"), but always its own ~/.var/app/{app id}, at the same path: the profile goes
//!   in its `cache` folder there. `flatpak run` waits for the browser, and the browser has a
//!   process namespace of its own, so its lock names the pid it has inside it.
//!   All four (Chromium and Firefox, as Snaps and as Flatpaks) signed in to a local page in
//!   WSL and were read (2026-11-21), but WSL has no AppArmor: snapd ran them with partial
//!   confinement only.
//! - **Default browser**: the first `x-scheme-handler/https` entry under `[Default
//!   Applications]` in the `mimeapps.list` files, searched as the freedesktop.org "Association
//!   between MIME types and applications" spec lists them.
//!
//! Google Chrome and Firefox (Mozilla's .deb, ESR too) have run here (in WSL), with the
//! Snaps and Flatpaks above; the platform's `live::live_browsers_sign_in_plainly` runs on
//! every one found.

use std::ffi::OsString;
use std::fs::File;
use std::io::{self, Read};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

pub use crate::common::browsers::*;
use crate::window::Handle;

pub mod cookies;

/// Where a browser's package puts it.
struct Place {
    id: &'static str,
    /// Its launchers, looked for on `PATH` (then in the usual bin folders), in this order.
    commands: &'static [&'static str],
    /// Its launcher where its package puts it, for a `PATH` without it.
    installed: &'static [&'static str],
    /// Its Flatpak app id.
    flatpak: &'static str,
    /// Its desktop entries, to tell it as the default browser.
    desktop: &'static [&'static str],
}

const PLACES: [Place; 6] = [
    Place {
        id: "chrome",
        commands: &["google-chrome-stable", "google-chrome"],
        installed: &["/opt/google/chrome/google-chrome"],
        flatpak: "com.google.Chrome",
        desktop: &["google-chrome.desktop", "com.google.Chrome.desktop"],
    },
    Place {
        id: "chromium",
        commands: &["chromium", "chromium-browser"],
        installed: &[],
        flatpak: "org.chromium.Chromium",
        desktop: &[
            "chromium.desktop",
            "chromium-browser.desktop",
            "chromium_chromium.desktop",
            "org.chromium.Chromium.desktop",
        ],
    },
    Place {
        id: "firefox",
        commands: &["firefox", "firefox-esr"],
        installed: &[],
        flatpak: "org.mozilla.firefox",
        desktop: &[
            "firefox.desktop",
            "firefox-esr.desktop",
            "firefox_firefox.desktop",
            "org.mozilla.firefox.desktop",
        ],
    },
    Place {
        id: "edge",
        commands: &["microsoft-edge-stable", "microsoft-edge"],
        installed: &["/opt/microsoft/msedge/microsoft-edge"],
        flatpak: "com.microsoft.Edge",
        desktop: &["microsoft-edge.desktop", "com.microsoft.Edge.desktop"],
    },
    Place {
        id: "brave",
        commands: &["brave-browser-stable", "brave-browser", "brave"],
        installed: &["/opt/brave.com/brave/brave-browser"],
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
        flatpak: "com.vivaldi.Vivaldi",
        desktop: &["vivaldi-stable.desktop", "com.vivaldi.Vivaldi.desktop"],
    },
];

fn place(browser: &Browser) -> Option<&'static Place> {
    PLACES.iter().find(|p| p.id == browser.id)
}

/// Looked in after `PATH`, which a desktop session may set without some of them.
const BIN_DIRS: [&str; 4] = ["/usr/local/bin", "/usr/bin", "/bin", "/snap/bin"];

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
}

/// The sandbox a browser's launcher runs it in, which decides where its profile can be.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sandbox {
    None,
    /// A Snap, by its name (`chromium`, or `chromium_beta` for a second install of it).
    Snap(String),
    /// A Flatpak, by its app id.
    Flatpak(&'static str),
}

impl Sandbox {
    /// The folder the browser's temporary profile goes in, which the sandbox lets it reach:
    /// the Snap's ~/snap/{name}/common, the Flatpak's ~/.var/app/{app id}/cache. `None` for a
    /// browser outside a sandbox (its profile goes in the app's cache), or with no `HOME`.
    pub fn profiles(&self, pc: &impl Lookup) -> Option<PathBuf> {
        let home = PathBuf::from(pc.env("HOME")?);
        match self {
            Sandbox::None => None,
            Sandbox::Snap(name) => Some(home.join("snap").join(name).join("common")),
            Sandbox::Flatpak(id) => Some(home.join(".var/app").join(id).join("cache")),
        }
    }
}

/// The folders a sandboxed browser's temporary profile may have been put in: every Snap's
/// ~/snap/*/common and every Flatpak's ~/.var/app/*/cache that's there, for the sweep at
/// start-up.
pub fn sandbox_profile_folders() -> Vec<PathBuf> {
    let Some(home) = ThisPc.env("HOME").map(PathBuf::from) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (dir, inner) in [("snap", "common"), (".var/app", "cache")] {
        let Ok(entries) = std::fs::read_dir(home.join(dir)) else {
            continue;
        };
        for entry in entries.flatten() {
            // `file_type` doesn't follow links: only real folders are looked in.
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                out.push(entry.path().join(inner));
            }
        }
    }
    out
}

/// A browser's launcher as found, and the sandbox it runs in.
#[derive(Debug, PartialEq, Eq)]
pub struct Located {
    pub exe: PathBuf,
    pub sandbox: Sandbox,
}

impl Browser {
    /// The browser's launcher: the first of its commands on `PATH` (then in the usual bin
    /// folders), then where its package puts it, then its Flatpak. One that isn't a Snap or a
    /// Flatpak comes before one that is.
    pub fn find(&self, pc: &impl Lookup) -> Option<PathBuf> {
        self.locate(pc).map(|l| l.exe)
    }

    /// The launcher `find` gives, and the sandbox it runs in.
    pub fn locate(&self, pc: &impl Lookup) -> Option<Located> {
        let place = place(self)?;
        let mut sandboxed = None;
        for exe in candidates(place, pc) {
            if !pc.is_file(&exe) {
                continue;
            }
            match sandbox(&exe, place, pc) {
                Sandbox::None => {
                    return Some(Located {
                        exe,
                        sandbox: Sandbox::None,
                    });
                }
                sandbox => {
                    sandboxed.get_or_insert(Located { exe, sandbox });
                }
            }
        }
        sandboxed
    }

    /// The sandbox the launcher at `exe` runs this browser in.
    pub fn sandbox_of(&self, exe: &Path, pc: &impl Lookup) -> Sandbox {
        place(self).map_or(Sandbox::None, |p| sandbox(exe, p, pc))
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

/// The sandbox the launcher at `exe` (one of `place`'s) runs its browser in.
fn sandbox(exe: &Path, place: &Place, pc: &impl Lookup) -> Sandbox {
    let resolved = pc.resolve(exe).unwrap_or_else(|| exe.to_path_buf());
    for p in [exe, resolved.as_path()] {
        // /snap/bin/{command}: a Snap's app is `{name}.{app}`, or just `{name}`.
        if let Ok(command) = p.strip_prefix("/snap/bin")
            && let Some(name) = snap_name(command.as_os_str().as_bytes())
        {
            return Sandbox::Snap(name);
        }
        if p.components().any(|c| c.as_os_str() == "flatpak") {
            return Sandbox::Flatpak(place.flatpak);
        }
    }
    // A wrapper script that hands over to one (Ubuntu's `chromium-browser`).
    let Some(head) = pc.head(&resolved) else {
        return Sandbox::None;
    };
    if !head.starts_with(b"#!") {
        return Sandbox::None;
    }
    if let Some(at) = find(&head, b"/snap/bin/")
        && let Some(name) = snap_name(&head[at + b"/snap/bin/".len()..])
    {
        Sandbox::Snap(name)
    } else if find(&head, b"flatpak run").is_some() {
        Sandbox::Flatpak(place.flatpak)
    } else {
        Sandbox::None
    }
}

/// The Snap's name at the start of `command`: up to the first character a name can't have
/// (snapd's names are lower-case letters, digits and `-`; a second install adds `_{key}`).
fn snap_name(command: &[u8]) -> Option<String> {
    let end = command
        .iter()
        .position(|&b| !(b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_'))
        .unwrap_or(command.len());
    let name = std::str::from_utf8(&command[..end]).ok()?;
    (!name.is_empty()).then(|| name.to_owned())
}

/// Where `needle` first starts in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// The known browsers installed on this PC, in the order of `ALL`. None is blocked on Linux.
pub fn found(pc: &impl Lookup) -> Vec<Found> {
    ALL.into_iter()
        .filter_map(|browser| {
            let exe = browser.find(pc)?;
            Some(Found {
                browser,
                exe,
                blocked: None,
            })
        })
        .collect()
}

/// The user's default browser (`mimeapps.list` names one for https links) is none of
/// `found`: one the sign-in can't open.
pub fn default_elsewhere(pc: &impl Lookup, found: &[Found]) -> bool {
    default_https_entry(pc).is_some_and(|entry| {
        !found
            .iter()
            .filter_map(|f| place(f.browser))
            .any(|p| p.desktop.contains(&entry.as_str()))
    })
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

/// The lock a browser keeps on its profile while it runs: a link whose target ends in the pid
/// of the browser's main process. Chromium's `SingletonLock` reads `{host}-{pid}`
/// (`chrome/browser/process_singleton_posix.cc`) and goes on a clean exit; Firefox's `lock`
/// reads `{ip}:+{pid}` (`toolkit/profile/nsProfileLock.cpp`) and goes on SIGINT or SIGTERM,
/// but stays after a window closes it (its fast shutdown skips that) and after a kill, as
/// Chromium's does after a kill (all seen in WSL, 2026-11-21). So a lock only counts while a
/// process holds it (`holder`); one left behind is stale.
#[derive(Clone, Debug)]
pub struct ProfileLock {
    profile: PathBuf,
    link: PathBuf,
}

impl ProfileLock {
    pub fn new(profile: &Path, kind: Kind) -> Self {
        let name = match kind {
            Kind::Chromium => "SingletonLock",
            Kind::Firefox => "lock",
        };
        Self {
            profile: profile.to_path_buf(),
            link: profile.join(name),
        }
    }

    /// The running process that holds the lock, if any: the one with the lock's pid, or, for
    /// a browser in a process namespace of its own (a Flatpak), the one with that pid inside
    /// its namespace; either way only one that was started on this profile (it's in its
    /// command line), so a pid that's been reused since never counts.
    pub fn holder(&self) -> Option<u32> {
        let target = std::fs::read_link(&self.link).ok()?;
        let target = target.as_os_str().as_bytes();
        let digits = target
            .iter()
            .rev()
            .take_while(|b| b.is_ascii_digit())
            .count();
        let pid: u32 = std::str::from_utf8(&target[target.len() - digits..])
            .ok()?
            .parse()
            .ok()?;
        if pid > 1 && self.started_on_profile(pid) {
            return Some(pid);
        }
        self.in_namespace(pid)
    }

    /// The process `pid`'s command line names this profile.
    fn started_on_profile(&self, pid: u32) -> bool {
        let Ok(cmdline) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
            return false;
        };
        let profile = self.profile.as_os_str().as_bytes();
        find(&cmdline, profile).is_some()
    }

    /// The process started on this profile whose pid in its own namespace is `inner` (the
    /// last of the `NSpid` field in its `/proc/{pid}/status`, proc(5)).
    fn in_namespace(&self, inner: u32) -> Option<u32> {
        let inner = inner.to_string();
        std::fs::read_dir("/proc")
            .ok()?
            .flatten()
            .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
            .find(|&pid| {
                let Ok(status) = std::fs::read_to_string(format!("/proc/{pid}/status")) else {
                    return false;
                };
                let nspid = status.lines().find_map(|l| l.strip_prefix("NSpid:"));
                nspid.and_then(|ids| ids.split_whitespace().last()) == Some(inner.as_str())
                    && self.started_on_profile(pid)
            })
    }
}

/// The running browser, the leader of its own process group. Dropping it ends the group:
/// the browser and every helper it started.
pub struct Running {
    child: Mutex<Child>,
    pub pid: u32,
    /// The profile's lock, which tells the browser is still open after its launcher exited.
    lock: Option<ProfileLock>,
}

impl Running {
    /// The browser has exited: its process, and the process that holds its profile's lock
    /// (its helpers may linger a moment). With no lock to watch, its process alone.
    pub fn exited(&self) -> bool {
        let mut child = self.child.lock().unwrap_or_else(PoisonError::into_inner);
        if matches!(child.try_wait(), Ok(None)) {
            return false;
        }
        self.lock.as_ref().is_none_or(|l| l.holder().is_none())
    }

    /// Waits up to `limit` for the browser to exit; says whether it did.
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

    /// Asks the browser to quit: SIGINT to the process that holds its profile's lock (the
    /// browser itself, also inside a Flatpak, where a signal to `flatpak run` would leave it
    /// running), else to the process started. Chromium's shutdown handler takes it as a clean
    /// exit that writes its cookie store first. Not SIGTERM: Chrome 157 ended at once on it,
    /// without writing them (seen in WSL, 2026-11-13: cookies a page had set 12 s before were
    /// gone after SIGTERM and kept after SIGINT or SIGHUP; Chromium writes them every 30 s
    /// anyway). Firefox dies on it after taking its lock away, and has its cookies written
    /// within half a second of each change (`network.cookie.db.flushIntervalMs`). `wait` then
    /// tells when it has exited.
    pub fn quit(&self) {
        if let Some(holder) = self.holder() {
            signal(holder, libc::SIGINT);
        } else if !self.process_exited() {
            // Not reaped yet (`process_exited` says so), so its pid is still ours.
            signal(self.pid, libc::SIGINT);
        }
    }

    /// Ends the browser and every process in its group, and the process holding its
    /// profile's lock if that's another.
    pub fn kill(&self) {
        let holder = self.holder();
        let pgid = libc::pid_t::try_from(self.pid).unwrap_or(0);
        if pgid > 0 {
            // SAFETY: a signal to the process group our child leads (`process_group(0)`); no
            // memory is touched. A group that's gone already is ESRCH, which is fine.
            unsafe { libc::killpg(pgid, libc::SIGKILL) };
        }
        if let Some(holder) = holder {
            signal(holder, libc::SIGKILL);
        }
        let mut child = self.child.lock().unwrap_or_else(PoisonError::into_inner);
        // Reaps it, so no zombie stays; it has been sent SIGKILL, so this doesn't wait long.
        let _ = child.wait();
    }

    fn holder(&self) -> Option<u32> {
        self.lock.as_ref()?.holder()
    }

    fn process_exited(&self) -> bool {
        let mut child = self.child.lock().unwrap_or_else(PoisonError::into_inner);
        !matches!(child.try_wait(), Ok(None))
    }
}

/// Sends `sig` to `pid`: one of our children, or a process started on our temporary profile.
fn signal(pid: u32, sig: libc::c_int) {
    let pid = libc::pid_t::try_from(pid).unwrap_or(0);
    if pid > 1 {
        // SAFETY: a signal to one process; no memory is touched. One that's gone is ESRCH.
        unsafe { libc::kill(pid, sig) };
    }
}

/// Dropping a running browser ends it, as on Windows (whose job ends with its handle): callers
/// drop it on purpose.
impl Drop for Running {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Starts `exe` with `args`: a browser as the user would start it, for the sign-in window.
/// Nothing of the app's reaches it but its arguments (every descriptor of the app's is
/// close-on-exec; its standard ones are /dev/null). It leads a process group of its own,
/// which `Running` ends, and it's killed if the thread that started it ends first (the app
/// crashed or was killed): the sign-in thread starts it, watches it and ends it. With `lock`,
/// it counts as running while that's held, after its own process has exited too.
pub fn launch_plain(
    exe: &Path,
    args: &[OsString],
    lock: Option<ProfileLock>,
) -> io::Result<Running> {
    let mut command = Command::new(exe);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0);
    // SAFETY: `orphan_ends` only calls prctl, which is async-signal-safe, and touches no
    // memory of the parent's.
    unsafe {
        command.pre_exec(orphan_ends);
    }
    let child = command.spawn()?;
    Ok(Running {
        pid: child.id(),
        child: Mutex::new(child),
        lock,
    })
}

/// In the child, between fork and exec: it's killed when the thread that started it ends.
fn orphan_ends() -> io::Result<()> {
    // SAFETY: prctl with two integer arguments; it sets this process's own death signal.
    if unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) } == -1 {
        return Err(io::Error::last_os_error());
    }
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
