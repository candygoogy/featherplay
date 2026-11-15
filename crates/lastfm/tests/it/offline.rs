use std::path::PathBuf;

use featherplay_lastfm::{OfflineQueue, PendingScrobble};

const DAY: i64 = 86_400;

/// A folder of this test's own under `FEATHERPLAY_HOME` (so never on C: or in a real
/// profile). `None`, after printing why, when the variable is unset: the test returns early.
fn test_dir() -> Option<PathBuf> {
    let Some(base) = std::env::var_os("FEATHERPLAY_HOME").filter(|v| !v.is_empty()) else {
        eprintln!(
            "skipped: FEATHERPLAY_HOME is not set; point it at a scratch folder to run this test"
        );
        return None;
    };
    Some(PathBuf::from(base).join(format!("test-lastfm-offline-{}", std::process::id())))
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
    let Some(dir) = test_dir() else {
        return;
    };
    let now = 1_800_000_000;
    let mut queue = OfflineQueue::default();
    for i in 0..120 {
        // Every 12th item is 15 days old: 10 in total.
        let age = if i % 12 == 0 { 15 * DAY } else { DAY };
        queue.push(pending(i, now - age));
    }

    // Round-trip through disk so the batches come from a loaded queue.
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
