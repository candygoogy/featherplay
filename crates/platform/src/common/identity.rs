//! The app's identity for the shell: its names, where its registration lives, and whether
//! this run may write it. What writes it is each OS's (`identity::register`).

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// The AppUserModelID, in Microsoft's `Company.Product` form (no spaces, 128 characters at
/// most). No version part, so upgrades keep the same identity. Changing it orphans the
/// shortcut and registry key written under the old one, so it changes only with the name.
pub const AUMID: &str = "featherplay.featherplay";
/// The name the media card, Start and the notification settings show.
pub const DISPLAY_NAME: &str = "featherplay";
/// The Start menu shortcut, directly in the user's Programs folder.
pub const SHORTCUT_FILE: &str = "featherplay.lnk";
/// The icon `IconUri` points at, in the app's data dir.
pub const ICON_FILE: &str = "featherplay.ico";

/// Whether this run may write or remove the per-user registration: only a release build
/// run without `FEATHERPLAY_HOME`, which is how a user runs the app. Dev runs and tests set
/// `FEATHERPLAY_HOME` (and `cargo test` builds are debug), so they never touch the
/// Start menu or the registry. An empty `FEATHERPLAY_HOME` counts as unset, as in
/// `Paths::resolve`.
pub fn registration_allowed(release: bool, featherplay_home: Option<&OsStr>) -> bool {
    release && featherplay_home.is_none_or(OsStr::is_empty)
}

/// `registration_allowed` for this build and environment.
pub fn registration_allowed_here() -> bool {
    registration_allowed(
        !cfg!(debug_assertions),
        std::env::var_os("FEATHERPLAY_HOME").as_deref(),
    )
}

/// Set by `mark_registered` once this run's `register` succeeded.
static REGISTERED: AtomicBool = AtomicBool::new(false);

/// Records that `register` succeeded in this run, so toasts can use `AUMID` from now on
/// (`registered`). The caller sets it after `register` returns `Ok`, changed or not.
pub fn mark_registered() {
    REGISTERED.store(true, Ordering::Release);
}

/// Whether this run has the registration in place (`mark_registered`). False in dev runs
/// and tests, after a failed registration, and while the startup registration is still
/// running; one atomic load, so cheap enough for every toast.
pub fn registered() -> bool {
    REGISTERED.load(Ordering::Acquire)
}

/// Where the registration lives for one user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Registration {
    pub exe: PathBuf,
    /// `{Programs}/featherplay.lnk`.
    pub shortcut: PathBuf,
    /// `{data dir}/featherplay.ico`.
    pub icon_file: PathBuf,
}

impl Registration {
    /// The layout under a Programs folder and a data dir, without asking the system.
    pub fn new(exe: PathBuf, programs: &Path, data_dir: &Path) -> Registration {
        Registration {
            exe,
            shortcut: programs.join(SHORTCUT_FILE),
            icon_file: data_dir.join(ICON_FILE),
        }
    }
}

/// What `register` changed; all false when everything was already in place.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub icon: bool,
    pub registry: bool,
    pub shortcut: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aumid_follows_microsofts_form() {
        assert_eq!(AUMID, "featherplay.featherplay");
        assert!(AUMID.len() <= 128);
        assert!(!AUMID.contains(char::is_whitespace));
        assert!(AUMID.split('.').count() >= 2);
        assert!(AUMID.split('.').all(|part| !part.is_empty()));
        let lower = AUMID.to_lowercase();
        assert!(!lower.contains("youtube") && !lower.contains("ytm"));
        assert_eq!(DISPLAY_NAME, "featherplay");
    }

    #[test]
    fn guard_skips_dev_and_debug() {
        let dev = Some(OsStr::new(r"D:\Workspace\Cache\featherplay-dev"));
        assert!(registration_allowed(true, None));
        assert!(registration_allowed(true, Some(OsStr::new(""))));
        assert!(!registration_allowed(true, dev));
        assert!(!registration_allowed(false, None));
        assert!(!registration_allowed(false, dev));
    }

    #[test]
    fn tests_never_register() {
        // `cargo test` builds with debug assertions; dev runs set FEATHERPLAY_HOME.
        assert!(!registration_allowed_here());
    }
}
