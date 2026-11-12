//! Finding the browsers on a Windows PC: App Paths in the registry, the folders their
//! installers use, their `RemoteDebuggingAllowed` policies, and the default browser
//! (`Lookup`, which `ThisPc` reads and the tests stand in for). Where each fact comes from is
//! in `common::browsers`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::common::browsers::{ALL, Block, Browser, Found};

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

    /// Why the sign-in can't use this browser, if it can't: on Windows, only a policy.
    pub fn blocked(&self, pc: &impl Lookup) -> Option<Block> {
        self.pipe_blocked(pc).then_some(Block::Policy)
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

// The PCs these tests stand in are Windows ones, with Windows' paths (`C:\…` is absolute
// only there), as is the search they check.
#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::common::browsers::*;
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
        let blocked: Vec<bool> = found(&pc).iter().map(|f| f.blocked.is_some()).collect();
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
