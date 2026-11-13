use std::fs::{self, File};
use std::io::{self, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

/// Numbers the temp files, so two writes of one file at once (from two threads) each
/// rename their own instead of one taking the other's.
static WRITES: AtomicU64 = AtomicU64::new(0);

/// Writes `bytes` to `path` atomically: a temp file of its own in the same directory, then
/// a rename over the target. Creates the parent directory if needed. Two writes at once both
/// succeed; which lands last is the callers' to order.
pub fn write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let dir = path
        .parent()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no parent"))?;
    fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    let mut tmp_name = name.to_os_string();
    tmp_name.push(format!(
        ".{}-{}.tmp",
        std::process::id(),
        WRITES.fetch_add(1, Ordering::Relaxed)
    ));
    let tmp = dir.join(tmp_name);

    let result = (|| {
        let mut f = File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}
