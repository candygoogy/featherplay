//! Scrobbles that failed to send, kept on disk and retried later.

use std::fs;
use std::io::{self, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::client::MAX_BATCH;

/// Last.fm rejects scrobbles with timestamps older than this.
const MAX_AGE_S: i64 = 14 * 86_400;

/// One scrobble waiting to be sent. `timestamp` is the UTC Unix time the track started.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PendingScrobble {
    pub artist: String,
    pub track: String,
    pub album: Option<String>,
    pub timestamp: i64,
    pub duration_s: Option<u32>,
    /// The track's video id (ListenBrainz's `origin_url`). Queues saved before it existed
    /// load without it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_id: Option<String>,
}

/// An offline scrobble queue, stored as JSON: Last.fm's at `{config}/scrobble-queue.json`,
/// ListenBrainz's at `{config}/listenbrainz-queue.json`. Both follow Last.fm's limits.
#[derive(Debug, Default)]
pub struct OfflineQueue {
    items: Vec<PendingScrobble>,
}

impl OfflineQueue {
    /// Loads the queue from `path`. A missing or unreadable file gives an empty queue.
    pub fn load(path: &Path) -> Self {
        let items = fs::read(path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self { items }
    }

    pub fn push(&mut self, s: PendingScrobble) {
        self.items.push(s);
    }

    /// Drops scrobbles older than 14 days at `now` (Unix seconds), then removes and
    /// returns the oldest-queued ones, at most 50.
    pub fn take_batch(&mut self, now: i64) -> Vec<PendingScrobble> {
        self.items.retain(|s| now - s.timestamp <= MAX_AGE_S);
        let n = self.items.len().min(MAX_BATCH);
        self.items.drain(..n).collect()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    /// Writes the queue to `path` atomically: a temp file next to it, then a rename.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_vec(&self.items).map_err(io::Error::other)?;
        let mut tmp_name = path.as_os_str().to_owned();
        tmp_name.push(".tmp");
        let tmp = Path::new(&tmp_name);
        {
            let mut file = fs::File::create(tmp)?;
            file.write_all(&json)?;
            file.sync_all()?;
        }
        fs::rename(tmp, path).inspect_err(|_| {
            let _ = fs::remove_file(tmp);
        })
    }
}
