//! What every OS's `browsers` shares: the browsers the sign-in can open (Microsoft Edge,
//! Google Chrome, Brave and Vivaldi; on Linux Chromium too), why one can't be used
//! (`Block`), and which one the page offers (`pick`). All are Chromium, with its DevTools
//! pipe (`--remote-debugging-pipe`, see `launch`) and its `RemoteDebuggingAllowed` policy.
//! Opera stays out (its launcher starts another process), and Firefox has no CDP pipe.
//! Finding them is each OS's (`found`, through its `Lookup`, which its `ThisPc` reads: on
//! Windows the registry, the environment and the disk; on Linux `PATH`, the disk and the
//! policy files).
//!
//! Where each Windows fact comes from (checked 2026-11-09; Linux's are in `linux::browsers`):
//! - **Every one**: an installer registers its exe under `App Paths\{exe}`, in HKCU for a
//!   per-user install and in HKLM for the machine (Microsoft's "Application Registration",
//!   learn.microsoft.com/windows/win32/shell/app-registration). A Chromium installer puts it
//!   in `{company}\{product}\Application` under Program Files or, per user, `%LOCALAPPDATA%`
//!   (Chromium's `chrome/installer/mini_installer/README.md`,
//!   `chrome/install_static/install_util.cc`). `RemoteDebuggingAllowed` set to false turns
//!   off both `--remote-debugging-pipe` and the port (Chromium's policy definition,
//!   `components/policy/resources/templates/policy_definitions/Miscellaneous/
//!   RemoteDebuggingAllowed.yaml`, and the same text in Microsoft's reference for Edge).
//! - **Edge**: the policy under `SOFTWARE\Policies\Microsoft\Edge`, a REG_DWORD (Microsoft's
//!   policy reference, learn.microsoft.com/deployedge/microsoft-edge-policies/
//!   remotedebuggingallowed). Edge Stable installs for the machine only.
//! - **Chrome**: `Google\Chrome` (`chrome/install_static/google_chrome_install_modes.h`),
//!   per user under `%LOCALAPPDATA%` (the mini_installer README), policies under
//!   `SOFTWARE\Policies\Google\Chrome` (Chrome Enterprise help, support.google.com/chrome/a/
//!   answer/9131254). Since Chrome 136 the pipe and the port are ignored on Chrome's default
//!   data directory and work with a `--user-data-dir` elsewhere
//!   (developer.chrome.com/blog/remote-debugging-port): the sign-in's temporary profile is
//!   that supported case.
//! - **Brave**, from brave-core's sources: `BraveSoftware\Brave-Browser`
//!   (`chromium_src/chrome/install_static/chromium_install_modes.h`), `brave.exe`
//!   (`chromium_src/chrome/installer/util/util_constants.h`), policies under
//!   `SOFTWARE\Policies\BraveSoftware\Brave`
//!   (`chromium_src/components/policy/tools/generate_policy_source.py`). Brave's help has a
//!   Group Policy page, but it couldn't be read.
//! - **Vivaldi**: its help confirms an install for all users or for one user
//!   (help.vivaldi.com/article/install-the-vivaldi-browser). **Not in its documentation**:
//!   `vivaldi.exe`, the folder `Vivaldi\Application` and the policy key
//!   `SOFTWARE\Policies\Vivaldi` come from users and volunteers on Vivaldi's forum; Vivaldi
//!   publishes no policy templates yet. A wrong policy key only means a block is found late:
//!   the sign-in then ends after 15 s without an answer, as for any browser.
//!
//! Only Edge has run here against the real browser; the app's
//! `live_browsers_answer_on_the_pipe_and_close` runs for every one that's installed, e.g. on
//! a CI runner with Chrome, Brave and Vivaldi.

use std::path::{Path, PathBuf};

/// A Chromium browser the sign-in can drive over the DevTools pipe.
#[derive(Debug, PartialEq, Eq)]
pub struct Browser {
    /// What the settings file calls it, to remember the one used last.
    pub id: &'static str,
    /// As the user knows it, for the page and the log.
    pub name: &'static str,
    /// Its name in a sentence that's about it: "Edge closed".
    pub short: &'static str,
    /// Its registration under `Software\Microsoft\Windows\CurrentVersion\App Paths`.
    pub exe: &'static str,
    /// Where its installer puts the exe, under `%ProgramFiles(x86)%` or `%ProgramFiles%`,
    /// and under `%LOCALAPPDATA%` when it's installed for one user (`per_user`).
    pub install_dir: &'static str,
    /// Its installer can install for one user, without administrator rights.
    pub per_user: bool,
    /// Its switches besides the profile and the pipe: no welcome pages, no "make me the
    /// default" bar. Nothing that marks the browser as automated (on Linux, they take off
    /// the mark the pipe leaves on pages).
    pub switches: &'static [&'static str],
    /// Its policies' key, in HKLM and HKCU. Its `RemoteDebuggingAllowed` set to 0 turns
    /// the DevTools pipe off.
    pub policy_key: &'static str,
    /// Written to the temporary profile's `Default/Preferences` before the browser starts,
    /// for a browser whose own defaults get in the sign-in's way.
    pub preferences: Option<&'static str>,
}

/// Chromium's own switches (`chrome/common/chrome_switches.h`), the same in all of them.
#[cfg(windows)]
const QUIET_START: &[&str] = &["--no-first-run", "--no-default-browser-check"];

/// As on Windows, and:
/// - the throwaway profile keeps its cookies in Chromium's own basic store rather than the
///   desktop's keyring (`--password-store`, `chrome/common/chrome_switches.cc`), so a sign-in
///   never adds an item there or asks to unlock a wallet;
/// - pages see `navigator.webdriver` false, as in a browser the user started. Chromium hands
///   `--remote-debugging-pipe` on to every renderer, which then reports `navigator.webdriver`
///   true (seen in WSL, Chrome 157, 2026-11-12: the only difference a page could see between
///   the sign-in's launch and a plain one), and Google's sign-in then refused with "This
///   browser or app may not be secure". `--disable-blink-features=AutomationControlled` turns
///   that Blink feature off; `--test-type` keeps Chromium from adding a bar that warns about
///   that switch ("unsupported command-line flag", `chrome/browser/ui/startup/
///   bad_flags_prompt.cc`, which skips its check under `--test-type`).
#[cfg(target_os = "linux")]
const QUIET_START: &[&str] = &[
    "--no-first-run",
    "--no-default-browser-check",
    "--password-store=basic",
    "--disable-blink-features=AutomationControlled",
    "--test-type",
];

pub const EDGE: Browser = Browser {
    id: "edge",
    name: "Microsoft Edge",
    short: "Edge",
    exe: "msedge.exe",
    install_dir: r"Microsoft\Edge\Application",
    per_user: false,
    switches: QUIET_START,
    policy_key: r"Software\Policies\Microsoft\Edge",
    preferences: None,
};

pub const CHROME: Browser = Browser {
    id: "chrome",
    name: "Google Chrome",
    short: "Chrome",
    exe: "chrome.exe",
    install_dir: r"Google\Chrome\Application",
    per_user: true,
    switches: QUIET_START,
    policy_key: r"Software\Policies\Google\Chrome",
    preferences: None,
};

pub const BRAVE: Browser = Browser {
    id: "brave",
    name: "Brave",
    short: "Brave",
    exe: "brave.exe",
    install_dir: r"BraveSoftware\Brave-Browser\Application",
    per_user: true,
    switches: QUIET_START,
    policy_key: r"Software\Policies\BraveSoftware\Brave",
    preferences: None,
};

pub const VIVALDI: Browser = Browser {
    id: "vivaldi",
    name: "Vivaldi",
    short: "Vivaldi",
    exe: "vivaldi.exe",
    install_dir: r"Vivaldi\Application",
    per_user: true,
    switches: QUIET_START,
    policy_key: r"Software\Policies\Vivaldi",
    // Vivaldi asks "Confirm exit" when its window is closed, so closing it to cancel the
    // sign-in left it running behind the dialog (checked on a GitHub Windows runner,
    // Vivaldi 8.3, 2026-11-10). With the dialog off it exits within 2 s, as the others do.
    preferences: Some(r#"{"vivaldi":{"system":{"show_exit_confirmation_dialog":false}}}"#),
};

/// Chromium itself, which Linux distributions package. Windows has no installer for it, so
/// the sign-in doesn't look for it there; its Windows facts are a self-built Chromium's
/// (`chrome/install_static/chromium_install_modes.h`).
pub const CHROMIUM: Browser = Browser {
    id: "chromium",
    name: "Chromium",
    short: "Chromium",
    exe: "chrome.exe",
    install_dir: r"Chromium\Application",
    per_user: true,
    switches: QUIET_START,
    policy_key: r"Software\Policies\Chromium",
    preferences: None,
};

/// Every browser the sign-in knows, in the order it offers them.
#[cfg(windows)]
pub const ALL: [&Browser; 4] = [&EDGE, &CHROME, &BRAVE, &VIVALDI];

/// Every browser the sign-in knows, in the order it offers them: Chrome first, as Linux has
/// no browser of its own, then Chromium, which most distributions package.
#[cfg(target_os = "linux")]
pub const ALL: [&Browser; 5] = [&CHROME, &CHROMIUM, &EDGE, &BRAVE, &VIVALDI];

/// Why the sign-in can't use a browser that's installed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Block {
    /// A policy (`RemoteDebuggingAllowed` off) turns off its DevTools pipe.
    Policy,
    /// It's a Snap (Linux), whose sandbox keeps out a profile folder of the app's.
    Snap,
    /// It's a Flatpak (Linux), whose sandbox keeps out the profile folder and the pipe.
    Flatpak,
}

/// The browser the settings file calls `id`.
pub fn by_id(id: &str) -> Option<&'static Browser> {
    ALL.into_iter().find(|b| b.id == id)
}

/// A known browser that's installed on this PC.
#[derive(Debug, PartialEq, Eq)]
pub struct Found {
    pub browser: &'static Browser,
    pub exe: PathBuf,
    /// Why the sign-in can't use it, if it can't.
    pub blocked: Option<Block>,
}

/// Which of `found` the sign-in offers: the one used last (`remembered`, a `Browser::id`),
/// else the user's default browser (`default`, the exe https links open with), else the
/// first. One the sign-in can't use is offered only when it can use none.
pub fn pick(found: &[Found], remembered: Option<&str>, default: Option<&Path>) -> Option<usize> {
    let rank = |f: &Found| {
        if remembered == Some(f.browser.id) {
            0
        } else if default.is_some_and(|exe| same_path(exe, &f.exe)) {
            1
        } else {
            2
        }
    };
    (0..found.len()).min_by_key(|&i| (found[i].blocked.is_some(), rank(&found[i]), i))
}

/// Windows' paths don't tell upper case from lower.
#[cfg(windows)]
fn same_path(a: &Path, b: &Path) -> bool {
    a.as_os_str().eq_ignore_ascii_case(b.as_os_str())
}

/// Linux's paths do.
#[cfg(not(windows))]
fn same_path(a: &Path, b: &Path) -> bool {
    a == b
}
