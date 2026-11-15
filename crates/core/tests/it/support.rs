//! Builders and readers the player and queue tests share, so a new `Track` or
//! `PlaybackPrefs` field is one edit here.

use std::time::{Duration, SystemTime};

use featherplay_core::model::{LikeStatus, Track, TrackId, VideoType};
use featherplay_core::player::{Effect, Input, PlaybackPrefs, Player, StreamInfo};

pub fn id(s: &str) -> TrackId {
    TrackId(s.to_string())
}

/// Song `s`: 200 s long, titled "Title {s}", on no album, not rated.
pub fn track(s: &str) -> Track {
    Track {
        id: id(s),
        title: format!("Title {s}"),
        artists: Vec::new(),
        album: None,
        duration: Some(Duration::from_secs(200)),
        video_type: VideoType::Song,
        thumbnail: None,
        like: LikeStatus::Indifferent,
        counterpart: None,
    }
}

pub fn tracks(ids: &[&str]) -> Vec<Track> {
    ids.iter().map(|s| track(s)).collect()
}

/// The settings the tests start from: song versions preferred, non-music left out of
/// radios, other media scrobbled, disliked songs passed over, a list stops at its end, no
/// crossfade and normal speed.
pub fn prefs() -> PlaybackPrefs {
    PlaybackPrefs {
        prefer_song_versions: true,
        skip_non_music_in_radio: true,
        scrobble_other_media: true,
        list_end: Default::default(),
        skip_disliked: true,
        crossfade: None,
        fade_on_skip: false,
        speed_percent: 100,
    }
}

/// The clock the tests' ticks and stream expiries count from.
pub fn t0() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000)
}

/// `s` resolved to a 200 s stream that stays good for 6 hours from `t0`.
pub fn ok(s: &str) -> Input {
    Input::Resolved {
        id: id(s),
        result: Ok(StreamInfo {
            expires_at: t0() + Duration::from_secs(6 * 3600),
            duration: Some(Duration::from_secs(200)),
        }),
    }
}

/// The effects that drive playback; `StateChanged` only tells the UI to refresh.
pub fn run(p: &mut Player, input: Input) -> Vec<Effect> {
    p.handle(input)
        .into_iter()
        .filter(|e| *e != Effect::StateChanged)
        .collect()
}

/// The ids `fx` resolves, in order.
pub fn resolves(fx: &[Effect]) -> Vec<String> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Resolve(id) => Some(id.0.clone()),
            _ => None,
        })
        .collect()
}

pub fn current(p: &Player) -> Option<String> {
    p.queue().current().map(|t| t.id.0.clone())
}

pub fn upcoming(p: &Player) -> Vec<String> {
    p.queue()
        .upcoming()
        .iter()
        .map(|t| t.id.0.clone())
        .collect()
}
