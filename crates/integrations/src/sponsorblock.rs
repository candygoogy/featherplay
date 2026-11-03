//! SponsorBlock's `music_offtopic` segments: the non-music parts of music videos (intros,
//! skits, outros). Lookups go by a 4-character hash prefix, so the server never learns
//! which video is playing.

use std::sync::OnceLock;
use std::time::Duration;

use featherplay_core::model::{TrackId, VideoType};
use serde::Deserialize;
use sha2::{Digest, Sha256};

const API: &str = "https://sponsor.ajay.app/api/skipSegments";
/// `["music_offtopic"]`, URL-encoded.
const CATEGORIES: &str = "%5B%22music_offtopic%22%5D";
/// A segment is skipped only while more than this much of it is left.
const SKIP_MARGIN: Duration = Duration::from_millis(500);

/// A part of the track to skip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Segment {
    pub start: Duration,
    pub end: Duration,
}

/// The first 4 hex characters of the SHA-256 of the videoId.
pub fn hash_prefix(id: &TrackId) -> String {
    Sha256::digest(id.0.as_bytes())[..2]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Only music videos and user uploads have non-music parts; songs (ATV) are the audio.
pub fn applies(t: VideoType) -> bool {
    matches!(t, VideoType::MusicVideo | VideoType::UserVideo)
}

/// Where to seek to: the end of the segment `position` is in, unless less than 0.5 s of
/// it is left.
pub fn skip_target(segments: &[Segment], position: Duration) -> Option<Duration> {
    segments
        .iter()
        .find(|s| position >= s.start && position + SKIP_MARGIN < s.end)
        .map(|s| s.end)
}

#[derive(Deserialize)]
struct Video {
    #[serde(rename = "videoID")]
    video_id: String,
    segments: Vec<RawSegment>,
}

#[derive(Deserialize)]
struct RawSegment {
    segment: [f64; 2],
    #[serde(rename = "actionType", default)]
    action_type: Option<String>,
}

/// The `music_offtopic` segments of `id`, sorted by start. A video nobody submitted
/// segments for (HTTP 404) gives an empty list.
pub async fn fetch_segments(id: &TrackId) -> Result<Vec<Segment>, String> {
    static CLIENT: OnceLock<Result<reqwest::Client, String>> = OnceLock::new();
    let client = CLIENT
        .get_or_init(|| {
            reqwest::Client::builder()
                .user_agent(concat!("featherplay/", env!("CARGO_PKG_VERSION")))
                .timeout(Duration::from_secs(10))
                .build()
                .map_err(|e| format!("client: {e}"))
        })
        .as_ref()
        .map_err(Clone::clone)?;
    let url = format!("{API}/{}?categories={CATEGORIES}", hash_prefix(id));
    let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    if status == reqwest::StatusCode::NOT_FOUND {
        return Ok(Vec::new());
    }
    if !status.is_success() {
        return Err(format!("HTTP {}", status.as_u16()));
    }
    let body = resp.bytes().await.map_err(|e| e.to_string())?;
    parse(&body, id)
}

fn parse(body: &[u8], id: &TrackId) -> Result<Vec<Segment>, String> {
    let videos: Vec<Video> =
        serde_json::from_slice(body).map_err(|e| format!("bad response: {e}"))?;
    let mut out: Vec<Segment> = videos
        .into_iter()
        .filter(|v| v.video_id == id.0)
        .flat_map(|v| v.segments)
        .filter(|s| s.action_type.as_deref().is_none_or(|a| a == "skip"))
        .filter_map(|s| {
            let [start, end] = s.segment;
            (start.is_finite() && end.is_finite() && start >= 0.0 && end > start).then(|| Segment {
                start: Duration::from_secs_f64(start),
                end: Duration::from_secs_f64(end),
            })
        })
        .collect();
    out.sort_by_key(|s| s.start);
    Ok(out)
}
