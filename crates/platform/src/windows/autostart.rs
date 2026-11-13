//! Starting featherplay when the user signs in to Windows: a value named `featherplay` under
//! `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`, the exe's path in quotes and
//! `--autostart` (`FLAG`). Per user, so it needs no admin rights, and the same for the
//! installed copy and the portable exe.
//!
//! Windows' own Startup apps list (Settings > Apps > Startup, Task Manager's Startup tab)
//! turns an entry off without deleting it: it writes a 12-byte value of the same name under
//! `...\Explorer\StartupApproved\Run` whose first byte is odd. featherplay reads that as off
//! (`State::TurnedOffBySystem`) and never turns it back on by itself; only the user turning
//! the setting on in featherplay clears it, as that's their latest word.

use std::path::Path;

use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    HKEY_CURRENT_USER, REG_BINARY, RRF_RT_REG_BINARY, RegDeleteKeyValueW, RegDeleteKeyW,
    RegDeleteTreeW, RegGetValueW, RegSetKeyValueW,
};
use windows::core::HSTRING;

pub use crate::common::autostart::*;
use crate::windows::identity::{read_hkcu_string, write_hkcu_strings};

/// Where Windows looks for the current user's startup programs.
const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
/// Where Windows' Startup apps list keeps the ones the user turned off.
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";
/// The value's name in both.
const NAME: &str = "featherplay";
/// Where tests keep their entries under HKCU (`for_tests`), never Windows' own keys.
const TEST_ROOT: &str = r"Software\featherplay-test";

/// The entry, under HKCU.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Autostart {
    run: String,
    approved: String,
}

impl Autostart {
    /// The current user's real entry.
    pub fn for_user() -> Result<Autostart, String> {
        Ok(Autostart::at(RUN, APPROVED))
    }

    /// An entry under other keys of HKCU.
    fn at(run: &str, approved: &str) -> Autostart {
        Autostart {
            run: run.to_string(),
            approved: approved.to_string(),
        }
    }

    /// An entry of a test's own, named `name`, under `HKCU\Software\featherplay-test`: never
    /// the one Windows runs. `remove_for_tests` deletes it.
    #[doc(hidden)]
    pub fn for_tests(name: &str) -> Autostart {
        let root = format!(r"{TEST_ROOT}\{name}-{}", std::process::id());
        Autostart::at(
            &format!(r"{root}\Run"),
            &format!(r"{root}\StartupApproved\Run"),
        )
    }

    /// Turns the entry off as Windows' Startup apps does (a first byte of 3), for tests.
    #[doc(hidden)]
    pub fn turn_off_for_tests(&self) -> Result<(), String> {
        let mut bytes = [0u8; 12];
        bytes[0] = 3;
        set_binary(&self.approved, NAME, &bytes)
    }

    /// Deletes a `for_tests` entry's keys; any other entry is left alone.
    #[doc(hidden)]
    pub fn remove_for_tests(&self) {
        let Some(root) = self
            .run
            .strip_suffix(r"\Run")
            .filter(|r| r.starts_with(TEST_ROOT))
        else {
            return;
        };
        // SAFETY: HKCU is predefined; the subkey is a test's own.
        let _ = unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, &HSTRING::from(root)) };
        // The tests' folder too, once no other test's entry is in it (RegDeleteKeyW only
        // deletes a key without subkeys).
        // SAFETY: as above.
        let _ = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, &HSTRING::from(TEST_ROOT)) };
    }

    /// Whether featherplay starts at sign-in, as Windows will see it.
    pub fn state(&self) -> State {
        if read_hkcu_string(&self.run, NAME).is_none() {
            return State::Off;
        }
        match read_binary(&self.approved, NAME) {
            Some(bytes) if turned_off(&bytes) => State::TurnedOffBySystem,
            _ => State::On,
        }
    }

    /// The command the entry runs, if there's one.
    pub fn command(&self) -> Option<String> {
        read_hkcu_string(&self.run, NAME)
    }

    /// Starts `exe` at sign-in from now on, and clears a "turned off" Startup apps left.
    pub fn enable(&self, exe: &Path) -> Result<(), String> {
        write_hkcu_strings(&self.run, &[(NAME, command_line(exe))])?;
        delete_value(&self.approved, NAME)
    }

    /// Takes the entry away, and Windows' note about it.
    pub fn disable(&self) -> Result<(), String> {
        let run = delete_value(&self.run, NAME);
        let approved = delete_value(&self.approved, NAME);
        run.and(approved)
    }

    /// Points an entry that's there at `exe`, if it names another path (the exe moved, or the
    /// portable copy was replaced by the installed one). Turned off in Startup apps it stays
    /// off: Windows keeps that by the value's name. Returns whether it changed anything.
    pub fn refresh(&self, exe: &Path) -> Result<bool, String> {
        let Some(current) = self.command() else {
            return Ok(false);
        };
        let wanted = command_line(exe);
        if current.eq_ignore_ascii_case(&wanted) {
            return Ok(false);
        }
        write_hkcu_strings(&self.run, &[(NAME, wanted)])?;
        Ok(true)
    }
}

/// The program featherplay's entry should start: this exe.
pub fn launch_path() -> Result<std::path::PathBuf, String> {
    std::env::current_exe().map_err(|e| format!("the exe's path: {e}"))
}

/// What the Run value holds for `exe`: its path in quotes (it may have spaces), then the flag.
pub fn command_line(exe: &Path) -> String {
    format!("\"{}\" {FLAG}", exe.display())
}

/// Whether a StartupApproved value says the entry is turned off: its first byte is odd
/// (2 and 6 are on, 3 and 7 off).
fn turned_off(bytes: &[u8]) -> bool {
    bytes.first().is_some_and(|b| b & 1 == 1)
}

/// A `REG_BINARY` value under HKCU, if it's there.
fn read_binary(subkey: &str, name: &str) -> Option<Vec<u8>> {
    let mut buf = [0u8; 64];
    let mut len = buf.len() as u32;
    // SAFETY: the buffer and its size in bytes are passed together.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(subkey),
            &HSTRING::from(name),
            RRF_RT_REG_BINARY,
            None,
            Some(buf.as_mut_ptr().cast()),
            Some(&mut len),
        )
    };
    (status == ERROR_SUCCESS).then(|| buf[..(len as usize).min(buf.len())].to_vec())
}

/// Sets a `REG_BINARY` value under HKCU, making the key if needed.
fn set_binary(subkey: &str, name: &str, bytes: &[u8]) -> Result<(), String> {
    // SAFETY: HKCU is predefined; the data and its length go together.
    let status = unsafe {
        RegSetKeyValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(subkey),
            &HSTRING::from(name),
            REG_BINARY.0,
            Some(bytes.as_ptr().cast()),
            bytes.len() as u32,
        )
    };
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(format!(
            "set HKCU\\{subkey}\\{name}: {}",
            status.to_hresult().message()
        ))
    }
}

/// Deletes a value under HKCU; one that isn't there is fine.
fn delete_value(subkey: &str, name: &str) -> Result<(), String> {
    // SAFETY: HKCU is a predefined key; both strings are NUL-terminated and outlive the call.
    let status = unsafe {
        RegDeleteKeyValueW(
            HKEY_CURRENT_USER,
            &HSTRING::from(subkey),
            &HSTRING::from(name),
        )
    };
    if status == ERROR_SUCCESS || status == ERROR_FILE_NOT_FOUND {
        Ok(())
    } else {
        Err(format!(
            "delete HKCU\\{subkey}\\{name}: {}",
            status.to_hresult().message()
        ))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    /// A test's own entry (`for_tests`), deleted on drop.
    struct TestEntry(Autostart);

    impl TestEntry {
        fn new(name: &str) -> TestEntry {
            let a = Autostart::for_tests(name);
            assert!(!a.run.starts_with(RUN) && !a.approved.starts_with(APPROVED));
            TestEntry(a)
        }
    }

    impl Drop for TestEntry {
        fn drop(&mut self) {
            self.0.remove_for_tests();
        }
    }

    #[test]
    fn the_command_quotes_the_path_and_adds_the_flag() {
        let exe = Path::new(r"C:\Users\u\AppData\Local\Programs\featherplay\featherplay.exe");
        assert_eq!(
            command_line(exe),
            r#""C:\Users\u\AppData\Local\Programs\featherplay\featherplay.exe" --autostart"#
        );
        assert!(turned_off(&[3, 0]) && turned_off(&[7]));
        assert!(!turned_off(&[2, 0]) && !turned_off(&[6]) && !turned_off(&[]));
    }

    #[test]
    fn on_off_and_windows_turning_it_off() {
        let entry = TestEntry::new("autostart");
        let a = &entry.0;
        let exe = PathBuf::from(r"E:\Apps\featherplay\featherplay.exe");
        assert_eq!(a.state(), State::Off);
        a.enable(&exe).unwrap();
        assert_eq!(a.state(), State::On);
        assert_eq!(a.command(), Some(command_line(&exe)));
        // Turned on in Startup apps (a first byte of 2): still on.
        let mut on = [0u8; 12];
        on[0] = 2;
        set_binary(&a.approved, NAME, &on).unwrap();
        assert_eq!(a.state(), State::On);
        // Turned off in Startup apps: off, and a moved exe keeps it off.
        a.turn_off_for_tests().unwrap();
        assert_eq!(a.state(), State::TurnedOffBySystem);
        let moved = PathBuf::from(r"F:\featherplay.exe");
        assert!(a.refresh(&moved).unwrap());
        assert_eq!(a.state(), State::TurnedOffBySystem);
        assert_eq!(a.command(), Some(command_line(&moved)));
        // The user turns it on in featherplay: it's on again.
        a.enable(&moved).unwrap();
        assert_eq!(a.state(), State::On);
        a.disable().unwrap();
        assert_eq!(a.state(), State::Off);
        a.disable().unwrap();
    }

    #[test]
    fn refresh_changes_only_an_entry_that_points_elsewhere() {
        let entry = TestEntry::new("autostart-refresh");
        let a = &entry.0;
        let exe = PathBuf::from(r"E:\Apps\featherplay\featherplay.exe");
        // No entry: nothing is made.
        assert!(!a.refresh(&exe).unwrap());
        assert_eq!(a.state(), State::Off);
        a.enable(&exe).unwrap();
        assert!(
            !a.refresh(Path::new(r"e:\apps\Featherplay\featherplay.EXE"))
                .unwrap()
        );
        assert!(a.refresh(Path::new(r"F:\featherplay.exe")).unwrap());
    }
}
