//! Test homes: fresh folders under `FEATHERPLAY_HOME`, so no test writes to C: or to a real
//! profile. With the variable unset, the tests that need a home say so and return early.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use featherplay_store::Paths;

/// `FEATHERPLAY_HOME` as it was when first asked for. `paths_honour_featherplay_home`
/// reads it through here before it changes the variable, so a test running alongside
/// never sees the temporary value.
pub fn base() -> Option<&'static Path> {
    static BASE: OnceLock<Option<PathBuf>> = OnceLock::new();
    BASE.get_or_init(|| {
        std::env::var_os("FEATHERPLAY_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    })
    .as_deref()
}

/// The note a test prints when it skips for want of `FEATHERPLAY_HOME`.
pub fn skip_note() {
    eprintln!(
        "skipped: FEATHERPLAY_HOME is not set; point it at a scratch folder to run this test"
    );
}

/// A fresh home directory that is removed on drop, even if the test panics.
pub struct TestHome(pub PathBuf);

impl TestHome {
    /// `None`, after printing why, when `FEATHERPLAY_HOME` is unset.
    pub fn new(name: &str) -> Option<Self> {
        let Some(base) = base() else {
            skip_note();
            return None;
        };
        let dir = base.join(format!("test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        Some(Self(dir))
    }

    pub fn paths(&self) -> Paths {
        Paths::from_home(&self.0)
    }
}

impl Drop for TestHome {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn test_homes_live_under_featherplay_home() {
    let Some(home) = TestHome::new("where") else {
        return;
    };
    let base = base().expect("set, or new() would have skipped");
    assert!(
        home.0.starts_with(base),
        "{} is not under {}",
        home.0.display(),
        base.display()
    );
}
