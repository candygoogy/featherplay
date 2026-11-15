use std::time::Duration;

use featherplay_core::model::{TrackId, VideoType};
use featherplay_integrations::sponsorblock::{Segment, applies, hash_prefix, skip_target};

#[test]
fn hash_prefix_known() {
    // The first 4 hex characters of SHA-256("dQw4w9WgXcQ").
    assert_eq!(hash_prefix(&TrackId("dQw4w9WgXcQ".into())), "5f6b");
}

#[test]
fn skip_inside_segment() {
    let segments = [Segment {
        start: Duration::from_secs(10),
        end: Duration::from_secs(20),
    }];
    assert_eq!(
        skip_target(&segments, Duration::from_secs(12)),
        Some(Duration::from_secs(20))
    );
    assert_eq!(skip_target(&segments, Duration::from_millis(19_600)), None);
    assert_eq!(skip_target(&segments, Duration::from_secs(25)), None);
}

#[test]
fn applies_only_to_videos() {
    assert!(applies(VideoType::MusicVideo));
    assert!(applies(VideoType::UserVideo));
    assert!(!applies(VideoType::Song));
    assert!(!applies(VideoType::Episode));
    assert!(!applies(VideoType::Other));
}
