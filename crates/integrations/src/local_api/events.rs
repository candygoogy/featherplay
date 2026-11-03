//! What the API says about the player, in Pear Desktop's shapes: the song (`/song`), the
//! WebSocket's first message (`PLAYER_INFO`) and the events after it, which go out only for
//! changes a client can't work out itself.
//!
//! A position always comes as whole seconds with `timestamp`: the Unix time, in
//! milliseconds, when the track was at exactly that second. While `isPlaying`, a client moves
//! it on by itself (`position + (now - timestamp) / 1000`); the server sends nothing per
//! second.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use featherplay_core::model::{LikeStatus, RepeatMode, Track, TrackId, VideoType};
use serde_json::{Map, Value, json};

use super::Snapshot;

/// A position further than this from where the last one would have moved on to is a jump
/// (a seek, a restart, a stall) and is sent.
const JUMP: Duration = Duration::from_secs(1);

/// What changed between two snapshots, as a client sees it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Changes {
    track: bool,
    playing: bool,
    /// Moved other than with the clock: a client's own count is off.
    position: bool,
    volume: bool,
    repeat: bool,
    shuffle: bool,
}

/// The current track's ID.
fn track_id(s: &Snapshot) -> Option<&TrackId> {
    s.track.as_ref().map(|t| &t.id)
}

impl Changes {
    fn between(prev: &Snapshot, next: &Snapshot) -> Changes {
        let expected = position_at(prev, next.read_at);
        let gap = next.position.abs_diff(expected);
        Changes {
            track: track_id(prev) != track_id(next),
            playing: prev.playing != next.playing,
            // The sound starting or stopping restarts a client's count from a new time.
            position: prev.audible != next.audible || gap > JUMP,
            volume: prev.volume != next.volume,
            repeat: prev.repeat != next.repeat,
            shuffle: prev.shuffle != next.shuffle,
        }
    }

    fn any(self) -> bool {
        self != Changes::default()
    }
}

/// Whether going from `prev` to `next` gives a client something to hear.
pub(super) fn worth_sending(prev: &Snapshot, next: &Snapshot) -> bool {
    Changes::between(prev, next).any()
}

/// The messages that bring a client that saw `prev` up to `next`, at `now`. A new track
/// carries the position, and so does a play or pause; a jump alone is `POSITION_CHANGED`.
pub(super) fn events(prev: &Snapshot, next: &Snapshot, now: SystemTime) -> Vec<Value> {
    let c = Changes::between(prev, next);
    let (position, timestamp) = clock(next, now);
    let mut out = Vec::new();
    if c.track {
        let mut ev = json!({
            "type": "VIDEO_CHANGED",
            "position": position,
            "timestamp": timestamp,
        });
        if let Some(t) = &next.track {
            ev["song"] = song(t, next, now);
        }
        out.push(ev);
    }
    if c.playing {
        out.push(json!({
            "type": "PLAYER_STATE_CHANGED",
            "isPlaying": next.playing,
            "position": position,
            "timestamp": timestamp,
        }));
    } else if c.position && !c.track {
        out.push(json!({
            "type": "POSITION_CHANGED",
            "position": position,
            "timestamp": timestamp,
        }));
    }
    if c.volume {
        out.push(json!({ "type": "VOLUME_CHANGED", "volume": next.volume, "muted": false }));
    }
    if c.repeat {
        out.push(json!({ "type": "REPEAT_CHANGED", "repeat": repeat(next.repeat) }));
    }
    if c.shuffle {
        out.push(json!({ "type": "SHUFFLE_CHANGED", "shuffle": next.shuffle }));
    }
    out
}

/// The WebSocket's first message: the whole state.
pub(super) fn player_info(s: &Snapshot, now: SystemTime) -> Value {
    let (position, timestamp) = clock(s, now);
    let mut info = Map::new();
    info.insert("type".into(), "PLAYER_INFO".into());
    if let Some(t) = &s.track {
        info.insert("song".into(), song(t, s, now));
    }
    info.insert("isPlaying".into(), s.playing.into());
    info.insert("muted".into(), false.into());
    info.insert("position".into(), position.into());
    info.insert("timestamp".into(), timestamp.into());
    info.insert("volume".into(), s.volume.into());
    info.insert("repeat".into(), repeat(s.repeat).into());
    info.insert("shuffle".into(), s.shuffle.into());
    Value::Object(info)
}

/// Pear's song info. `views` is always 0: featherplay doesn't load the count.
pub(super) fn song(t: &Track, s: &Snapshot, now: SystemTime) -> Value {
    let (elapsed, _) = clock(s, now);
    let artist: Vec<&str> = t.artists.iter().map(|a| a.name.as_str()).collect();
    json!({
        "title": t.title,
        "artist": artist.join(", "),
        "views": 0,
        "imageSrc": t.thumbnail,
        "isPaused": !s.playing,
        "songDuration": t.duration.map_or(0, |d| d.as_secs()),
        "elapsedSeconds": elapsed,
        "url": format!("https://music.youtube.com/watch?v={}", t.id.0),
        "album": t.album.as_ref().map(|a| a.title.as_str()),
        "videoId": t.id.0,
        "mediaType": media_type(t.video_type),
    })
}

/// Pear's `RepeatMode`.
pub(super) fn repeat(mode: RepeatMode) -> &'static str {
    match mode {
        RepeatMode::Off => "NONE",
        RepeatMode::All => "ALL",
        RepeatMode::One => "ONE",
    }
}

/// Pear's `LikeType`.
pub(super) fn like(status: LikeStatus) -> &'static str {
    match status {
        LikeStatus::Like => "LIKE",
        LikeStatus::Dislike => "DISLIKE",
        LikeStatus::Indifferent => "INDIFFERENT",
    }
}

fn media_type(v: VideoType) -> &'static str {
    match v {
        VideoType::Song => "AUDIO",
        VideoType::MusicVideo => "ORIGINAL_MUSIC_VIDEO",
        VideoType::UserVideo => "USER_GENERATED_CONTENT",
        VideoType::Episode => "PODCAST_EPISODE",
        VideoType::Other => "OTHER_VIDEO",
    }
}

/// Where the track is at `at`: moved on from the snapshot's position while the sound
/// plays, never past the track's end.
pub(super) fn position_at(s: &Snapshot, at: SystemTime) -> Duration {
    let moved = if s.audible {
        at.duration_since(s.read_at).unwrap_or_default()
    } else {
        Duration::ZERO
    };
    let p = s.position + moved;
    match s.track.as_ref().and_then(|t| t.duration) {
        Some(end) => p.min(end),
        None => p,
    }
}

/// The position at `now` in whole seconds, and the Unix time in ms when the track was at
/// exactly that second.
fn clock(s: &Snapshot, now: SystemTime) -> (u64, u64) {
    let p = position_at(s, now);
    let secs = p.as_secs();
    let into_second = p - Duration::from_secs(secs);
    let at = now.checked_sub(into_second).unwrap_or(now);
    let ms = at
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
    (secs, ms)
}

#[cfg(test)]
mod tests {
    use featherplay_core::model::ArtistRef;

    use super::*;

    fn at(ms: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(1_800_000_000) + Duration::from_millis(ms)
    }

    fn track(id: &str) -> Track {
        Track {
            id: TrackId(id.into()),
            title: "Title".into(),
            artists: vec![
                ArtistRef {
                    id: None,
                    name: "One".into(),
                },
                ArtistRef {
                    id: None,
                    name: "Two".into(),
                },
            ],
            album: None,
            duration: Some(Duration::from_secs(100)),
            video_type: VideoType::MusicVideo,
            thumbnail: None,
            like: LikeStatus::Like,
            song_version: None,
        }
    }

    fn playing() -> Snapshot {
        Snapshot {
            track: Some(track("a")),
            playing: true,
            audible: true,
            position: Duration::from_secs(10),
            read_at: at(0),
            volume: 50,
            shuffle: false,
            repeat: RepeatMode::Off,
        }
    }

    fn later(s: &Snapshot, ms: u64) -> Snapshot {
        let mut next = s.clone();
        next.read_at = s.read_at + Duration::from_millis(ms);
        if s.audible {
            next.position = s.position + Duration::from_millis(ms);
        }
        next
    }

    fn types(evs: &[Value]) -> Vec<&str> {
        evs.iter()
            .map(|e| e["type"].as_str().unwrap_or("?"))
            .collect()
    }

    #[test]
    fn the_clock_moving_on_is_no_change() {
        let s = playing();
        assert!(!worth_sending(&s, &later(&s, 250)));
        assert!(!worth_sending(&s, &later(&s, 60_000)));
    }

    #[test]
    fn a_paused_track_standing_still_is_no_change() {
        let mut s = playing();
        s.playing = false;
        s.audible = false;
        assert!(!worth_sending(&s, &later(&s, 5_000)));
    }

    #[test]
    fn a_pause_is_one_state_event_with_the_position() {
        let s = playing();
        let mut paused = later(&s, 1_500);
        paused.playing = false;
        paused.audible = false;
        let evs = events(&s, &paused, paused.read_at);
        assert_eq!(types(&evs), ["PLAYER_STATE_CHANGED"]);
        assert_eq!(evs[0]["isPlaying"], false);
        assert_eq!(evs[0]["position"], 11);
        // 11.5 s in at `read_at`: whole second 11 was half a second before.
        assert_eq!(
            evs[0]["timestamp"],
            json!((at(1_000).duration_since(UNIX_EPOCH).unwrap()).as_millis() as u64)
        );
    }

    #[test]
    fn a_seek_is_a_position_event() {
        let s = playing();
        let mut seeked = later(&s, 250);
        seeked.position = Duration::from_secs(70);
        assert_eq!(
            types(&events(&s, &seeked, seeked.read_at)),
            ["POSITION_CHANGED"]
        );
    }

    #[test]
    fn a_small_wobble_is_no_jump() {
        let s = playing();
        let mut next = later(&s, 1_000);
        next.position -= Duration::from_millis(400);
        assert!(!worth_sending(&s, &next));
    }

    #[test]
    fn a_stall_is_sent_when_the_sound_stops_and_starts() {
        let s = playing();
        let mut stalled = later(&s, 250);
        stalled.audible = false;
        assert_eq!(
            types(&events(&s, &stalled, stalled.read_at)),
            ["POSITION_CHANGED"]
        );
        let mut back = stalled.clone();
        back.read_at += Duration::from_secs(3);
        back.audible = true;
        assert_eq!(
            types(&events(&stalled, &back, back.read_at)),
            ["POSITION_CHANGED"]
        );
    }

    #[test]
    fn a_new_track_carries_the_song_and_its_position() {
        let s = playing();
        let mut next = later(&s, 250);
        next.track = Some(track("b"));
        next.position = Duration::ZERO;
        let evs = events(&s, &next, next.read_at);
        assert_eq!(types(&evs), ["VIDEO_CHANGED"]);
        assert_eq!(evs[0]["song"]["videoId"], "b");
        assert_eq!(evs[0]["position"], 0);
    }

    #[test]
    fn a_new_track_that_starts_playing_also_sends_the_state() {
        let mut s = playing();
        s.playing = false;
        s.audible = false;
        let mut next = playing();
        next.track = Some(track("b"));
        assert_eq!(
            types(&events(&s, &next, next.read_at)),
            ["VIDEO_CHANGED", "PLAYER_STATE_CHANGED"]
        );
    }

    #[test]
    fn the_settings_each_send_their_event() {
        let s = playing();
        let mut next = later(&s, 250);
        next.volume = 80;
        next.repeat = RepeatMode::All;
        next.shuffle = true;
        let evs = events(&s, &next, next.read_at);
        assert_eq!(
            types(&evs),
            ["VOLUME_CHANGED", "REPEAT_CHANGED", "SHUFFLE_CHANGED"]
        );
        assert_eq!(evs[0]["volume"], 80);
        assert_eq!(evs[1]["repeat"], "ALL");
        assert_eq!(evs[2]["shuffle"], true);
    }

    #[test]
    fn a_like_alone_is_no_event() {
        let s = playing();
        let mut next = later(&s, 250);
        if let Some(t) = &mut next.track {
            t.like = LikeStatus::Indifferent;
        }
        assert!(!worth_sending(&s, &next));
    }

    #[test]
    fn the_position_stops_at_the_end() {
        let s = playing();
        assert_eq!(position_at(&s, at(500_000)), Duration::from_secs(100));
    }

    #[test]
    fn the_song_is_pears_shape() {
        let s = playing();
        let song = song(s.track.as_ref().unwrap(), &s, at(2_400));
        assert_eq!(song["artist"], "One, Two");
        assert_eq!(song["elapsedSeconds"], 12);
        assert_eq!(song["songDuration"], 100);
        assert_eq!(song["mediaType"], "ORIGINAL_MUSIC_VIDEO");
        assert_eq!(song["album"], Value::Null);
        assert_eq!(song["isPaused"], false);
        assert_eq!(song["url"], "https://music.youtube.com/watch?v=a");
    }

    #[test]
    fn the_player_info_leaves_out_a_missing_song() {
        let info = player_info(&Snapshot::default(), at(0));
        assert!(info.get("song").is_none());
        assert_eq!(info["isPlaying"], false);
    }
}
