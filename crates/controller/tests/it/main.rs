//! Every integration test of `controller` in one test program, one module per area.
//! Run them with `cargo nextest run` (one process per test); see `.config/nextest.toml`.

mod diagnostics;
mod playing_from;
mod song_menu_queue;
mod updates;

/// A fresh, empty folder for a test, `test-{name}-{process id}` under `FEATHERPLAY_HOME`.
/// `None` when `FEATHERPLAY_HOME` isn't set: the test skips, saying so, and writes nothing
/// anywhere else.
#[allow(clippy::print_stderr, reason = "a test says why it skips")]
fn test_home(name: &str) -> Option<std::path::PathBuf> {
    let Some(base) = std::env::var_os("FEATHERPLAY_HOME") else {
        eprintln!("skipped: FEATHERPLAY_HOME isn't set");
        return None;
    };
    let dir = std::path::PathBuf::from(base).join(format!("test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("the test's folder");
    Some(dir)
}
