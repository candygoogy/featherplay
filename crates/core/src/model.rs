//! Track and session types shared by every crate.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// A YouTube videoId.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TrackId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PlaylistId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AlbumId(pub String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ArtistId(pub String);

/// The API's `musicVideoType`: ATV, OMV, UGC, PODCAST_EPISODE, anything else.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum VideoType {
    Song,
    MusicVideo,
    UserVideo,
    Episode,
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LikeStatus {
    Like,
    Dislike,
    Indifferent,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtistRef {
    pub id: Option<ArtistId>,
    pub name: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AlbumRef {
    pub id: AlbumId,
    pub title: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub id: TrackId,
    pub title: String,
    pub artists: Vec<ArtistRef>,
    pub album: Option<AlbumRef>,
    pub duration: Option<Duration>,
    pub video_type: VideoType,
    pub thumbnail: Option<String>,
    pub like: LikeStatus,
    /// The ATV counterpart of a `MusicVideo`, if known.
    pub song_version: Option<TrackId>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

/// What a list (a queue that isn't a radio) does when it runs out with Repeat off.
/// Repeat All always restarts it; a radio always continues by itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ListEnd {
    /// The last track stays current, and Next does nothing.
    #[default]
    Stop,
    /// Near the end, a radio of the last track is fetched once and plays on after it.
    Autoplay,
    /// Next on the last track, and its end, go back to the first track.
    Restart,
}

/// Persisted by the store crate, restored by `Player`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub queue: Vec<Track>,
    pub index: usize,
    pub position_ms: u64,
    pub radio: bool,
    pub shuffle: bool,
    pub repeat: RepeatMode,
}
