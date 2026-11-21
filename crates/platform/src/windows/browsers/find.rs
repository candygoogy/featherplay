//! Finding the browsers on a Windows PC: App Paths in the registry, the folders their
//! installers use, their `RemoteDebuggingAllowed` policies, and the default browser
//! (`Lookup`, which `ThisPc` reads and the tests stand in for). Where each fact comes from is
//! in `common::browsers`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::common::browsers::{ALL, Block, Browser, Found, same_path};

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
    /// skipped, and so is one that's another browser's (`registered_to`).
    pub fn find(&self, pc: &impl Lookup) -> Option<PathBuf> {
        let registered = [Hive::CurrentUser, Hive::LocalMachine]
            .into_iter()
            .filter_map(|hive| pc.app_path(hive, self.exe))
            .map(|v| PathBuf::from(v.trim().trim_matches('"')))
            .filter(|p| p.is_absolute())
            .filter(|p| registered_to(self.exe, p).is_none_or(|b| b.id == self.id));
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

/// Which browser `App Paths\{exe}` pointing at `path` is: the one whose install folder it's
/// in, else the first in `ALL` with that exe (the stable one). The channels of a brand share
/// their exe's registration, which the last one installed owns.
fn registered_to(exe: &str, path: &Path) -> Option<&'static Browser> {
    let path = path.as_os_str().to_string_lossy().to_ascii_lowercase();
    let in_folder = |b: &&Browser| {
        let tail = format!(r"\{}\{}", b.install_dir, b.exe).to_ascii_lowercase();
        path.ends_with(&tail)
    };
    let same_exe = || ALL.into_iter().filter(move |b| b.exe == exe);
    same_exe().find(in_folder).or_else(|| same_exe().next())
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

/// The user's default browser (what https links open with) is none of `found`: one the
/// sign-in can't open.
pub fn default_elsewhere(pc: &impl Lookup, found: &[Found]) -> bool {
    pc.https_handler()
        .is_some_and(|exe| !found.iter().any(|f| same_path(&exe, &f.exe)))
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
    fn a_policy_at_0_blocks_the_pipe_and_the_machines_wins_over_the_users() {
        // (case, the machine's policy, the user's, blocked)
        let cases = [
            ("the machine's at 0", Some(0), None, true),
            ("the user's at 0", None, Some(0), true),
            (
                "the machine's at 1 over the user's at 0",
                Some(1),
                Some(0),
                false,
            ),
            ("no policy", None, None, false),
            ("both at 1", Some(1), Some(1), false),
        ];
        for (label, machine, user, blocked) in cases {
            let mut pc = pc();
            for (hive, value) in [(Hive::LocalMachine, machine), (Hive::CurrentUser, user)] {
                if let Some(value) = value {
                    policy(&mut pc, &EDGE, hive, value);
                }
            }
            assert_eq!(EDGE.pipe_blocked(&pc), blocked, "{label}");
        }
    }

    /// A brand's channels and editions read its one policy key.
    #[test]
    fn each_policy_key_blocks_its_brand_only() {
        let keys: [(&str, &[&str]); 5] = [
            (
                r"Software\Policies\Microsoft\Edge",
                &["edge", "edge-beta", "edge-dev", "edge-canary"],
            ),
            (
                r"Software\Policies\Google\Chrome",
                &["chrome", "chrome-beta", "chrome-dev", "chrome-canary"],
            ),
            (
                r"Software\Policies\BraveSoftware\Brave",
                &["brave", "brave-beta", "brave-nightly"],
            ),
            (r"Software\Policies\Vivaldi", &["vivaldi"]),
            (r"Software\Policies\YandexBrowser", &["yandex"]),
        ];
        for (key, brand) in keys {
            let mut pc = pc();
            pc.dwords
                .insert((Hive::LocalMachine, key, "RemoteDebuggingAllowed"), 0);
            let blocked: Vec<&str> = ALL
                .iter()
                .filter(|b| b.pipe_blocked(&pc))
                .map(|b| b.id)
                .collect();
            assert_eq!(blocked, brand, "{key}");
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
    fn app_paths_come_first_then_program_files_x86_then_program_files() {
        const EDGE_X64: &str = r"C:\Program Files\Microsoft\Edge\Application\msedge.exe";
        const CHROME_USERS: &str =
            r"C:\Users\ada\AppData\Local\Google\Chrome\Application\chrome.exe";
        /// A case, the browser, its App Paths, the files on the PC, what's found.
        type Case<'a> = (
            &'a str,
            &'a Browser,
            &'a [(Hive, &'a str)],
            &'a [&'a str],
            Option<&'a str>,
        );
        let cases: [Case; 8] = [
            (
                "the user's App Path comes first",
                &EDGE,
                &[
                    (Hive::CurrentUser, r"D:\Edge\msedge.exe"),
                    (Hive::LocalMachine, r"E:\Edge\msedge.exe"),
                ],
                &[r"D:\Edge\msedge.exe", r"E:\Edge\msedge.exe"],
                Some(r"D:\Edge\msedge.exe"),
            ),
            (
                "the machine's App Path is next and may be quoted",
                &EDGE,
                &[(
                    Hive::LocalMachine,
                    r#""C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe""#,
                )],
                &[EDGE_EXE],
                Some(EDGE_EXE),
            ),
            (
                "a registration comes before Program Files",
                &EDGE,
                &[(Hive::LocalMachine, r"D:\Edge\msedge.exe")],
                &[EDGE_EXE, r"D:\Edge\msedge.exe"],
                Some(r"D:\Edge\msedge.exe"),
            ),
            (
                "a registration whose file is gone falls back to Program Files",
                &EDGE,
                &[(Hive::CurrentUser, r"D:\Gone\msedge.exe")],
                &[EDGE_X64],
                Some(EDGE_X64),
            ),
            (
                "Program Files (x86) comes before Program Files",
                &EDGE,
                &[],
                &[EDGE_X64, EDGE_EXE],
                Some(EDGE_EXE),
            ),
            ("nothing found", &EDGE, &[], &[], None),
            (
                "a relative registration isn't trusted",
                &EDGE,
                &[(Hive::CurrentUser, "msedge.exe")],
                &["msedge.exe"],
                None,
            ),
            (
                "the machine's install comes before the user's",
                &CHROME,
                &[],
                &[CHROME_USERS, CHROME_EXE],
                Some(CHROME_EXE),
            ),
        ];
        for (label, browser, app_paths, files, expected) in cases {
            let mut pc = pc();
            for &(hive, value) in app_paths {
                pc.app_paths.insert((hive, browser.exe), value);
            }
            pc.files = files.iter().map(PathBuf::from).collect();
            assert_eq!(browser.find(&pc), expected.map(PathBuf::from), "{label}");
        }
    }

    #[test]
    fn each_browser_is_found_where_its_installer_puts_it() {
        // (browser, a registration of its own, its machine install, its per-user install)
        let cases = [
            (&EDGE, r"D:\Apps\msedge.exe", EDGE_EXE, None),
            (
                &CHROME,
                r"D:\Apps\chrome.exe",
                CHROME_EXE,
                Some(r"C:\Users\ada\AppData\Local\Google\Chrome\Application\chrome.exe"),
            ),
            (
                &BRAVE,
                r"D:\Apps\brave.exe",
                BRAVE_EXE,
                Some(
                    r"C:\Users\ada\AppData\Local\BraveSoftware\Brave-Browser\Application\brave.exe",
                ),
            ),
            (
                &VIVALDI,
                r"D:\Apps\vivaldi.exe",
                r"C:\Program Files\Vivaldi\Application\vivaldi.exe",
                Some(VIVALDI_EXE),
            ),
        ];
        for (browser, registered, machine, per_user) in cases {
            let id = browser.id;
            // Its registration finds it, and none of the others.
            let mut pc = pc();
            pc.app_paths
                .insert((Hive::LocalMachine, browser.exe), registered);
            pc.files = vec![registered.into()];
            let found: Vec<&str> = ALL
                .iter()
                .filter(|b| b.find(&pc).is_some())
                .map(|b| b.id)
                .collect();
            assert_eq!(found, [id], "{id} by its registration");

            let mut pc = self::pc();
            pc.files = vec![machine.into()];
            assert_eq!(
                browser.find(&pc),
                Some(machine.into()),
                "{id} for the machine"
            );

            // One that installs for the machine only isn't looked for under LOCALAPPDATA.
            let mut pc = self::pc();
            pc.files =
                vec![per_user.map(PathBuf::from).unwrap_or_else(|| {
                    Path::new(LOCAL).join(browser.install_dir).join(browser.exe)
                })];
            let expected = per_user.map(PathBuf::from);
            assert_eq!(browser.find(&pc), expected, "{id} for one user");
        }
    }

    /// Each channel and edition where its installer puts it (for the machine, or for the
    /// user where it installs per user), found as itself and as nothing else.
    #[test]
    fn each_channel_is_found_in_its_own_folder() {
        let cases = [
            (
                &EDGE_BETA,
                r"C:\Program Files (x86)\Microsoft\Edge Beta\Application\msedge.exe",
            ),
            (
                &EDGE_DEV,
                r"C:\Program Files (x86)\Microsoft\Edge Dev\Application\msedge.exe",
            ),
            (
                &EDGE_CANARY,
                r"C:\Users\ada\AppData\Local\Microsoft\Edge SxS\Application\msedge.exe",
            ),
            (
                &CHROME_BETA,
                r"C:\Program Files\Google\Chrome Beta\Application\chrome.exe",
            ),
            (
                &CHROME_DEV,
                r"C:\Users\ada\AppData\Local\Google\Chrome Dev\Application\chrome.exe",
            ),
            (
                &CHROME_CANARY,
                r"C:\Users\ada\AppData\Local\Google\Chrome SxS\Application\chrome.exe",
            ),
            (
                &BRAVE_BETA,
                r"C:\Program Files\BraveSoftware\Brave-Browser-Beta\Application\brave.exe",
            ),
            (
                &BRAVE_NIGHTLY,
                r"C:\Users\ada\AppData\Local\BraveSoftware\Brave-Browser-Nightly\Application\brave.exe",
            ),
            (
                &YANDEX,
                r"C:\Users\ada\AppData\Local\Yandex\YandexBrowser\Application\browser.exe",
            ),
        ];
        for (browser, exe) in cases {
            let mut pc = pc();
            pc.files = vec![exe.into()];
            assert_eq!(
                at(&found(&pc)),
                [(browser.id, exe.into())],
                "{}",
                browser.id
            );
        }
    }

    /// Chrome and Chrome Beta both register `App Paths\chrome.exe`, and the last one
    /// installed owns it: pointing at Beta, it's Beta's, and Chrome is found in its folder.
    #[test]
    fn a_shared_app_path_is_taken_by_the_channel_whose_folder_it_is() {
        const BETA: &str = r"C:\Program Files\Google\Chrome Beta\Application\chrome.exe";
        let mut pc = pc();
        pc.app_paths
            .insert((Hive::LocalMachine, "chrome.exe"), BETA);
        pc.files = vec![BETA.into(), CHROME_EXE.into()];
        assert_eq!(
            at(&found(&pc)),
            [("chrome", CHROME_EXE.into()), ("chrome-beta", BETA.into())]
        );
        pc.files = vec![BETA.into()];
        assert_eq!(at(&found(&pc)), [("chrome-beta", BETA.into())]);
    }

    /// Which browser was found where.
    fn at(found: &[Found]) -> Vec<(&'static str, PathBuf)> {
        found
            .iter()
            .map(|f| (f.browser.id, f.exe.clone()))
            .collect()
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

    #[test]
    fn the_found_browsers_come_in_the_order_edge_chrome_brave_vivaldi() {
        let pc = pc_with(&[&VIVALDI, &BRAVE, &EDGE, &CHROME]);
        assert_eq!(ids(&found(&pc)), ["edge", "chrome", "brave", "vivaldi"]);
        let pc = pc_with(&[&VIVALDI, &CHROME]);
        assert_eq!(ids(&found(&pc)), ["chrome", "vivaldi"]);
        assert!(found(&self::pc()).is_empty());
    }

    /// The stable ones first, then the other channels.
    #[test]
    fn the_other_channels_come_after_the_stable_ones() {
        let mut pc = pc_with(&[&CHROME]);
        pc.files.extend(
            [
                r"C:\Program Files\Google\Chrome Beta\Application\chrome.exe",
                r"C:\Users\ada\AppData\Local\Microsoft\Edge SxS\Application\msedge.exe",
                r"C:\Users\ada\AppData\Local\Yandex\YandexBrowser\Application\browser.exe",
            ]
            .map(PathBuf::from),
        );
        assert_eq!(
            ids(&found(&pc)),
            ["chrome", "yandex", "edge-canary", "chrome-beta"]
        );
    }

    #[test]
    fn found_says_which_ones_a_policy_blocks() {
        let mut pc = pc_with(&[&EDGE, &CHROME]);
        policy(&mut pc, &CHROME, Hive::CurrentUser, 0);
        let blocked: Vec<bool> = found(&pc).iter().map(|f| f.blocked.is_some()).collect();
        assert_eq!(blocked, [false, true]);
    }

    #[test]
    fn a_default_browser_the_sign_in_cant_open_is_told_apart() {
        let mut pc = pc_with(&[&EDGE]);
        assert!(!default_elsewhere(&pc, &found(&pc)), "no default set");
        // The shell may give the path in another case.
        pc.https = Some(r"c:\program files (x86)\microsoft\edge\application\MSEDGE.EXE");
        assert!(!default_elsewhere(&pc, &found(&pc)));
        pc.https = Some(r"C:\Program Files\Mozilla Firefox\firefox.exe");
        assert!(default_elsewhere(&pc, &found(&pc)));
    }
}
