//! Finding Edge, Chrome, Brave and Vivaldi on this PC (the registry, the https association,
//! the disk: `ThisPc`), and starting one with the DevTools pipe in a kill-on-close job
//! (`launch`). The browsers, and the order they're looked for in, are in `common`; how
//! they're found is in `find`.

mod find;
mod launch;

use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub use crate::common::browsers::*;
pub use find::{Hive, Lookup, found};
pub use launch::{Pipes, Running, launch};

use crate::window::Handle;

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

    fn https_handler(&self) -> Option<PathBuf> {
        https_handler()
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

/// The exe of the program registered for https links, as the shell resolves the user's
/// choice (`AssocQueryStringW`). Only asks; nothing is opened or changed.
fn https_handler() -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows::Win32::Foundation::S_OK;
    use windows::Win32::UI::Shell::{
        ASSOCF_IS_PROTOCOL, ASSOCF_NOTRUNCATE, ASSOCSTR_EXECUTABLE, AssocQueryStringW,
    };
    use windows::core::{PCWSTR, PWSTR, w};

    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    // SAFETY: the buffer and its length in characters are passed together, and
    // ASSOCF_NOTRUNCATE fails the call rather than write a cut path; the strings are
    // NUL-terminated literals.
    let result = unsafe {
        AssocQueryStringW(
            ASSOCF_IS_PROTOCOL | ASSOCF_NOTRUNCATE,
            ASSOCSTR_EXECUTABLE,
            w!("https"),
            PCWSTR::null(),
            Some(PWSTR(buf.as_mut_ptr())),
            &mut len,
        )
    };
    if result != S_OK {
        return None;
    }
    let end = buf.iter().position(|&c| c == 0)?;
    let exe = PathBuf::from(OsString::from_wide(&buf[..end]));
    exe.is_absolute().then_some(exe)
}

/// The visible top-level windows of the process `pid`. For the app's live
/// check that a browser exits when its window is closed.
pub fn visible_windows(pid: u32) -> Vec<Handle> {
    use windows::Win32::Foundation::{HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowThreadProcessId, IsWindowVisible,
    };
    use windows::core::BOOL;

    struct Find {
        pid: u32,
        found: Vec<Handle>,
    }
    unsafe extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
        // SAFETY: `lparam` is the `Find` passed to EnumWindows below, alive for the call.
        let find = unsafe { &mut *(lparam.0 as *mut Find) };
        let mut pid = 0u32;
        // SAFETY: a window handle Windows gave us and a live out pointer.
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut pid)) };
        // SAFETY: handle-only query.
        if pid == find.pid && unsafe { IsWindowVisible(hwnd) }.as_bool() {
            find.found.push(Handle::from_hwnd(hwnd));
        }
        true.into()
    }

    let mut find = Find {
        pid,
        found: Vec::new(),
    };
    // SAFETY: `each` matches WNDENUMPROC and `find` outlives the call.
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM((&raw mut find) as isize));
    }
    find.found
}

/// Asks `window` (one of [`visible_windows`]) to close, as its close button would.
pub fn close_window(window: Handle) {
    use windows::Win32::Foundation::{LPARAM, WPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};

    // SAFETY: only queues a message for a window of the browser we started.
    unsafe {
        let _ = PostMessageW(Some(window.hwnd()), WM_CLOSE, WPARAM(0), LPARAM(0));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What this PC really has: the known browsers found, the exe its https links open
    /// with, and the browser offered. Nothing is started.
    /// `cargo nextest run -p featherplay-platform --lib --run-ignored only -E 'test(this_pc)' --no-capture`
    #[test]
    #[ignore = "reads this PC's registry and reports what it finds"]
    #[allow(clippy::print_stderr, reason = "the live check reports what it saw")]
    fn this_pc_lists_its_browsers_and_its_default() {
        let found = found(&ThisPc);
        for f in &found {
            eprintln!(
                "found {}: {} (blocked: {:?})",
                f.browser.name,
                f.exe.display(),
                f.blocked
            );
            assert!(f.exe.is_file());
        }
        let default = ThisPc.https_handler();
        eprintln!("https links open with: {default:?}");
        let offered = pick(&found, None, default.as_deref()).map(|i| found[i].browser.name);
        eprintln!("offered: {offered:?}");
        // A fresh CI runner may have no default browser set: that's reported, not failed.
        assert!(default.is_none_or(|exe| exe.is_file()));
    }
}
