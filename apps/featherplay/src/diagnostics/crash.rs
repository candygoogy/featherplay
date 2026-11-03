//! Crash files: a panic writes `crash-<YYYYMMDD-HHMMSS>.txt` (its message, where it
//! happened, the thread and a backtrace) to the log folder, and only the newest few are kept.

use std::backtrace::Backtrace;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::panic::PanicHookInfo;
use std::path::{Path, PathBuf};

use super::clock::Stamp;
use super::redact;

/// How many crash files are kept.
pub const KEEP: usize = 5;

/// Chains a hook that writes a crash file into `dir` before the default one (which prints
/// the message to stderr) runs.
pub fn install(dir: PathBuf) {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        on_panic(&dir, info);
        default(info);
    }));
}

/// Writes the crash file for `info` and logs a line naming it. The text goes through the
/// redaction filter too: a panic message can quote anything.
pub fn on_panic(dir: &Path, info: &PanicHookInfo<'_>) -> Option<PathBuf> {
    let message = message(info.payload());
    let location = info
        .location()
        .map_or_else(|| "an unknown place".to_string(), ToString::to_string);
    let thread = std::thread::current();
    let thread = thread.name().unwrap_or("an unnamed thread");
    let text = redact(&report(
        message,
        &location,
        thread,
        &Backtrace::force_capture(),
    ));
    let written = write(dir, &Stamp::now(), &text);
    match &written {
        Ok(path) => super::write(format_args!(
            "panic in {thread} at {location}: {message} (crash file {})",
            path.file_name().unwrap_or_default().to_string_lossy()
        )),
        Err(e) => super::write(format_args!(
            "panic in {thread} at {location}: {message} (no crash file: {e})"
        )),
    }
    super::flush();
    written.ok()
}

/// The panic's message: a `&str` or `String` payload (what `panic!` makes), else a placeholder.
pub fn message(payload: &(dyn std::any::Any + Send)) -> &str {
    payload
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| payload.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("(a panic without a message)")
}

/// The crash file's text.
pub fn report(message: &str, location: &str, thread: &str, backtrace: &Backtrace) -> String {
    format!(
        "featherplay {} crashed.\n\
         Time: {} (after {:.3} s)\n\
         Windows: {}\n\
         Thread: {thread}\n\
         Where: {location}\n\
         Message: {message}\n\
         \n\
         Backtrace:\n{backtrace}\n",
        env!("CARGO_PKG_VERSION"),
        Stamp::now().line(),
        crate::controller::uptime().as_secs_f64(),
        super::windows(),
    )
}

/// Writes `text` to a new `crash-<stamp>.txt` in `dir` (`-2`, `-3`… when that second
/// already has one), then removes all but the newest `KEEP`.
pub fn write(dir: &Path, stamp: &Stamp, text: &str) -> io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let base = format!("crash-{}", stamp.file());
    let mut n = 1;
    let path = loop {
        let name = if n == 1 {
            format!("{base}.txt")
        } else {
            format!("{base}-{n}.txt")
        };
        let path = dir.join(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(mut file) => {
                file.write_all(text.as_bytes())?;
                break path;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists && n < 100 => n += 1,
            Err(e) => return Err(e),
        }
    };
    let all = files(dir);
    for old in &all[..all.len().saturating_sub(KEEP)] {
        let _ = std::fs::remove_file(old);
    }
    Ok(path)
}

/// The crash files in `dir`, oldest first: by their time, then their `-N`.
pub fn files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<((String, u32), PathBuf)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let stem = name.strip_prefix("crash-")?.strip_suffix(".txt")?;
            let (stamp, n) = match stem.get(15..) {
                Some("") => (stem, 1),
                Some(rest) => (&stem[..15], rest.strip_prefix('-')?.parse().ok()?),
                None => return None,
            };
            Some(((stamp.to_string(), n), e.path()))
        })
        .collect();
    found.sort();
    found.into_iter().map(|(_, path)| path).collect()
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
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn at(day: u16, second: u16) -> Stamp {
        Stamp {
            year: 2026,
            month: 11,
            day,
            hour: 10,
            minute: 0,
            second,
            millis: 0,
        }
    }

    fn names(dir: &Path) -> Vec<String> {
        files(dir)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn only_the_newest_five_are_kept() {
        let dir = scratch("keep");
        std::fs::write(dir.join("featherplay.log"), "the log stays").unwrap();
        for day in 1..=7 {
            write(&dir, &at(day, 0), "boom").unwrap();
        }
        assert_eq!(
            names(&dir),
            [3, 4, 5, 6, 7].map(|d| format!("crash-202611{d:02}-100000.txt"))
        );
        assert!(dir.join("featherplay.log").is_file());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn two_crashes_in_one_second_get_two_files() {
        let dir = scratch("same-second");
        let first = write(&dir, &at(2, 5), "first").unwrap();
        let second = write(&dir, &at(2, 5), "second").unwrap();
        assert_ne!(first, second);
        assert_eq!(std::fs::read_to_string(&first).unwrap(), "first");
        assert_eq!(std::fs::read_to_string(&second).unwrap(), "second");
        assert_eq!(
            names(&dir),
            ["crash-20261102-100005.txt", "crash-20261102-100005-2.txt"]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_message_comes_from_either_payload() {
        let s: Box<dyn std::any::Any + Send> = Box::new("static text");
        let owned: Box<dyn std::any::Any + Send> = Box::new(format!("owned {}", 1));
        let other: Box<dyn std::any::Any + Send> = Box::new(7_u8);
        assert_eq!(message(s.as_ref()), "static text");
        assert_eq!(message(owned.as_ref()), "owned 1");
        assert_eq!(message(other.as_ref()), "(a panic without a message)");
    }

    #[test]
    fn the_report_names_the_message_place_and_thread() {
        let text = report(
            "index out of bounds",
            "src/queue.rs:12:5",
            "featherplay-controller",
            &Backtrace::force_capture(),
        );
        assert!(text.contains("index out of bounds"), "{text}");
        assert!(text.contains("src/queue.rs:12:5"), "{text}");
        assert!(text.contains("featherplay-controller"), "{text}");
        assert!(text.contains(env!("CARGO_PKG_VERSION")), "{text}");
        assert!(text.contains("Backtrace:"), "{text}");
    }
}
