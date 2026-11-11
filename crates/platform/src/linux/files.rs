//! Temporary files: whether one is in use, and positioned reads and writes.

use std::fs::{File, OpenOptions};
use std::io;
use std::path::Path;

/// Whether the file can't be opened. Linux has no exclusive open, so a file another
/// process has open still counts as free.
pub fn in_use(path: &Path) -> bool {
    OpenOptions::new().read(true).open(path).is_err()
}

/// Writes all of `buf` at `pos`, whatever the file's cursor.
pub fn write_all_at(f: &File, buf: &[u8], pos: u64) -> io::Result<()> {
    std::os::unix::fs::FileExt::write_all_at(f, buf, pos)
}

/// Fills `buf` from `pos`, whatever the file's cursor.
pub fn read_exact_at(f: &File, buf: &mut [u8], pos: u64) -> io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(f, buf, pos)
}
