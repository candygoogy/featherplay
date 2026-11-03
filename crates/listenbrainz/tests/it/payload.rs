use featherplay_listenbrainz::{Listen, ListenType, payload};
use serde_json::json;

fn listen(i: i64) -> Listen {
    Listen {
        artist: format!("Artist {i}"),
        track: format!("Track {i}"),
        album: Some("Album".into()),
        duration_s: Some(200),
        video_id: Some(format!("vid{i}")),
        listened_at: 1_800_000_000 + i,
    }
}

fn info(id: &str) -> serde_json::Value {
    json!({
        "media_player": "featherplay",
        "submission_client": "featherplay",
        "submission_client_version": env!("CARGO_PKG_VERSION"),
        "music_service": "music.youtube.com",
        "origin_url": format!("https://music.youtube.com/watch?v={id}"),
        "duration_ms": 200_000,
    })
}

#[test]
fn playing_now_has_no_timestamp() {
    assert_eq!(
        payload(ListenType::PlayingNow, &[listen(1)]),
        json!({
            "listen_type": "playing_now",
            "payload": [{
                "track_metadata": {
                    "artist_name": "Artist 1",
                    "track_name": "Track 1",
                    "release_name": "Album",
                    "additional_info": info("vid1"),
                }
            }]
        })
    );
}

#[test]
fn single_and_import_carry_listened_at() {
    assert_eq!(
        payload(ListenType::Single, &[listen(1)]),
        json!({
            "listen_type": "single",
            "payload": [{
                "listened_at": 1_800_000_001,
                "track_metadata": {
                    "artist_name": "Artist 1",
                    "track_name": "Track 1",
                    "release_name": "Album",
                    "additional_info": info("vid1"),
                }
            }]
        })
    );
    let import = payload(ListenType::Import, &[listen(1), listen(2)]);
    assert_eq!(import["listen_type"], "import");
    assert_eq!(import["payload"][1]["listened_at"], 1_800_000_002);
    assert_eq!(
        import["payload"][1]["track_metadata"]["track_name"],
        "Track 2"
    );
}

#[test]
fn missing_fields_are_left_out() {
    let bare = Listen {
        album: None,
        duration_s: None,
        video_id: None,
        ..listen(1)
    };
    let p = payload(ListenType::Single, &[bare]);
    let meta = &p["payload"][0]["track_metadata"];
    assert!(meta.get("release_name").is_none());
    let info = meta["additional_info"].as_object().unwrap();
    assert!(!info.contains_key("origin_url") && !info.contains_key("duration_ms"));
    assert_eq!(info["media_player"], "featherplay");
}
