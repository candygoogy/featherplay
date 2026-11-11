//! Opening a URL or a folder with the desktop's default app (`xdg-open`).

use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Stdio};

/// Opens `url` in the default browser. It may take a moment: call it off the UI thread.
pub fn url(url: &str) -> Result<(), String> {
    xdg_open(OsStr::new(url))
}

/// Opens `path` in the file manager, or its default app for a file. It may take a moment:
/// call it off the UI thread.
pub fn path(path: &Path) -> Result<(), String> {
    xdg_open(path.as_os_str())
}

fn xdg_open(target: &OsStr) -> Result<(), String> {
    let status = Command::new("xdg-open")
        .arg(target)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("xdg-open: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("xdg-open ended with {status}"))
    }
}
