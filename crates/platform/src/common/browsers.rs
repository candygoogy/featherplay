//! What every OS's `browsers` shares: the browsers the sign-in can open (Microsoft Edge,
//! Google Chrome, Brave and Vivaldi; on Linux Chromium and Firefox too; on Windows also Yandex
//! Browser and the other channels of Edge, Chrome and Brave), why one can't be used
//! (`Block`), and which one the page offers (`pick`). On Windows all are Chromium, read
//! over the DevTools pipe (`--remote-debugging-pipe`, see `launch`) once the sign-in window has
//! closed, which a `RemoteDebuggingAllowed` policy can turn off. On Linux the app reads the
//! closed browser's cookie database itself (`linux::browsers::cookies`), so no policy is in
//! the way and Firefox can join (`Kind`). Opera stays out (its launcher starts another
//! process). Finding them is each OS's (`found`, through its `Lookup`, which its `ThisPc`
//! reads: on Windows the registry, the environment and the disk; on Linux `PATH` and the
//! disk).
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
//! - **The other channels** (checked 2026-11-21): each channel's install mode, compiled into
//!   every channel's binary, gives its folder suffix (Chromium's
//!   `chrome/install_static/google_chrome_install_modes.h`: " Beta", " Dev", " SxS" for
//!   Canary, which installs for one user only; Edge's own table, read from msedge_elf.dll
//!   158: the same suffixes; brave-core's `chromium_install_modes.h`: "-Beta", "-Nightly").
//!   One policy key serves the brand (`generate_policy_source.py`'s `CHROME_POLICY_KEY`,
//!   brave-core's `CHROMIUM_POLICY_KEY`; msedge_elf.dll holds only Edge's). Every
//!   channel registers `App Paths\{exe}` for its own exe (Chromium's `shell_util.cc`,
//!   `GetChromeAppRegistrationEntries`), so the channels of a brand overwrite one another's,
//!   and a registration is taken only by the browser whose folder it's in (`find`). A Beta
//!   or Dev Edge installed for one user was reported on Microsoft's forum.
//! - **Yandex Browser**: `browser.exe` in its installer's archive (Yandex.exe 26.8.5.811, as
//!   winget lists it, with `Scope: user`), the folder `Yandex\YandexBrowser` and
//!   `\Application` from its browser_elf.dll, and its policy key, with
//!   `RemoteDebuggingAllowed`, from Yandex's ADMX templates (as ALT Linux packages them,
//!   `admx-yandex-browser` 138.0).
//! - **Brave Origin** stays out on Windows: before any window it shows a dialog that checks the
//!   purchase, kept in the browser's own `Local State`, which a new profile never has
//!   (brave-core's `BraveOriginStartupView::ShouldShowDialog`), so every sign-in would stop
//!   there.
//!
//! Only Edge has run here against the real browser (Yandex Browser hasn't run on Windows at
//! all); the app's
//! `live_browsers_answer_on_the_pipe_and_close` runs for every one that's installed, e.g. on
//! a CI runner with Chrome, Brave and Vivaldi, and `live::live_browser_signs_in_plainly` for
//! the first one found.

use std::path::{Path, PathBuf};

/// A browser the sign-in can open: on Windows a Chromium one, read over the DevTools pipe; on
/// Linux a Chromium one or Firefox, read from its profile on disk.
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
    /// default" bar. Nothing that marks the browser as automated or hides that mark.
    pub switches: &'static [&'static str],
    /// Its policies' key, in HKLM and HKCU. Its `RemoteDebuggingAllowed` set to 0 turns
    /// the DevTools pipe off.
    pub policy_key: &'static str,
    /// Written to the temporary profile before the browser starts, for a browser whose own
    /// defaults get in the sign-in's way: a Chromium one's `Default/Preferences`, Firefox's
    /// `user.js`.
    pub preferences: Option<&'static str>,
    /// Chromium or Firefox: how it's started, and how its profile keeps the cookies.
    #[cfg(target_os = "linux")]
    pub kind: Kind,
}

/// The two families of browsers the Linux sign-in can read.
#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Chromium,
    Firefox,
}

/// Chromium's headless mode (`--headless=new`, the same browser with no window,
/// `headless/README.md`): the start that reads the profile, on a blank page.
#[cfg(windows)]
pub const HEADLESS: &str = "--headless=new";

/// Chromium's own switches (`chrome/common/chrome_switches.h`), the same in all of them.
///
/// The sign-in window gets no pipe at all: Chromium hands `--remote-debugging-pipe` on to
/// every renderer, whose pages then see `navigator.webdriver` true (seen in WSL with Chrome
/// 157, 2026-11-12, and on Windows with Edge), and Google's sign-in may refuse such a browser,
/// rightly ("This browser or app may not be secure").
#[cfg(windows)]
const QUIET_START: &[&str] = &["--no-first-run", "--no-default-browser-check"];

/// As on Windows, and the throwaway profile keeps its cookies in Chromium's own basic store
/// rather than the desktop's keyring (`--password-store`, `chrome/common/chrome_switches.cc`),
/// so a sign-in never adds an item there or asks to unlock a wallet, and the app can decrypt
/// them afterwards with that store's fixed key (`linux::browsers::cookies`).
#[cfg(target_os = "linux")]
const QUIET_START: &[&str] = &[
    "--no-first-run",
    "--no-default-browser-check",
    "--password-store=basic",
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
    #[cfg(target_os = "linux")]
    kind: Kind::Chromium,
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
    #[cfg(target_os = "linux")]
    kind: Kind::Chromium,
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
    #[cfg(target_os = "linux")]
    kind: Kind::Chromium,
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
    #[cfg(target_os = "linux")]
    kind: Kind::Chromium,
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
    #[cfg(target_os = "linux")]
    kind: Kind::Chromium,
};

/// Firefox, on Linux only, where most distributions ship it. Started as a new instance of its
/// own (`--new-instance`, so it never hands the page to a Firefox the user has open) on the
/// temporary profile (`--profile <dir>`): both in Firefox's `--help` and
/// `toolkit/xre/nsAppRunner.cpp`. Its `user.js` skips what a new profile shows first
/// (`BrowserContentHandler.sys.mjs`, `browser/app/profile/firefox.js`,
/// `modules/libpref/init/all.js`): the welcome and what's-new pages, the privacy notice tab,
/// the terms of use, the default-browser question. Nothing about automation. Its import
/// wizard opens only with `--migration` or a profile reset. The Windows fields are unused.
#[cfg(target_os = "linux")]
pub const FIREFOX: Browser = Browser {
    id: "firefox",
    name: "Firefox",
    short: "Firefox",
    exe: "firefox",
    install_dir: "",
    per_user: false,
    switches: &["--new-instance"],
    policy_key: "",
    preferences: Some(concat!(
        "user_pref(\"browser.startup.homepage_override.mstone\", \"ignore\");\n",
        "user_pref(\"startup.homepage_welcome_url\", \"\");\n",
        "user_pref(\"browser.aboutwelcome.enabled\", false);\n",
        "user_pref(\"datareporting.policy.firstRunURL\", \"\");\n",
        "user_pref(\"datareporting.policy.dataSubmissionPolicyBypassNotification\", true);\n",
        "user_pref(\"termsofuse.bypassNotification\", true);\n",
        "user_pref(\"browser.shell.checkDefaultBrowser\", false);\n",
    )),
    kind: Kind::Firefox,
};

/// The other channels and editions on Windows. A channel installs beside the stable one in
/// a folder of its own (`{company}\{product}{suffix}\Application`) with the same exe and
/// reads the brand's policies; Edge and Chrome Canary install for one user only.
pub const EDGE_BETA: Browser = Browser {
    id: "edge-beta",
    name: "Microsoft Edge Beta",
    short: "Edge Beta",
    install_dir: r"Microsoft\Edge Beta\Application",
    per_user: true,
    ..EDGE
};

pub const EDGE_DEV: Browser = Browser {
    id: "edge-dev",
    name: "Microsoft Edge Dev",
    short: "Edge Dev",
    install_dir: r"Microsoft\Edge Dev\Application",
    per_user: true,
    ..EDGE
};

pub const EDGE_CANARY: Browser = Browser {
    id: "edge-canary",
    name: "Microsoft Edge Canary",
    short: "Edge Canary",
    install_dir: r"Microsoft\Edge SxS\Application",
    per_user: true,
    ..EDGE
};

pub const CHROME_BETA: Browser = Browser {
    id: "chrome-beta",
    name: "Google Chrome Beta",
    short: "Chrome Beta",
    install_dir: r"Google\Chrome Beta\Application",
    ..CHROME
};

pub const CHROME_DEV: Browser = Browser {
    id: "chrome-dev",
    name: "Google Chrome Dev",
    short: "Chrome Dev",
    install_dir: r"Google\Chrome Dev\Application",
    ..CHROME
};

pub const CHROME_CANARY: Browser = Browser {
    id: "chrome-canary",
    name: "Google Chrome Canary",
    short: "Chrome Canary",
    install_dir: r"Google\Chrome SxS\Application",
    ..CHROME
};

pub const BRAVE_BETA: Browser = Browser {
    id: "brave-beta",
    name: "Brave Beta",
    short: "Brave Beta",
    install_dir: r"BraveSoftware\Brave-Browser-Beta\Application",
    ..BRAVE
};

pub const BRAVE_NIGHTLY: Browser = Browser {
    id: "brave-nightly",
    name: "Brave Nightly",
    short: "Brave Nightly",
    install_dir: r"BraveSoftware\Brave-Browser-Nightly\Application",
    ..BRAVE
};

/// Yandex Browser's start: as the others', without its background mode, in which closing its
/// window starts it again with no window on the same profile, which then never lets go
/// (seen in WSL with 26.10: a `--no-startup-window` process; with the switch it exited).
/// `--disable-background-mode` is an older Chromium switch that both Yandex.exe 26.8's
/// browser.dll and the Linux package's binary still have.
#[cfg(windows)]
const YANDEX_START: &[&str] = &[
    "--no-first-run",
    "--no-default-browser-check",
    "--disable-background-mode",
];

#[cfg(target_os = "linux")]
const YANDEX_START: &[&str] = &[
    "--no-first-run",
    "--no-default-browser-check",
    "--password-store=basic",
    "--disable-background-mode",
];

pub const YANDEX: Browser = Browser {
    id: "yandex",
    name: "Yandex Browser",
    short: "Yandex Browser",
    exe: "browser.exe",
    install_dir: r"Yandex\YandexBrowser\Application",
    switches: YANDEX_START,
    policy_key: r"Software\Policies\YandexBrowser",
    ..CHROME
};

/// Every browser the sign-in knows, in the order it offers them: the stable ones first, so a
/// PC with Chrome and Chrome Beta offers Chrome unless Beta is the default or was used last.
#[cfg(windows)]
pub const ALL: [&Browser; 13] = [
    &EDGE,
    &CHROME,
    &BRAVE,
    &VIVALDI,
    &YANDEX,
    &EDGE_BETA,
    &EDGE_DEV,
    &EDGE_CANARY,
    &CHROME_BETA,
    &CHROME_DEV,
    &CHROME_CANARY,
    &BRAVE_BETA,
    &BRAVE_NIGHTLY,
];

/// Every browser the sign-in knows, in the order it offers them (after the default browser):
/// Chrome first, as Linux has no browser of its own, then Chromium and Firefox, which most
/// distributions package.
#[cfg(target_os = "linux")]
pub const ALL: [&Browser; 6] = [&CHROME, &CHROMIUM, &FIREFOX, &EDGE, &BRAVE, &VIVALDI];

/// Why the sign-in can't use a browser that's installed. Only Windows has one: Linux reads
/// the profile from disk, which no policy turns off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Block {
    /// A policy (`RemoteDebuggingAllowed` off) turns off its DevTools pipe.
    Policy,
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
pub(crate) fn same_path(a: &Path, b: &Path) -> bool {
    a.as_os_str().eq_ignore_ascii_case(b.as_os_str())
}

/// Linux's paths do.
#[cfg(not(windows))]
pub(crate) fn same_path(a: &Path, b: &Path) -> bool {
    a == b
}

#[cfg(test)]
mod live;

#[cfg(test)]
mod tests {
    use super::*;

    /// The sign-in window is a browser as the user would start it: its switches neither mark
    /// it as automated nor hide such a mark, and on Linux keep its cookies out of the keyring.
    #[test]
    fn every_browser_starts_plainly() {
        for b in ALL {
            for s in b.switches {
                assert!(
                    !s.contains("automation")
                        && !s.contains("headless")
                        && !s.contains("remote-debugging")
                        && (!s.starts_with("--disable-") || *s == "--disable-background-mode")
                        && !s.contains("test"),
                    "{}: {s}",
                    b.name
                );
            }
            #[cfg(target_os = "linux")]
            assert_eq!(
                b.switches.contains(&"--password-store=basic"),
                b.kind == Kind::Chromium,
                "{}",
                b.name
            );
        }
    }

    /// Firefox's `user.js` is only `user_pref(name, value);` lines, each a pref that skips a
    /// first-run page or question.
    #[cfg(target_os = "linux")]
    #[test]
    fn firefox_s_user_js_only_skips_first_run_pages() {
        let prefs = FIREFOX.preferences.unwrap();
        for line in prefs.lines() {
            assert!(
                line.starts_with("user_pref(\"") && line.ends_with(");"),
                "{line}"
            );
            assert!(
                !line.contains("automation") && !line.contains("webdriver"),
                "{line}"
            );
        }
        assert!(prefs.contains("\"browser.aboutwelcome.enabled\", false"));
    }

    #[test]
    fn the_ids_are_distinct_and_lead_back_to_their_browser() {
        for browser in ALL {
            assert_eq!(by_id(browser.id), Some(browser));
        }
        assert_eq!(by_id("opera"), None);
        // Each is told apart on the page and, on Windows, by its folder.
        let distinct = |field: fn(&Browser) -> &str| {
            let mut all: Vec<&str> = ALL.iter().map(|b| field(b)).collect();
            all.sort_unstable();
            all.dedup();
            all.len() == ALL.len()
        };
        assert!(distinct(|b| b.name) && distinct(|b| b.short));
        #[cfg(windows)]
        assert!(distinct(|b| b.install_dir));
    }

    /// Where `browser` is installed in these cases: a made-up path of its own.
    fn exe(browser: &Browser) -> String {
        format!("/apps/{}/{}", browser.id, browser.exe)
    }

    fn at(browser: &'static Browser, blocked: Option<Block>) -> Found {
        Found {
            browser,
            exe: exe(browser).into(),
            blocked,
        }
    }

    struct Case {
        label: &'static str,
        found: Vec<Found>,
        remembered: Option<&'static str>,
        default: Option<String>,
        offered: Option<&'static str>,
    }

    #[test]
    fn pick_offers_the_one_used_last_then_the_default_then_the_first_that_works() {
        let usual = || vec![at(&EDGE, None), at(&CHROME, None), at(&BRAVE, None)];
        let two = || vec![at(&EDGE, None), at(&CHROME, None)];
        let policy = Some(Block::Policy);
        let opera = "/apps/opera/opera.exe".to_string();
        #[cfg_attr(not(windows), allow(unused_mut, reason = "Windows adds a case"))]
        let mut cases = vec![
            Case {
                label: "no default: the first found",
                found: usual(),
                remembered: None,
                default: None,
                offered: Some("edge"),
            },
            Case {
                label: "a default the sign-in doesn't know: the first found",
                found: usual(),
                remembered: None,
                default: Some(opera),
                offered: Some("edge"),
            },
            Case {
                label: "no default, other browsers: the first found",
                found: vec![at(&BRAVE, None), at(&VIVALDI, None)],
                remembered: None,
                default: None,
                offered: Some("brave"),
            },
            Case {
                label: "a known default",
                found: usual(),
                remembered: None,
                default: Some(exe(&BRAVE)),
                offered: Some("brave"),
            },
            Case {
                label: "a channel that's the default comes before its stable one",
                found: vec![at(&CHROME, None), at(&CHROME_BETA, None)],
                remembered: None,
                default: Some(exe(&CHROME_BETA)),
                offered: Some("chrome-beta"),
            },
            Case {
                label: "the one used last comes before the default",
                found: usual(),
                remembered: Some("brave"),
                default: Some(exe(&CHROME)),
                offered: Some("brave"),
            },
            Case {
                label: "one used last that's gone",
                found: two(),
                remembered: Some("vivaldi"),
                default: Some(exe(&CHROME)),
                offered: Some("chrome"),
            },
            Case {
                label: "one used last that's unknown",
                found: two(),
                remembered: Some("netscape"),
                default: Some(exe(&CHROME)),
                offered: Some("chrome"),
            },
            Case {
                label: "the one used last and the default are blocked",
                found: vec![at(&EDGE, policy), at(&CHROME, policy), at(&BRAVE, None)],
                remembered: Some("edge"),
                default: Some(exe(&CHROME)),
                offered: Some("brave"),
            },
            Case {
                label: "the first is blocked: the default among those that work",
                found: vec![at(&EDGE, policy), at(&CHROME, None), at(&BRAVE, None)],
                remembered: None,
                default: Some(exe(&BRAVE)),
                offered: Some("brave"),
            },
            Case {
                label: "the first is blocked: the one used last among those that work",
                found: vec![at(&EDGE, policy), at(&CHROME, None), at(&BRAVE, None)],
                remembered: Some("chrome"),
                default: Some(exe(&BRAVE)),
                offered: Some("chrome"),
            },
            Case {
                label: "every one blocked, no default: the first",
                found: vec![at(&EDGE, policy), at(&CHROME, policy)],
                remembered: None,
                default: None,
                offered: Some("edge"),
            },
            Case {
                label: "every one blocked: the default",
                found: vec![at(&EDGE, policy), at(&CHROME, policy)],
                remembered: None,
                default: Some(exe(&CHROME)),
                offered: Some("chrome"),
            },
            Case {
                label: "nothing found",
                found: Vec::new(),
                remembered: Some("edge"),
                default: None,
                offered: None,
            },
        ];
        // Windows' paths compare without case.
        #[cfg(windows)]
        cases.push(Case {
            label: "a default in other case (Windows)",
            found: usual(),
            remembered: None,
            default: Some(exe(&CHROME).to_uppercase()),
            offered: Some("chrome"),
        });
        for c in cases {
            let default = c.default.as_deref().map(Path::new);
            let offered = pick(&c.found, c.remembered, default).map(|i| c.found[i].browser.id);
            assert_eq!(offered, c.offered, "{}", c.label);
        }
    }
}
