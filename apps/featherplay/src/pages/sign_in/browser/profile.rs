//! The browser's temporary profile: `{cache}/signin-{pid}-{random}`, made for one sign-in
//! and deleted after it. A crash or a kill leaves it behind, so every start deletes the
//! ones other processes left (`sweep`), as the yt-dlp cookie files are swept.

use std::hash::{BuildHasher, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Every temporary profile's folder name starts with this.
pub const PREFIX: &str = "signin-";

/// Makes a new, empty profile folder in `cache` (created if needed).
pub fn create(cache: &Path) -> io::Result<PathBuf> {
    std::fs::create_dir_all(cache)?;
    let mut last = None;
    for _ in 0..8 {
        let path = cache.join(format!("{}{:016x}", ours(), random()));
        // `create_dir`, not `_all`: a folder that's already there is never reused.
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => last = Some(e),
            Err(e) => return Err(e),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("no free profile name")))
}

/// Writes `preferences` as the new `profile`'s `Default/Preferences`, which the browser reads
/// as it starts (`Browser::preferences`).
pub fn seed(profile: &Path, preferences: &str) -> io::Result<()> {
    let default = profile.join("Default");
    std::fs::create_dir_all(&default)?;
    std::fs::write(default.join("Preferences"), preferences)
}

/// Deletes `profile`, retrying while the browser's processes still hold files in it. Says
/// whether it's gone.
pub fn remove(profile: &Path, tries: u32, pause: Duration) -> bool {
    for i in 0..tries.max(1) {
        match std::fs::remove_dir_all(profile) {
            Ok(()) => return true,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return true,
            Err(_) if i + 1 < tries => std::thread::sleep(pause),
            Err(_) => {}
        }
    }
    !profile.exists()
}

/// Deletes the temporary profiles other processes left in `cache`; returns how many went.
/// Only folders (never a link) named `signin-*` directly in `cache` are touched, and never
/// this process's own.
pub fn sweep(cache: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(cache) else {
        return 0;
    };
    let mine = ours();
    let mut removed = 0;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        // `file_type` doesn't follow links: a junction or a symlink isn't a directory here.
        let is_dir = entry.file_type().is_ok_and(|t| t.is_dir());
        if !is_dir || !name.starts_with(PREFIX) || name.starts_with(&mine) {
            continue;
        }
        if std::fs::remove_dir_all(entry.path()).is_ok() {
            removed += 1;
        }
    }
    removed
}

/// This process's prefix: `signin-{pid}-`.
fn ours() -> String {
    format!("{PREFIX}{}-", std::process::id())
}

/// Unpredictable enough for a folder name (std's per-process random hash keys).
fn random() -> u64 {
    let mut h = std::hash::RandomState::new().build_hasher();
    h.write_u128(
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos(),
    );
    h.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scratch folder under the test home (`FEATHERPLAY_HOME`), or the temp dir.
    fn scratch(name: &str) -> PathBuf {
        let base = std::env::var_os("FEATHERPLAY_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let dir = base.join(format!("test-{name}-{}-{:x}", std::process::id(), random()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn a_profile_is_a_new_folder_under_the_cache_dir() {
        let cache = scratch("profile").join("cache");
        let a = create(&cache).unwrap();
        let b = create(&cache).unwrap();
        assert_ne!(a, b);
        for p in [&a, &b] {
            assert_eq!(p.parent(), Some(cache.as_path()));
            assert!(p.is_dir());
            let name = p.file_name().unwrap().to_str().unwrap();
            assert!(name.starts_with(&ours()), "{name}");
        }
        assert!(remove(&a, 1, Duration::ZERO));
        assert!(!a.exists());
        // Already gone counts as removed.
        assert!(remove(&a, 1, Duration::ZERO));
        std::fs::remove_dir_all(cache.parent().unwrap()).unwrap();
    }

    #[test]
    fn a_browsers_preferences_go_in_before_it_starts() {
        let cache = scratch("seed").join("cache");
        let p = create(&cache).unwrap();
        let prefs = crate::pages::sign_in::browser::known::VIVALDI
            .preferences
            .expect("Vivaldi's exit confirmation is turned off");
        seed(&p, prefs).unwrap();
        let written = std::fs::read_to_string(p.join("Default").join("Preferences")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&written).unwrap();
        assert_eq!(
            v["vivaldi"]["system"]["show_exit_confirmation_dialog"],
            false
        );
        std::fs::remove_dir_all(cache.parent().unwrap()).unwrap();
    }

    #[test]
    fn the_profile_follows_featherplay_home() {
        let home = scratch("home");
        let cache = featherplay_store::Paths::from_home(&home).cache;
        let p = create(&cache).unwrap();
        assert!(p.starts_with(&home), "{}", p.display());
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn the_sweep_deletes_only_other_processes_signin_folders() {
        let cache = scratch("sweep");
        let left = cache.join("signin-1-abc");
        std::fs::create_dir_all(left.join("Default")).unwrap();
        std::fs::write(left.join("Default").join("Cookies"), b"x").unwrap();
        let mine = create(&cache).unwrap();
        let file = cache.join("signin-2-file");
        std::fs::write(&file, b"x").unwrap();
        for keep in ["webview2", "tmp", "covers", "xsignin-3"] {
            std::fs::create_dir_all(cache.join(keep)).unwrap();
        }

        assert_eq!(sweep(&cache), 1);
        assert!(!left.exists());
        assert!(mine.exists(), "this process's own profile stays");
        assert!(file.exists(), "a file isn't a profile");
        for keep in ["webview2", "tmp", "covers", "xsignin-3"] {
            assert!(cache.join(keep).exists(), "{keep}");
        }
        assert_eq!(sweep(&cache.join("missing")), 0);
        std::fs::remove_dir_all(&cache).unwrap();
    }
}
