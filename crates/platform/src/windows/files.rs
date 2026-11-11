//! Temporary files: whether one is in use, and positioned reads and writes.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::windows::fs::{FileExt, OpenOptionsExt};
use std::path::Path;

/// Whether another handle has the file open: an exclusive open fails then.
pub fn in_use(path: &Path) -> bool {
    OpenOptions::new()
        .read(true)
        .share_mode(0)
        .open(path)
        .is_err()
}

/// Writes all of `buf` at `pos`, whatever the file's cursor.
pub fn write_all_at(f: &File, mut buf: &[u8], mut pos: u64) -> io::Result<()> {
    while !buf.is_empty() {
        let n = f.seek_write(buf, pos)?;
        if n == 0 {
            return Err(io::ErrorKind::WriteZero.into());
        }
        buf = &buf[n..];
        pos += n as u64;
    }
    Ok(())
}

/// Fills `buf` from `pos`, whatever the file's cursor.
pub fn read_exact_at(f: &File, mut buf: &mut [u8], mut pos: u64) -> io::Result<()> {
    while !buf.is_empty() {
        let n = f.seek_read(buf, pos)?;
        if n == 0 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        buf = &mut buf[n..];
        pos += n as u64;
    }
    Ok(())
}
