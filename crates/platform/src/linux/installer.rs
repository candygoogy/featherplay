//! The AppImage this copy runs from, replacing it with a newer one, and starting that one
//! once this process has ended. Only an AppImage updates in place
//! (`supports(Feature::InPlaceUpdate)`): a copy from the .tar.gz or a distribution's package
//! is only told about a newer version.
//!
//! The AppImage runtime sets `APPIMAGE` (the image's own path) and `APPDIR` (where it
//! mounted or extracted it) before it starts the program inside.

use std::ffi::{CString, OsStr};
use std::fs::{self, File, OpenOptions, Permissions};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Waits for the process `$1` to end, a tenth of a second at a time and 30 s at most, then
/// runs the rest of its arguments. A program that outlives the wait starts anyway, and then
/// finds the old copy still running (`instance`).
const WAIT_THEN_RUN: &str = r#"pid=$1; shift; n=0
while kill -0 "$pid" 2>/dev/null && [ "$n" -lt 300 ]; do sleep 0.1; n=$((n + 1)); done
exec "$@""#;

/// The AppImage this process runs from, if it can be replaced: the runtime's `APPIMAGE`, an
/// absolute path to a file in a folder this user may write, with this program inside the
/// runtime's `APPDIR`. A stray `APPIMAGE` (inherited from another AppImage by a copy started
/// from the .tar.gz, say) doesn't count.
pub fn appimage() -> Option<PathBuf> {
    let image = PathBuf::from(std::env::var_os("APPIMAGE")?);
    let appdir = PathBuf::from(std::env::var_os("APPDIR")?);
    let exe = std::env::current_exe().ok()?;
    appimage_at(&image, &appdir, &exe)
}

/// `appimage` for these paths.
fn appimage_at(image: &Path, appdir: &Path, exe: &Path) -> Option<PathBuf> {
    if !image.is_absolute() || !appdir.is_absolute() {
        return None;
    }
    let appdir = fs::canonicalize(appdir).ok()?;
    let inside = fs::canonicalize(exe).is_ok_and(|e| e.starts_with(&appdir));
    let file = fs::metadata(image).is_ok_and(|m| m.is_file());
    let folder = image.parent()?;
    (inside && file && writable(folder)).then(|| image.to_path_buf())
}

/// Whether this user may create files in `dir`.
fn writable(dir: &Path) -> bool {
    let Ok(path) = CString::new(dir.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: `path` is a NUL-terminated string that lives across the call.
    unsafe { libc::access(path.as_ptr(), libc::W_OK) == 0 }
}

/// Replaces `target` with what `write` writes, all at once: into a new file beside it, which
/// is flushed to the disk, given `target`'s permissions (executable for its owner at
/// least) and renamed over it. If `write` fails, or anything after it, `target` is as it was
/// and the new file is gone.
pub fn replace_file(
    target: &Path,
    write: impl FnOnce(&mut File) -> io::Result<()>,
) -> io::Result<()> {
    let (Some(dir), Some(name)) = (target.parent(), target.file_name()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "not a file path",
        ));
    };
    let mode = fs::metadata(target).map_or(0o755, |m| m.permissions().mode() & 0o7777) | 0o100;
    let mut part_name = OsStr::new(".").to_os_string();
    part_name.push(name);
    part_name.push(".update");
    let part = dir.join(part_name);
    let _ = fs::remove_file(&part);
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o700)
            .open(&part)?;
        write(&mut file)?;
        file.sync_all()?;
        fs::set_permissions(&part, Permissions::from_mode(mode))?;
        fs::rename(&part, target)?;
        // The rename itself, on the disk.
        File::open(dir)?.sync_all()
    })();
    if result.is_err() {
        let _ = fs::remove_file(&part);
    }
    result
}

/// The command that starts the AppImage at `image` as this one was started: extracted, not
/// mounted, if this one was (the runtime's `--appimage-extract-and-run`, where there's no
/// FUSE), else as it is.
pub fn appimage_command(image: &Path) -> Command {
    let mut command = Command::new(image);
    let mounted = std::env::var_os("APPDIR").is_some_and(|dir| is_mount_point(Path::new(&dir)));
    if !mounted {
        command.env("APPIMAGE_EXTRACT_AND_RUN", "1");
    }
    command
}

/// Whether `dir` is a mount point, as `/proc/self/mountinfo` lists them.
fn is_mount_point(dir: &Path) -> bool {
    let Ok(dir) = fs::canonicalize(dir) else {
        return false;
    };
    let Ok(mounts) = fs::read_to_string("/proc/self/mountinfo") else {
        return false;
    };
    mounts
        .lines()
        .filter_map(|line| line.split(' ').nth(4))
        .any(|point| unescape(point) == dir.as_os_str().as_bytes())
}

/// A mountinfo field with its octal escapes (`\040` for a space) undone.
fn unescape(field: &str) -> Vec<u8> {
    let bytes = field.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let octal = bytes
            .get(i + 1..i + 4)
            .filter(|d| d.iter().all(|b| (b'0'..=b'7').contains(b)));
        match (bytes[i], octal) {
            (b'\\', Some(d)) => {
                out.push((d[0] - b'0') * 64 + (d[1] - b'0') * 8 + (d[2] - b'0'));
                i += 4;
            }
            (b, _) => {
                out.push(b);
                i += 1;
            }
        }
    }
    out
}

/// Starts the program `command()` names, with its arguments and environment, once this
/// process has ended (`WAIT_THEN_RUN`), so that it doesn't find this copy still running. It
/// runs in a process group of its own, which outlives this one and the terminal it was
/// started from.
pub fn start_setup(command: impl Fn() -> Command) -> io::Result<()> {
    start_after(std::process::id(), &command())
}

fn start_after(pid: u32, command: &Command) -> io::Result<()> {
    let mut sh = Command::new("/bin/sh");
    sh.arg("-c")
        .arg(WAIT_THEN_RUN)
        .arg("featherplay-restart")
        .arg(pid.to_string())
        .arg(command.get_program())
        .args(command.get_args());
    for (key, value) in command.get_envs() {
        match value {
            Some(value) => sh.env(key, value),
            None => sh.env_remove(key),
        };
    }
    sh.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map(drop)
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::time::{Duration, Instant};

    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let base = std::env::var_os("FEATHERPLAY_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let dir = base
            .join("test-installer")
            .join(format!("{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn names(dir: &Path) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(dir)
            .unwrap()
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_replaced_file_has_the_new_bytes_and_stays_executable() {
        let dir = scratch("replace");
        let target = dir.join("featherplay-x86_64.AppImage");
        fs::write(&target, b"old").unwrap();
        fs::set_permissions(&target, Permissions::from_mode(0o750)).unwrap();

        replace_file(&target, |f| f.write_all(b"new")).unwrap();

        assert_eq!(fs::read(&target).unwrap(), b"new");
        let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode, 0o750);
        assert_eq!(names(&dir), ["featherplay-x86_64.AppImage"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_failed_write_leaves_the_old_file_and_no_other() {
        let dir = scratch("replace-fails");
        let target = dir.join("featherplay-x86_64.AppImage");
        fs::write(&target, b"old").unwrap();
        fs::set_permissions(&target, Permissions::from_mode(0o644)).unwrap();

        let err = replace_file(&target, |f| {
            f.write_all(b"half")?;
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "sha256 mismatch",
            ))
        })
        .unwrap_err();

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
        assert_eq!(fs::read(&target).unwrap(), b"old");
        assert_eq!(names(&dir), ["featherplay-x86_64.AppImage"]);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_an_image_this_program_runs_from_counts() {
        let dir = scratch("appimage");
        let image = dir.join("featherplay-x86_64.AppImage");
        fs::write(&image, b"image").unwrap();
        let appdir = dir.join("mount");
        fs::create_dir_all(appdir.join("usr/bin")).unwrap();
        let exe = appdir.join("usr/bin/featherplay");
        fs::write(&exe, b"exe").unwrap();

        assert_eq!(appimage_at(&image, &appdir, &exe), Some(image.clone()));
        let elsewhere = dir.join("featherplay");
        fs::write(&elsewhere, b"exe").unwrap();
        assert_eq!(
            appimage_at(&image, &appdir, &elsewhere),
            None,
            "a stray APPIMAGE"
        );
        assert_eq!(appimage_at(&dir.join("gone.AppImage"), &appdir, &exe), None);
        assert_eq!(appimage_at(Path::new("rel.AppImage"), &appdir, &exe), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn mountinfo_escapes_are_undone() {
        assert_eq!(unescape(r"/tmp/.mount_feath\040x"), b"/tmp/.mount_feath x");
        assert_eq!(unescape(r"/a\b"), b"/a\\b");
        assert!(is_mount_point(Path::new("/")));
        assert!(!is_mount_point(Path::new("/definitely/not/here")));
    }

    #[test]
    fn the_program_starts_once_the_process_has_ended() {
        let dir = scratch("start-after");
        let mark = dir.join("started");
        let mut waited = Command::new("sleep").arg("30").spawn().unwrap();
        let mut command = Command::new("/bin/sh");
        command
            .arg("-c")
            .arg(r#"printf '%s' "$MARK_TEXT" > "$1""#)
            .arg("sh")
            .arg(&mark)
            .env("MARK_TEXT", "after");

        start_after(waited.id(), &command).unwrap();

        std::thread::sleep(Duration::from_millis(300));
        assert!(!mark.exists(), "started before the process ended");
        // Ended and reaped, so `kill -0` no longer finds it.
        waited.kill().unwrap();
        waited.wait().unwrap();
        let until = Instant::now() + Duration::from_secs(10);
        while !mark.exists() && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_eq!(fs::read_to_string(&mark).unwrap(), "after");
        fs::remove_dir_all(&dir).unwrap();
    }
}
