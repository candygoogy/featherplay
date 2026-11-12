use std::path::PathBuf;

use featherplay_lastfm::{OfflineQueue, PendingScrobble};

const DAY: i64 = 86_400;

/// A folder of this test's own, under `FEATHERPLAY_HOME`, else the temp folder.
fn test_dir() -> PathBuf {
    let base = std::env::var("FEATHERPLAY_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir().join("featherplay-tests"));
    base.join(format!("test-lastfm-offline-{}", std::process::id()))
}

fn pending(i: i64, timestamp: i64) -> PendingScrobble {
    PendingScrobble {
        artist: format!("Artist {i}"),
        track: format!("Track {i}"),
        album: None,
        timestamp,
        duration_s: Some(200),
        video_id: None,
    }
}

#[test]
fn offline_drops_old_and_batches_50() {
    let now = 1_800_000_000;
    let mut queue = OfflineQueue::default();
    for i in 0..120 {
        // Every 12th item is 15 days old: 10 in total.
        let age = if i % 12 == 0 { 15 * DAY } else { DAY };
        queue.push(pending(i, now - age));
    }

    // Round-trip through disk so the batches come from a loaded queue.
    let dir = test_dir();
    let path = dir.join("scrobble-queue.json");
    queue.save(&path).unwrap();
    let mut queue = OfflineQueue::load(&path);
    std::fs::remove_dir_all(&dir).unwrap();
    assert_eq!(queue.len(), 120);

    let first = queue.take_batch(now);
    assert_eq!(first.len(), 50);

    let mut returned = first;
    loop {
        let batch = queue.take_batch(now);
        if batch.is_empty() {
            break;
        }
        assert!(batch.len() <= 50);
        returned.extend(batch);
    }
    assert_eq!(returned.len(), 110);
    assert!(returned.iter().all(|s| now - s.timestamp < 14 * DAY));
    assert!(queue.is_empty());
}
