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
            featherplay_proxy::client()
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
            // A time past what a `Duration` holds is dropped like the other bad ones, and so
            // is a segment shorter than the nanosecond a `Duration` counts in.
            let segment = Segment {
                start: Duration::try_from_secs_f64(start).ok()?,
                end: Duration::try_from_secs_f64(end).ok()?,
            };
            (segment.start < segment.end).then_some(segment)
        })
        .collect();
    out.sort_by_key(|s| s.start);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use proptest::num;
    use proptest::prelude::*;

    use super::*;

    fn id() -> TrackId {
        TrackId("dQw4w9WgXcQ".into())
    }

    fn secs(start: f64, end: f64) -> Segment {
        Segment {
            start: Duration::from_secs_f64(start),
            end: Duration::from_secs_f64(end),
        }
    }

    #[test]
    fn a_reply_gives_this_videos_skip_segments_sorted() {
        // The prefix lookup also returns other videos; a mute segment isn't skipped; a
        // backwards, negative or empty one is dropped, and so is one whose time is past
        // what a `Duration` holds or that is shorter than its nanosecond.
        let body = br#"[
            {"videoID": "other", "segments": [{"segment": [1, 2], "actionType": "skip"}]},
            {"videoID": "dQw4w9WgXcQ", "segments": [
                {"segment": [30.5, 40], "actionType": "skip", "UUID": "a"},
                {"segment": [0, 12.25]},
                {"segment": [50, 60], "actionType": "mute"},
                {"segment": [70, 65], "actionType": "skip"},
                {"segment": [-1, 5], "actionType": "skip"},
                {"segment": [80, 80], "actionType": "skip"},
                {"segment": [90, 1e20], "actionType": "skip"},
                {"segment": [0, 4.8e-22], "actionType": "skip"}
            ]}
        ]"#;
        assert_eq!(
            parse(body, &id()),
            Ok(vec![secs(0.0, 12.25), secs(30.5, 40.0)])
        );
    }

    #[test]
    fn an_empty_reply_gives_no_segments() {
        assert_eq!(parse(b"[]", &id()), Ok(Vec::new()));
        let none_for_it = br#"[{"videoID": "other", "segments": [{"segment": [1, 2]}]}]"#;
        assert_eq!(parse(none_for_it, &id()), Ok(Vec::new()));
    }

    #[test]
    fn a_malformed_reply_is_an_error() {
        for (case, body) in [
            ("not json", &b"<html>busy</html>"[..]),
            ("not a list", br#"{"videoID": "dQw4w9WgXcQ"}"#),
            (
                "a segment without its end",
                br#"[{"videoID": "dQw4w9WgXcQ", "segments": [{"segment": [1]}]}]"#,
            ),
        ] {
            let got = parse(body, &id());
            assert!(
                got.as_ref().is_err_and(|e| e.starts_with("bad response")),
                "{case}: {got:?}"
            );
        }
    }

    /// A segment time: mostly a plausible one, else any finite number (huge, tiny,
    /// negative), which the server could send just as well.
    fn time() -> impl Strategy<Value = f64> {
        prop_oneof![
            3 => 0.0..900.0f64,
            1 => num::f64::POSITIVE
                | num::f64::NEGATIVE
                | num::f64::NORMAL
                | num::f64::SUBNORMAL
                | num::f64::ZERO,
        ]
    }

    /// A reply in the API's shape: videos sharing the hash prefix, this one among them or
    /// not, with segments of any action.
    fn reply() -> impl Strategy<Value = Vec<serde_json::Value>> {
        let action = prop::option::of(prop::sample::select(vec!["skip", "mute", "poi"]));
        let segment = (time(), time(), action).prop_map(|(start, end, action)| {
            let mut s = serde_json::json!({ "segment": [start, end] });
            if let Some(a) = action {
                s["actionType"] = a.into();
            }
            s
        });
        let video = (
            prop::sample::select(vec!["dQw4w9WgXcQ", "other"]),
            prop::collection::vec(segment, 0..8),
        )
            .prop_map(|(id, segments)| serde_json::json!({ "videoID": id, "segments": segments }));
        prop::collection::vec(video, 0..4)
    }

    proptest! {
        /// Any reply in the API's shape is read, and gives segments sorted by start, each
        /// ending after it starts.
        #[test]
        fn a_reply_gives_sorted_segments_that_end_after_they_start(videos in reply()) {
            let body = serde_json::to_vec(&videos).unwrap();
            let segments = parse(&body, &id()).unwrap();
            prop_assert!(segments.is_sorted_by_key(|s| s.start), "{segments:?}");
            for s in &segments {
                prop_assert!(s.start < s.end, "{s:?}");
            }
        }

        /// Any bytes give segments or an error, never a panic.
        #[test]
        fn any_bytes_give_segments_or_an_error(body in prop::collection::vec(any::<u8>(), 0..256)) {
            let _ = parse(&body, &id());
        }
    }
}
