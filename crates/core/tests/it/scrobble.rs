use std::time::{Duration, SystemTime};

use featherplay_core::model::{LikeStatus, Track, TrackId, VideoType};
use featherplay_core::scrobble::{ScrobbleTracker, clean_title, eligible, threshold};

fn track(secs: u64) -> Track {
    Track {
        id: TrackId("abc".to_string()),
        title: "Song".to_string(),
        artists: Vec::new(),
        album: None,
        duration: Some(Duration::from_secs(secs)),
        video_type: VideoType::Song,
        thumbnail: None,
        like: LikeStatus::Indifferent,
        song_version: None,
    }
}

#[test]
fn threshold_values() {
    assert_eq!(threshold(Duration::from_secs(30)), None);
    assert_eq!(
        threshold(Duration::from_secs(31)),
        Some(Duration::from_millis(15_500))
    );
    assert_eq!(
        threshold(Duration::from_secs(200)),
        Some(Duration::from_secs(100))
    );
    assert_eq!(
        threshold(Duration::from_secs(600)),
        Some(Duration::from_secs(240))
    );
}

#[test]
fn clean_title_strips_only_videos() {
    assert_eq!(
        clean_title("Song (Official Video)", VideoType::MusicVideo),
        "Song"
    );
    assert_eq!(
        clean_title("Song [Official Video]", VideoType::UserVideo),
        "Song"
    );
    assert_eq!(
        clean_title("Song (Official Video)", VideoType::Song),
        "Song (Official Video)"
    );
    assert_eq!(clean_title("Song (LYRICS)", VideoType::MusicVideo), "Song");
}

#[test]
fn eligible_rules() {
    assert!(!eligible(VideoType::UserVideo, false));
    assert!(eligible(VideoType::MusicVideo, false));
    assert!(eligible(VideoType::Episode, true));
}

#[test]
fn tracker_fires_once_on_played_time() {
    // A 300 s track (threshold 150 s): two 60 s deltas stay under it and the 41 s one
    // crosses it. A 200 s track's 100 s threshold would already fall on the second.
    let t = track(300);
    let started = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000);
    let mut tracker = ScrobbleTracker::default();
    tracker.start(&t, started);
    assert_eq!(tracker.played(Duration::from_secs(60)), None);
    assert_eq!(tracker.played(Duration::from_secs(60)), None);
    assert_eq!(
        tracker.played(Duration::from_secs(41)),
        Some((t.clone(), started))
    );
    assert_eq!(tracker.played(Duration::from_secs(100)), None);
}

#[test]
fn seek_does_not_count() {
    let mut tracker = ScrobbleTracker::default();
    tracker.start(&track(200), SystemTime::UNIX_EPOCH);
    // The position jumped 150 s, but only 1 s of wall-clock play time passed.
    assert_eq!(tracker.played(Duration::from_secs(1)), None);
}

mod alternates {
    use featherplay_core::model::{AlbumId, AlbumRef, ArtistRef};
    use featherplay_core::scrobble::{Alternates, MetaOptions, scrobble_fields};

    use super::*;

    fn song() -> Track {
        Track {
            title: "Yoru ni Kakeru".into(),
            artists: vec![ArtistRef {
                id: None,
                name: "YOASOBI".into(),
            }],
            album: Some(AlbumRef {
                id: AlbumId("MPREb_x".into()),
                title: "THE BOOK".into(),
            }),
            ..track(261)
        }
    }

    fn opts(titles: bool, artists: bool) -> MetaOptions {
        MetaOptions {
            alternative_titles: titles,
            alternative_artists: artists,
        }
    }

    #[test]
    fn alternates_replace_title_and_artist_only_when_on() {
        let alt = Alternates::new(Some("夜に駆ける"), Some("ヨアソビ"));
        let off = scrobble_fields(&song(), &alt, opts(false, false)).unwrap();
        assert_eq!(
            (off.track.as_str(), off.artist.as_str()),
            ("Yoru ni Kakeru", "YOASOBI")
        );
        assert_eq!(off.album.as_deref(), Some("THE BOOK"));
        assert_eq!(off.duration_s, Some(261));

        let titles = scrobble_fields(&song(), &alt, opts(true, false)).unwrap();
        assert_eq!(
            (titles.track.as_str(), titles.artist.as_str()),
            ("夜に駆ける", "YOASOBI")
        );

        let artists = scrobble_fields(&song(), &alt, opts(false, true)).unwrap();
        assert_eq!(
            (artists.track.as_str(), artists.artist.as_str()),
            ("Yoru ni Kakeru", "ヨアソビ")
        );

        let both = scrobble_fields(&song(), &alt, opts(true, true)).unwrap();
        assert_eq!(
            (both.track.as_str(), both.artist.as_str()),
            ("夜に駆ける", "ヨアソビ")
        );
    }

    #[test]
    fn missing_or_blank_alternates_fall_back() {
        let blank = Alternates::new(Some("  "), Some(""));
        assert_eq!(blank, Alternates::default());
        let f = scrobble_fields(&song(), &blank, opts(true, true)).unwrap();
        assert_eq!(
            (f.track.as_str(), f.artist.as_str()),
            ("Yoru ni Kakeru", "YOASOBI")
        );

        // No YouTube Music artist: the tag still gives one, but only when it's used.
        let no_artist = Track {
            artists: Vec::new(),
            ..song()
        };
        let alt = Alternates::new(None, Some("YOASOBI"));
        assert!(scrobble_fields(&no_artist, &alt, opts(true, false)).is_none());
        assert_eq!(
            scrobble_fields(&no_artist, &alt, opts(false, true))
                .unwrap()
                .artist,
            "YOASOBI"
        );
    }

    #[test]
    fn video_titles_are_cleaned_whichever_is_used() {
        let mv = Track {
            title: "Get Lucky".into(),
            video_type: VideoType::MusicVideo,
            ..song()
        };
        let alt = Alternates::new(Some("Get Lucky (Official Video) "), None);
        let f = scrobble_fields(&mv, &alt, opts(true, true)).unwrap();
        assert_eq!(f.track, "Get Lucky");
        assert_eq!(f.artist, "YOASOBI");
    }
}
