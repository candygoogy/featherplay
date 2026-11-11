//! Opening a URL or a folder with the shell (`ShellExecuteW`).

use std::path::Path;

use windows::Win32::System::Com::{
    COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
use windows::core::{HSTRING, w};

/// Opens `url` in the default browser. It may take a moment: call it off the UI thread.
pub fn url(url: &str) -> Result<(), String> {
    shell_open(&HSTRING::from(url))
}

/// Opens `path` as Explorer would (a folder opens in a window of its own). It may take a
/// moment: call it off the UI thread.
pub fn path(path: &Path) -> Result<(), String> {
    shell_open(&HSTRING::from(path.as_os_str()))
}

/// The shell's "open" on `target`.
fn shell_open(target: &HSTRING) -> Result<(), String> {
    // SAFETY: COM is initialised on this thread for the shell call and released after; the
    // strings outlive the call.
    unsafe {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        let done = ShellExecuteW(None, w!("open"), target, None, None, SW_SHOWNORMAL);
        if com.is_ok() {
            CoUninitialize();
        }
        // Values above 32 mean success.
        if done.0 as isize > 32 {
            Ok(())
        } else {
            Err(format!("ShellExecute returned {}", done.0 as isize))
        }
    }
}
