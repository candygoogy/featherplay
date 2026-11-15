//! What the unit tests share.

use std::path::PathBuf;

use featherplay_core::model::{LikeStatus, Track, TrackId, VideoType};

/// A song with id `id`, titled the same and nothing else known. A test sets the fields it
/// reads: `Track { title, ..track(id) }`.
pub fn track(id: &str) -> Track {
    Track {
        id: TrackId(id.into()),
        title: id.into(),
        artists: Vec::new(),
        album: None,
        duration: None,
        video_type: VideoType::Song,
        thumbnail: None,
        like: LikeStatus::Indifferent,
        counterpart: None,
    }
}

/// A new, empty folder for one test, `test-<name>-<process>` under `FEATHERPLAY_HOME`.
/// `None` when that isn't set, which this says: the test then skips, as it writes nowhere
/// else.
#[allow(clippy::print_stderr, reason = "a test says why it skips")]
pub fn scratch(name: &str) -> Option<PathBuf> {
    let Some(home) = std::env::var_os("FEATHERPLAY_HOME").filter(|v| !v.is_empty()) else {
        eprintln!("skipped: set FEATHERPLAY_HOME to a test home to run this test");
        return None;
    };
    let dir = PathBuf::from(home).join(format!("test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a scratch folder");
    Some(dir)
}
