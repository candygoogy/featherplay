//! `featherplay.log`, appended to line by line, and moved to `featherplay.1.log` (replacing
//! the one before) when the next line would take it past its size limit: two files at most.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

pub const CURRENT: &str = "featherplay.log";
pub const PREVIOUS: &str = "featherplay.1.log";
/// About 1 MB per file.
pub const LIMIT: u64 = 1024 * 1024;

pub struct LogFile {
    dir: PathBuf,
    limit: u64,
    file: File,
    size: u64,
}

impl LogFile {
    /// Opens (or creates) `dir/featherplay.log` for appending, creating `dir` too.
    pub fn open(dir: &Path, limit: u64) -> io::Result<LogFile> {
        std::fs::create_dir_all(dir)?;
        let file = append(&dir.join(CURRENT))?;
        let size = file.metadata()?.len();
        Ok(LogFile {
            dir: dir.to_path_buf(),
            limit,
            file,
            size,
        })
    }

    /// Writes `text` as it is (the caller ends it with a newline), rotating first if needed.
    pub fn write(&mut self, text: &str) -> io::Result<()> {
        let len = text.len() as u64;
        if self.size > 0 && self.size + len > self.limit {
            self.rotate()?;
        }
        self.file.write_all(text.as_bytes())?;
        self.size += len;
        Ok(())
    }

    /// The file becomes the one before (replacing that), and a new one starts.
    fn rotate(&mut self) -> io::Result<()> {
        let (current, previous) = (self.dir.join(CURRENT), self.dir.join(PREVIOUS));
        // The open handle doesn't stop a rename: Rust opens files with FILE_SHARE_DELETE.
        // When something else holds the old file, this one starts over instead, so the log
        // never grows past its limit.
        match std::fs::rename(&current, &previous) {
            Ok(()) => self.file = append(&current)?,
            Err(_) => self.file.set_len(0)?,
        }
        self.size = 0;
        Ok(())
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

fn append(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let base = std::env::var("FEATHERPLAY_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir().join("featherplay-tests"));
        let dir = base
            .join("test-diagnostics")
            .join(format!("{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn read(dir: &Path, name: &str) -> String {
        std::fs::read_to_string(dir.join(name)).unwrap_or_default()
    }

    #[test]
    fn lines_are_appended_across_opens() {
        let dir = scratch("append");
        LogFile::open(&dir, LIMIT).unwrap().write("one\n").unwrap();
        LogFile::open(&dir, LIMIT).unwrap().write("two\n").unwrap();
        assert_eq!(read(&dir, CURRENT), "one\ntwo\n");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_file_rotates_at_the_size_limit_keeping_two() {
        let dir = scratch("rotate");
        let mut log = LogFile::open(&dir, 100).unwrap();
        // 40 bytes a line: two fit under 100, the third rotates.
        let line = |n: usize| format!("{n:039}\n");
        for n in 0..7 {
            log.write(&line(n)).unwrap();
        }
        log.flush().unwrap();
        // 0 1 | 2 3 | 4 5 | 6: the file holds 6, the one before 4 and 5, the rest are gone.
        assert_eq!(read(&dir, CURRENT), line(6));
        assert_eq!(read(&dir, PREVIOUS), line(4) + &line(5));
        let files = std::fs::read_dir(&dir).unwrap().count();
        assert_eq!(files, 2, "only the log and the one before it are kept");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_reopened_file_counts_what_it_already_holds() {
        let dir = scratch("reopen");
        LogFile::open(&dir, 100)
            .unwrap()
            .write(&"a".repeat(80))
            .unwrap();
        LogFile::open(&dir, 100)
            .unwrap()
            .write(&"b".repeat(30))
            .unwrap();
        assert_eq!(read(&dir, PREVIOUS), "a".repeat(80));
        assert_eq!(read(&dir, CURRENT), "b".repeat(30));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
