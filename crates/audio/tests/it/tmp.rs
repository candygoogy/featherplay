//! Startup cleanup of `{cache}/tmp`: only the engine's own leftovers go.

use std::fs;
use std::path::{Path, PathBuf};

use featherplay_audio::clear_stale_temp;

/// A fresh folder under `FEATHERPLAY_HOME` (so never on C: or in a real profile), removed
/// on drop.
struct TestDir(PathBuf);

impl TestDir {
    /// `None`, after printing why, when `FEATHERPLAY_HOME` is unset: the test returns early.
    fn new(name: &str) -> Option<TestDir> {
        let Some(base) = std::env::var_os("FEATHERPLAY_HOME").filter(|v| !v.is_empty()) else {
            eprintln!(
                "skipped: FEATHERPLAY_HOME is not set; point it at a scratch folder to run this test"
            );
            return None;
        };
        let dir = Path::new(&base).join(format!("test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Some(TestDir(dir))
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn removes_only_our_unused_temp_files() {
    let Some(dir) = TestDir::new("tmp-cleanup") else {
        return;
    };
    let d = &dir.0;
    fs::write(d.join("featherplay-audio-a1B2c3"), b"stale audio").unwrap();
    fs::write(d.join("featherplay-audio-x9"), b"").unwrap();
    // Someone else's files, and a folder that happens to share the prefix.
    fs::write(d.join("notes.txt"), b"keep").unwrap();
    fs::write(d.join("other-featherplay-audio-1"), b"keep").unwrap();
    fs::create_dir(d.join("featherplay-audio-dir")).unwrap();
    // A temp file still open, as another running instance would hold it.
    let open_path = d.join("featherplay-audio-open");
    let held = fs::File::create(&open_path).unwrap();

    assert_eq!(clear_stale_temp(d), 2);

    let mut left: Vec<String> = fs::read_dir(d)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    left.sort();
    assert_eq!(
        left,
        [
            "featherplay-audio-dir",
            "featherplay-audio-open",
            "notes.txt",
            "other-featherplay-audio-1"
        ]
    );
    drop(held);
    assert_eq!(clear_stale_temp(d), 1, "closed, it is stale too");
}

#[test]
fn a_missing_folder_is_fine() {
    let Some(dir) = TestDir::new("tmp-missing") else {
        return;
    };
    assert_eq!(clear_stale_temp(&dir.0.join("nope")), 0);
}

#[test]
fn test_dirs_live_under_featherplay_home() {
    let Some(dir) = TestDir::new("where") else {
        return;
    };
    let base = std::env::var_os("FEATHERPLAY_HOME").expect("set, or new() would have skipped");
    assert!(
        dir.0.starts_with(&base),
        "{} is not under {base:?}",
        dir.0.display()
    );
}
