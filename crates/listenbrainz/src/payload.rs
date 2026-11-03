//! The JSON body of `POST /1/submit-listens`.

use serde_json::{Map, Value, json};

/// Sent as `media_player` and `submission_client`.
const CLIENT: &str = "featherplay";
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// One listen. `listened_at` is the UTC Unix time the track started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listen {
    pub artist: String,
    pub track: String,
    pub album: Option<String>,
    pub duration_s: Option<u32>,
    /// The video id, for `origin_url`.
    pub video_id: Option<String>,
    pub listened_at: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListenType {
    /// What's playing now: exactly one listen, without `listened_at`.
    PlayingNow,
    /// One finished listen.
    Single,
    /// Several finished listens (the offline queue).
    Import,
}

impl ListenType {
    fn name(self) -> &'static str {
        match self {
            ListenType::PlayingNow => "playing_now",
            ListenType::Single => "single",
            ListenType::Import => "import",
        }
    }
}

/// `{"listen_type": …, "payload": [{"listened_at": …, "track_metadata": {…}}, …]}`.
pub fn payload(kind: ListenType, listens: &[Listen]) -> Value {
    let items: Vec<Value> = listens
        .iter()
        .map(|l| {
            let mut item = Map::new();
            if kind != ListenType::PlayingNow {
                item.insert("listened_at".into(), l.listened_at.into());
            }
            item.insert("track_metadata".into(), track_metadata(l));
            Value::Object(item)
        })
        .collect();
    json!({ "listen_type": kind.name(), "payload": items })
}

fn track_metadata(l: &Listen) -> Value {
    let mut info = Map::new();
    info.insert("media_player".into(), CLIENT.into());
    info.insert("submission_client".into(), CLIENT.into());
    info.insert("submission_client_version".into(), VERSION.into());
    info.insert("music_service".into(), "music.youtube.com".into());
    if let Some(id) = &l.video_id {
        info.insert(
            "origin_url".into(),
            format!("https://music.youtube.com/watch?v={id}").into(),
        );
    }
    if let Some(d) = l.duration_s {
        info.insert("duration_ms".into(), (u64::from(d) * 1000).into());
    }
    let mut meta = Map::new();
    meta.insert("artist_name".into(), l.artist.as_str().into());
    meta.insert("track_name".into(), l.track.as_str().into());
    if let Some(album) = &l.album {
        meta.insert("release_name".into(), album.as_str().into());
    }
    meta.insert("additional_info".into(), Value::Object(info));
    Value::Object(meta)
}
