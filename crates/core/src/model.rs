//! Track and session types shared by every crate.

use std::borrow::Cow;
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
    /// The same recording's other version, if known: a `MusicVideo`'s song (ATV), or for
    /// the song that plays in a music video's place (`into_song_version`), that video.
    /// Missing from what earlier versions saved.
    #[serde(default)]
    pub counterpart: Option<Box<Track>>,
}

impl Track {
    /// A music video's song version, when it has one.
    pub fn song_version(&self) -> Option<&Track> {
        self.counterpart
            .as_deref()
            .filter(|s| self.video_type == VideoType::MusicVideo && s.video_type == VideoType::Song)
    }

    /// The video a song version plays for: what the list it started from shows.
    pub fn video_version(&self) -> Option<&TrackId> {
        self.counterpart
            .as_deref()
            .filter(|v| self.video_type == VideoType::Song && v.video_type == VideoType::MusicVideo)
            .map(|v| &v.id)
    }

    /// A music video with a song version becomes that song, with the video as its
    /// counterpart: the bar, the media card, the lyrics and the scrobble then name the
    /// song that plays. Anything else stays as it is.
    pub fn into_song_version(mut self) -> Track {
        if self.song_version().is_none() {
            return self;
        }
        let Some(mut song) = self.counterpart.take() else {
            return self;
        };
        song.counterpart = Some(Box::new(self));
        *song
    }

    /// The other way round: a song version becomes the music video it plays for again,
    /// with the song as its counterpart. Anything else stays as it is.
    pub fn into_video_version(mut self) -> Track {
        if self.video_version().is_none() {
            return self;
        }
        let Some(mut video) = self.counterpart.take() else {
            return self;
        };
        video.counterpart = Some(Box::new(self));
        *video
    }

    /// The title as it's shown and scrobbled: a video's without its video tags
    /// (`scrobble::display_title`). `title` stays as YouTube Music has it, for lookups.
    pub fn display_title(&self) -> Cow<'_, str> {
        crate::scrobble::display_title(&self.title, self.video_type)
    }

    /// The ids this entry stands for: its own, and its counterpart's.
    pub fn ids(&self) -> impl Iterator<Item = &TrackId> {
        std::iter::once(&self.id).chain(self.counterpart.as_deref().map(|c| &c.id))
    }
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
