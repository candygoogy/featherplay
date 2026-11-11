//! What every OS's `browsers` shares: the browsers the sign-in can open, and how each is
//! found on this PC: Microsoft Edge, Google Chrome, Brave and Vivaldi. All four are
//! Chromium, with its DevTools pipe (`--remote-debugging-pipe`, see `launch`) and its
//! `RemoteDebuggingAllowed` policy. Opera stays out (its launcher starts another process),
//! and Firefox has no CDP pipe. What finding reads goes through `Lookup`; each OS's
//! `ThisPc` reads the real PC (Windows: the registry, the environment, the disk).
//!
//! Where each fact comes from (checked 2026-11-09):
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

use std::ffi::OsString;
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
    /// default" bar. Nothing that marks the browser as automated.
    pub switches: &'static [&'static str],
    /// Its policies' key, in HKLM and HKCU. Its `RemoteDebuggingAllowed` set to 0 turns
    /// the DevTools pipe off.
    pub policy_key: &'static str,
    /// Written to the temporary profile's `Default/Preferences` before the browser starts,
    /// for a browser whose own defaults get in the sign-in's way.
    pub preferences: Option<&'static str>,
}

/// Chromium's own switches (`chrome/common/chrome_switches.h`), the same in all four.
const QUIET_START: &[&str] = &["--no-first-run", "--no-default-browser-check"];

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

/// Every browser the sign-in knows, in the order it offers them.
pub const ALL: [&Browser; 4] = [&EDGE, &CHROME, &BRAVE, &VIVALDI];

/// The policy that turns off `--remote-debugging-pipe` (and the port) when it's 0.
const DEBUGGING_POLICY: &str = "RemoteDebuggingAllowed";

/// A registry hive App Paths is read from, the user's first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Hive {
    CurrentUser,
    LocalMachine,
}

/// What finding a browser reads, so tests can stand in for the PC.
pub trait Lookup {
    /// The default value of `App Paths\{exe}` in `hive`.
    fn app_path(&self, hive: Hive, exe: &str) -> Option<String>;
    fn env(&self, var: &str) -> Option<OsString>;
    fn is_file(&self, path: &Path) -> bool;
    /// A DWORD value under `key` in `hive`; `None` when it isn't set. Only ever read.
    fn dword(&self, hive: Hive, key: &str, value: &str) -> Option<u32>;
    /// The exe Windows opens https links with: the user's default browser.
    fn https_handler(&self) -> Option<PathBuf>;
}

impl Browser {
    /// The browser's exe: App Paths (the user's hive, then the machine's), then the
    /// install folder under `%ProgramFiles(x86)%`, under `%ProgramFiles%` and, for one that
    /// installs per user, under `%LOCALAPPDATA%`. A registration whose file is gone is
    /// skipped.
    pub fn find(&self, pc: &impl Lookup) -> Option<PathBuf> {
        let registered = [Hive::CurrentUser, Hive::LocalMachine]
            .into_iter()
            .filter_map(|hive| pc.app_path(hive, self.exe))
            .map(|v| PathBuf::from(v.trim().trim_matches('"')))
            .filter(|p| p.is_absolute());
        let installed = ["ProgramFiles(x86)", "ProgramFiles"]
            .into_iter()
            .chain(self.per_user.then_some("LOCALAPPDATA"))
            .filter_map(|var| pc.env(var))
            .map(|base| PathBuf::from(base).join(self.install_dir).join(self.exe));
        registered.chain(installed).find(|p| pc.is_file(p))
    }

    /// A policy turns off the DevTools pipe the sign-in needs: the browser would open and
    /// never answer. The machine's policy wins over the user's, as in Chromium.
    pub fn pipe_blocked(&self, pc: &impl Lookup) -> bool {
        let value = |hive| pc.dword(hive, self.policy_key, DEBUGGING_POLICY);
        value(Hive::LocalMachine).or_else(|| value(Hive::CurrentUser)) == Some(0)
    }
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
    /// A policy turns off its DevTools pipe: the sign-in can't use it.
    pub blocked: bool,
}

/// The known browsers installed on this PC, in the order of `ALL`.
pub fn found(pc: &impl Lookup) -> Vec<Found> {
    ALL.into_iter()
        .filter_map(|browser| {
            let exe = browser.find(pc)?;
            let blocked = browser.pipe_blocked(pc);
            Some(Found {
                browser,
                exe,
                blocked,
            })
        })
        .collect()
}

/// Which of `found` the sign-in offers: the one used last (`remembered`, a `Browser::id`),
/// else the user's default browser (`default`, the exe https links open with), else the
/// first. One whose pipe a policy blocks is offered only when every one is blocked.
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
    (0..found.len()).min_by_key(|&i| (found[i].blocked, rank(&found[i]), i))
}

/// Windows' paths don't tell upper case from lower.
fn same_path(a: &Path, b: &Path) -> bool {
    a.as_os_str().eq_ignore_ascii_case(b.as_os_str())
}

// The PCs these tests stand in are Windows ones, with Windows' paths (`C:\…` is absolute
// only there), as is the search they check.
#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const X86: &str = r"C:\Program Files (x86)";
    const X64: &str = r"C:\Program Files";
    const LOCAL: &str = r"C:\Users\ada\AppData\Local";

    const EDGE_EXE: &str = r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe";
    const CHROME_EXE: &str = r"C:\Program Files\Google\Chrome\Application\chrome.exe";
    const BRAVE_EXE: &str = r"C:\Program Files\BraveSoftware\Brave-Browser\Application\brave.exe";
    const VIVALDI_EXE: &str = r"C:\Users\ada\AppData\Local\Vivaldi\Application\vivaldi.exe";

    #[derive(Default)]
    struct FakePc {
        app_paths: HashMap<(Hive, &'static str), &'static str>,
        env: HashMap<&'static str, &'static str>,
        files: Vec<PathBuf>,
        dwords: HashMap<(Hive, &'static str, &'static str), u32>,
        https: Option<&'static str>,
    }

    impl Lookup for FakePc {
        fn app_path(&self, hive: Hive, exe: &str) -> Option<String> {
            self.app_paths
                .iter()
                .find(|((h, e), _)| *h == hive && *e == exe)
                .map(|(_, v)| v.to_string())
        }
        fn env(&self, var: &str) -> Option<OsString> {
            self.env.get(var).map(OsString::from)
        }
        fn is_file(&self, path: &Path) -> bool {
            self.files.iter().any(|f| f == path)
        }
        fn dword(&self, hive: Hive, key: &str, value: &str) -> Option<u32> {
            self.dwords
                .iter()
                .find(|((h, k, v), _)| *h == hive && *k == key && *v == value)
                .map(|(_, d)| *d)
        }
        fn https_handler(&self) -> Option<PathBuf> {
            self.https.map(PathBuf::from)
        }
    }

    fn policy(pc: &mut FakePc, browser: &Browser, hive: Hive, value: u32) {
        pc.dwords
            .insert((hive, browser.policy_key, "RemoteDebuggingAllowed"), value);
    }

    #[test]
    fn the_machines_policy_at_0_blocks_the_pipe() {
        let mut pc = pc();
        policy(&mut pc, &EDGE, Hive::LocalMachine, 0);
        assert!(EDGE.pipe_blocked(&pc));
    }

    #[test]
    fn the_users_policy_at_0_blocks_the_pipe() {
        let mut pc = pc();
        policy(&mut pc, &EDGE, Hive::CurrentUser, 0);
        assert!(EDGE.pipe_blocked(&pc));
    }

    #[test]
    fn the_machines_policy_wins_over_the_users() {
        let mut pc = pc();
        policy(&mut pc, &EDGE, Hive::LocalMachine, 1);
        policy(&mut pc, &EDGE, Hive::CurrentUser, 0);
        assert!(!EDGE.pipe_blocked(&pc));
    }

    #[test]
    fn no_policy_leaves_the_pipe_alone() {
        assert!(!EDGE.pipe_blocked(&pc()));
    }

    #[test]
    fn the_policy_at_1_leaves_the_pipe_alone() {
        let mut pc = pc();
        policy(&mut pc, &EDGE, Hive::LocalMachine, 1);
        policy(&mut pc, &EDGE, Hive::CurrentUser, 1);
        assert!(!EDGE.pipe_blocked(&pc));
    }

    #[test]
    fn each_browser_is_blocked_by_its_own_policy_key_only() {
        let keys = [
            r"Software\Policies\Microsoft\Edge",
            r"Software\Policies\Google\Chrome",
            r"Software\Policies\BraveSoftware\Brave",
            r"Software\Policies\Vivaldi",
        ];
        for (i, key) in keys.into_iter().enumerate() {
            let mut pc = pc();
            pc.dwords
                .insert((Hive::LocalMachine, key, "RemoteDebuggingAllowed"), 0);
            let blocked: Vec<bool> = ALL.iter().map(|b| b.pipe_blocked(&pc)).collect();
            let only: Vec<bool> = (0..ALL.len()).map(|j| j == i).collect();
            assert_eq!(blocked, only, "{key}");
        }
    }

    fn pc() -> FakePc {
        FakePc {
            env: HashMap::from([
                ("ProgramFiles(x86)", X86),
                ("ProgramFiles", X64),
                ("LOCALAPPDATA", LOCAL),
            ]),
            ..FakePc::default()
        }
    }

    #[test]
    fn the_users_app_path_comes_first() {
        let mut pc = pc();
        pc.app_paths
            .insert((Hive::CurrentUser, "msedge.exe"), r"D:\Edge\msedge.exe");
        pc.app_paths
            .insert((Hive::LocalMachine, "msedge.exe"), r"E:\Edge\msedge.exe");
        pc.files = vec![r"D:\Edge\msedge.exe".into(), r"E:\Edge\msedge.exe".into()];
        assert_eq!(EDGE.find(&pc), Some(r"D:\Edge\msedge.exe".into()));
    }

    #[test]
    fn the_machines_app_path_is_next_and_may_be_quoted() {
        let mut pc = pc();
        pc.app_paths.insert(
            (Hive::LocalMachine, "msedge.exe"),
            r#""C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe""#,
        );
        pc.files = vec![EDGE_EXE.into()];
        assert_eq!(EDGE.find(&pc), Some(EDGE_EXE.into()));
    }

    #[test]
    fn a_registration_comes_before_program_files() {
        let mut pc = pc();
        pc.app_paths
            .insert((Hive::LocalMachine, "msedge.exe"), r"D:\Edge\msedge.exe");
        pc.files = vec![EDGE_EXE.into(), r"D:\Edge\msedge.exe".into()];
        assert_eq!(EDGE.find(&pc), Some(r"D:\Edge\msedge.exe".into()));
    }

    #[test]
    fn a_registration_whose_file_is_gone_falls_back_to_program_files() {
        let mut pc = pc();
        pc.app_paths
            .insert((Hive::CurrentUser, "msedge.exe"), r"D:\Gone\msedge.exe");
        pc.files = vec![r"C:\Program Files\Microsoft\Edge\Application\msedge.exe".into()];
        assert_eq!(
            EDGE.find(&pc),
            Some(r"C:\Program Files\Microsoft\Edge\Application\msedge.exe".into())
        );
    }

    #[test]
    fn program_files_x86_comes_before_program_files() {
        let mut pc = pc();
        pc.files = vec![
            r"C:\Program Files\Microsoft\Edge\Application\msedge.exe".into(),
            EDGE_EXE.into(),
        ];
        assert_eq!(EDGE.find(&pc), Some(EDGE_EXE.into()));
    }

    #[test]
    fn nothing_found_is_none() {
        assert_eq!(EDGE.find(&pc()), None);
        // A relative registration isn't trusted.
        let mut pc = pc();
        pc.app_paths
            .insert((Hive::CurrentUser, "msedge.exe"), "msedge.exe");
        pc.files = vec!["msedge.exe".into()];
        assert_eq!(EDGE.find(&pc), None);
    }

    #[test]
    fn each_browser_is_found_by_its_own_registration() {
        for browser in ALL {
            let mut pc = pc();
            let exe = match browser.id {
                "edge" => r"D:\Apps\msedge.exe",
                "chrome" => r"D:\Apps\chrome.exe",
                "brave" => r"D:\Apps\brave.exe",
                _ => r"D:\Apps\vivaldi.exe",
            };
            pc.app_paths.insert((Hive::LocalMachine, browser.exe), exe);
            pc.files = vec![exe.into()];
            let found: Vec<&str> = ALL
                .iter()
                .filter(|b| b.find(&pc).is_some())
                .map(|b| b.id)
                .collect();
            assert_eq!(found, [browser.id]);
        }
    }

    #[test]
    fn a_machine_install_is_found_under_program_files() {
        let mut pc = pc();
        pc.files = vec![
            CHROME_EXE.into(),
            BRAVE_EXE.into(),
            r"C:\Program Files\Vivaldi\Application\vivaldi.exe".into(),
        ];
        assert_eq!(CHROME.find(&pc), Some(CHROME_EXE.into()));
        assert_eq!(BRAVE.find(&pc), Some(BRAVE_EXE.into()));
        assert_eq!(
            VIVALDI.find(&pc),
            Some(r"C:\Program Files\Vivaldi\Application\vivaldi.exe".into())
        );
    }

    #[test]
    fn a_per_user_install_is_found_under_local_app_data() {
        let mut pc = pc();
        let chrome = r"C:\Users\ada\AppData\Local\Google\Chrome\Application\chrome.exe";
        let brave = r"C:\Users\ada\AppData\Local\BraveSoftware\Brave-Browser\Application\brave.exe";
        pc.files = vec![chrome.into(), brave.into(), VIVALDI_EXE.into()];
        assert_eq!(CHROME.find(&pc), Some(chrome.into()));
        assert_eq!(BRAVE.find(&pc), Some(brave.into()));
        assert_eq!(VIVALDI.find(&pc), Some(VIVALDI_EXE.into()));
    }

    #[test]
    fn the_machines_install_comes_before_the_users() {
        let mut pc = pc();
        let users = r"C:\Users\ada\AppData\Local\Google\Chrome\Application\chrome.exe";
        pc.files = vec![users.into(), CHROME_EXE.into()];
        assert_eq!(CHROME.find(&pc), Some(CHROME_EXE.into()));
    }

    #[test]
    fn edge_isnt_looked_for_under_local_app_data() {
        let mut pc = pc();
        pc.files = vec![r"C:\Users\ada\AppData\Local\Microsoft\Edge\Application\msedge.exe".into()];
        assert_eq!(EDGE.find(&pc), None);
    }

    #[test]
    fn no_browsers_switches_mark_it_as_automated() {
        for s in ALL.iter().flat_map(|b| b.switches) {
            assert!(
                !s.contains("automation")
                    && !s.contains("headless")
                    && !s.contains("remote-debugging")
                    && !s.starts_with("--disable-")
                    && !s.contains("test"),
                "{s}"
            );
        }
    }

    #[test]
    fn the_ids_are_distinct_and_lead_back_to_their_browser() {
        for browser in ALL {
            assert_eq!(by_id(browser.id), Some(browser));
        }
        assert_eq!(by_id("opera"), None);
    }

    /// A PC with these browsers installed where their installers put them.
    fn pc_with(browsers: &[&Browser]) -> FakePc {
        let mut pc = pc();
        pc.files = browsers
            .iter()
            .map(|b| match b.id {
                "edge" => EDGE_EXE.into(),
                "chrome" => CHROME_EXE.into(),
                "brave" => BRAVE_EXE.into(),
                _ => VIVALDI_EXE.into(),
            })
            .collect();
        pc
    }

    fn ids(found: &[Found]) -> Vec<&'static str> {
        found.iter().map(|f| f.browser.id).collect()
    }

    /// The id of the browser `pick` offers on `pc`.
    fn offered(pc: &FakePc, remembered: Option<&str>) -> Option<&'static str> {
        let found = found(pc);
        pick(&found, remembered, pc.https_handler().as_deref()).map(|i| found[i].browser.id)
    }

    #[test]
    fn the_found_browsers_come_in_the_order_edge_chrome_brave_vivaldi() {
        let pc = pc_with(&[&VIVALDI, &BRAVE, &EDGE, &CHROME]);
        assert_eq!(ids(&found(&pc)), ["edge", "chrome", "brave", "vivaldi"]);
        let pc = pc_with(&[&VIVALDI, &CHROME]);
        assert_eq!(ids(&found(&pc)), ["chrome", "vivaldi"]);
        assert!(found(&self::pc()).is_empty());
    }

    #[test]
    fn found_says_which_ones_a_policy_blocks() {
        let mut pc = pc_with(&[&EDGE, &CHROME]);
        policy(&mut pc, &CHROME, Hive::CurrentUser, 0);
        let blocked: Vec<bool> = found(&pc).iter().map(|f| f.blocked).collect();
        assert_eq!(blocked, [false, true]);
    }

    #[test]
    fn without_a_known_default_the_first_found_is_offered() {
        let mut pc = pc_with(&[&EDGE, &CHROME, &BRAVE]);
        assert_eq!(offered(&pc, None), Some("edge"));
        // Firefox is the default: not one the sign-in knows.
        pc.https = Some(r"C:\Program Files\Mozilla Firefox\firefox.exe");
        assert_eq!(offered(&pc, None), Some("edge"));
        assert_eq!(offered(&pc_with(&[&BRAVE, &VIVALDI]), None), Some("brave"));
    }

    #[test]
    fn the_default_browser_is_offered_when_its_a_known_one() {
        let mut pc = pc_with(&[&EDGE, &CHROME, &BRAVE]);
        pc.https = Some(BRAVE_EXE);
        assert_eq!(offered(&pc, None), Some("brave"));
        // Windows' paths compare without case.
        pc.https = Some(r"c:\program files\google\chrome\application\CHROME.EXE");
        assert_eq!(offered(&pc, None), Some("chrome"));
    }

    #[test]
    fn the_one_used_last_comes_before_the_default() {
        let mut pc = pc_with(&[&EDGE, &CHROME, &BRAVE]);
        pc.https = Some(CHROME_EXE);
        assert_eq!(offered(&pc, Some("brave")), Some("brave"));
    }

    #[test]
    fn one_used_last_thats_gone_or_unknown_is_passed_over() {
        let mut pc = pc_with(&[&EDGE, &CHROME]);
        pc.https = Some(CHROME_EXE);
        assert_eq!(offered(&pc, Some("vivaldi")), Some("chrome"));
        assert_eq!(offered(&pc, Some("netscape")), Some("chrome"));
    }

    #[test]
    fn a_blocked_browser_gives_way_to_one_that_works() {
        // The one used last and the default are both blocked.
        let mut pc = pc_with(&[&EDGE, &CHROME, &BRAVE]);
        pc.https = Some(CHROME_EXE);
        policy(&mut pc, &CHROME, Hive::LocalMachine, 0);
        policy(&mut pc, &EDGE, Hive::LocalMachine, 0);
        assert_eq!(offered(&pc, Some("edge")), Some("brave"));
        // Among those that work, the usual order holds.
        let mut pc = pc_with(&[&EDGE, &CHROME, &BRAVE]);
        pc.https = Some(BRAVE_EXE);
        policy(&mut pc, &EDGE, Hive::LocalMachine, 0);
        assert_eq!(offered(&pc, None), Some("brave"));
        assert_eq!(offered(&pc, Some("chrome")), Some("chrome"));
    }

    #[test]
    fn with_every_browser_blocked_the_usual_one_is_offered() {
        let mut pc = pc_with(&[&EDGE, &CHROME]);
        policy(&mut pc, &EDGE, Hive::LocalMachine, 0);
        policy(&mut pc, &CHROME, Hive::LocalMachine, 0);
        assert_eq!(offered(&pc, None), Some("edge"));
        pc.https = Some(CHROME_EXE);
        assert_eq!(offered(&pc, None), Some("chrome"));
    }

    #[test]
    fn nothing_found_offers_nothing() {
        assert_eq!(offered(&pc(), Some("edge")), None);
    }
}
