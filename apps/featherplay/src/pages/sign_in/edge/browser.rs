//! Which browser the sign-in opens, and where it's installed. Only Microsoft Edge for now;
//! another Chromium browser (Chrome) would be one more `Browser`. Firefox has no CDP pipe.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// A Chromium browser the sign-in can drive over the DevTools pipe.
#[derive(Debug)]
pub struct Browser {
    /// As the user knows it, for the log.
    pub name: &'static str,
    /// Its registration under `Software\Microsoft\Windows\CurrentVersion\App Paths`.
    pub exe: &'static str,
    /// Where its installer puts the exe, under `%ProgramFiles(x86)%` or `%ProgramFiles%`.
    pub install_dir: &'static str,
    /// Its switches besides the profile and the pipe: no welcome pages, no "make me the
    /// default" bar. Nothing that marks the browser as automated.
    pub switches: &'static [&'static str],
    /// Its policies' key, in HKLM and HKCU. Its `RemoteDebuggingAllowed` set to 0 turns
    /// the DevTools pipe off.
    pub policy_key: &'static str,
}

pub const EDGE: Browser = Browser {
    name: "Microsoft Edge",
    exe: "msedge.exe",
    install_dir: r"Microsoft\Edge\Application",
    switches: &["--no-first-run", "--no-default-browser-check"],
    policy_key: r"Software\Policies\Microsoft\Edge",
};

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
}

impl Browser {
    /// The browser's exe: App Paths (the user's hive, then the machine's), then the
    /// install folder under `%ProgramFiles(x86)%`, then under `%ProgramFiles%`. A
    /// registration whose file is gone is skipped.
    pub fn find(&self, pc: &impl Lookup) -> Option<PathBuf> {
        let registered = [Hive::CurrentUser, Hive::LocalMachine]
            .into_iter()
            .filter_map(|hive| pc.app_path(hive, self.exe))
            .map(|v| PathBuf::from(v.trim().trim_matches('"')))
            .filter(|p| p.is_absolute());
        let installed = ["ProgramFiles(x86)", "ProgramFiles"]
            .into_iter()
            .filter_map(|var| pc.env(var))
            .map(|base| PathBuf::from(base).join(self.install_dir).join(self.exe));
        registered.chain(installed).find(|p| pc.is_file(p))
    }

    /// A policy (the machine's or the user's) turns off the DevTools pipe the sign-in
    /// needs: the browser would open and never answer.
    pub fn pipe_blocked(&self, pc: &impl Lookup) -> bool {
        [Hive::LocalMachine, Hive::CurrentUser]
            .into_iter()
            .any(|hive| pc.dword(hive, self.policy_key, DEBUGGING_POLICY) == Some(0))
    }
}

/// This PC: the registry, the environment, the disk.
pub struct ThisPc;

impl Lookup for ThisPc {
    fn app_path(&self, hive: Hive, exe: &str) -> Option<String> {
        app_path(hive, exe)
    }

    fn env(&self, var: &str) -> Option<OsString> {
        std::env::var_os(var).filter(|v| !v.is_empty())
    }

    fn is_file(&self, path: &Path) -> bool {
        path.is_file()
    }

    fn dword(&self, hive: Hive, key: &str, value: &str) -> Option<u32> {
        dword(hive, key, value)
    }
}

fn root(hive: Hive) -> windows::Win32::System::Registry::HKEY {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    match hive {
        Hive::CurrentUser => HKEY_CURRENT_USER,
        Hive::LocalMachine => HKEY_LOCAL_MACHINE,
    }
}

/// Reads one DWORD; never writes.
fn dword(hive: Hive, key: &str, value: &str) -> Option<u32> {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{RRF_RT_REG_DWORD, RegGetValueW};
    use windows::core::HSTRING;

    let (key, value) = (HSTRING::from(key), HSTRING::from(value));
    let mut data = 0u32;
    let mut len = size_of::<u32>() as u32;
    // SAFETY: a u32 out value with its size in bytes; RRF_RT_REG_DWORD refuses any other
    // type, so nothing longer is written.
    let status = unsafe {
        RegGetValueW(
            root(hive),
            &key,
            &value,
            RRF_RT_REG_DWORD,
            None,
            Some((&raw mut data).cast()),
            Some(&mut len),
        )
    };
    (status == ERROR_SUCCESS).then_some(data)
}

fn app_path(hive: Hive, exe: &str) -> Option<String> {
    use windows::Win32::Foundation::ERROR_SUCCESS;
    use windows::Win32::System::Registry::{RRF_RT_REG_SZ, RegGetValueW};
    use windows::core::{HSTRING, PCWSTR};

    let key = HSTRING::from(format!(
        r"Software\Microsoft\Windows\CurrentVersion\App Paths\{exe}"
    ));
    let mut buf = vec![0u16; 1024];
    let mut len = (buf.len() * 2) as u32;
    // SAFETY: the buffer and its size in bytes are passed together; a null value name reads
    // the key's default value; RRF_RT_REG_SZ makes the result NUL-terminated (a REG_EXPAND_SZ
    // comes back expanded).
    let status = unsafe {
        RegGetValueW(
            root(hive),
            &key,
            PCWSTR::null(),
            RRF_RT_REG_SZ,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut len),
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    let chars = (len as usize / 2).min(buf.len());
    let end = buf[..chars].iter().position(|&c| c == 0).unwrap_or(chars);
    buf.truncate(end);
    let s = String::from_utf16(&buf).ok()?;
    (!s.trim().is_empty()).then_some(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    const X86: &str = r"C:\Program Files (x86)";
    const X64: &str = r"C:\Program Files";

    #[derive(Default)]
    struct FakePc {
        app_paths: HashMap<(Hive, &'static str), &'static str>,
        env: HashMap<&'static str, &'static str>,
        files: Vec<PathBuf>,
        dwords: HashMap<(Hive, &'static str, &'static str), u32>,
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
    }

    fn policy(pc: &mut FakePc, hive: Hive, value: u32) {
        pc.dwords.insert(
            (
                hive,
                r"Software\Policies\Microsoft\Edge",
                "RemoteDebuggingAllowed",
            ),
            value,
        );
    }

    #[test]
    fn the_machines_policy_at_0_blocks_the_pipe() {
        let mut pc = pc();
        policy(&mut pc, Hive::LocalMachine, 0);
        assert!(EDGE.pipe_blocked(&pc));
    }

    #[test]
    fn the_users_policy_at_0_blocks_the_pipe() {
        let mut pc = pc();
        policy(&mut pc, Hive::CurrentUser, 0);
        assert!(EDGE.pipe_blocked(&pc));
    }

    #[test]
    fn no_policy_leaves_the_pipe_alone() {
        assert!(!EDGE.pipe_blocked(&pc()));
    }

    #[test]
    fn the_policy_at_1_leaves_the_pipe_alone() {
        let mut pc = pc();
        policy(&mut pc, Hive::LocalMachine, 1);
        policy(&mut pc, Hive::CurrentUser, 1);
        assert!(!EDGE.pipe_blocked(&pc));
    }

    fn pc() -> FakePc {
        FakePc {
            env: HashMap::from([("ProgramFiles(x86)", X86), ("ProgramFiles", X64)]),
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
        pc.files = vec![r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe".into()];
        assert_eq!(
            EDGE.find(&pc),
            Some(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe".into())
        );
    }

    #[test]
    fn a_registration_comes_before_program_files() {
        let mut pc = pc();
        pc.app_paths
            .insert((Hive::LocalMachine, "msedge.exe"), r"D:\Edge\msedge.exe");
        pc.files = vec![
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe".into(),
            r"D:\Edge\msedge.exe".into(),
        ];
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
            r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe".into(),
        ];
        assert_eq!(
            EDGE.find(&pc),
            Some(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe".into())
        );
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
    fn edges_switches_mark_nothing_as_automated() {
        for s in EDGE.switches {
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
}
