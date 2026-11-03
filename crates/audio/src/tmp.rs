//! The temp files HTTP streams are buffered in (`{cache}/tmp`). They are deleted when a
//! track is dropped, but a crash or a killed process leaves them behind, so the app clears
//! the stale ones at startup.

use std::fs::{self, OpenOptions};
use std::path::Path;

/// Every temp file the engine creates starts with this.
pub(crate) const TEMP_PREFIX: &str = "featherplay-audio-";

/// Deletes the engine's leftover temp files in `dir` and returns how many went. Only
/// regular files named `featherplay-audio-*` are touched, and only when no process has
/// them open (another running instance keeps its own). A missing `dir` is not an error.
pub fn clear_stale_temp(dir: &Path) -> usize {
    let Ok(entries) = fs::read_dir(dir) else {
        return 0;
    };
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let ours = name.to_str().is_some_and(|n| n.starts_with(TEMP_PREFIX));
        let is_file = entry.file_type().is_ok_and(|t| t.is_file());
        if !ours || !is_file {
            continue;
        }
        let path = entry.path();
        if in_use(&path) {
            continue;
        }
        if fs::remove_file(&path).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// Whether another handle has the file open: an exclusive open fails then.
#[cfg(windows)]
fn in_use(path: &Path) -> bool {
    use std::os::windows::fs::OpenOptionsExt;
    OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path)
        .is_err()
}

#[cfg(not(windows))]
fn in_use(path: &Path) -> bool {
    OpenOptions::new().read(true).open(path).is_err()
}
