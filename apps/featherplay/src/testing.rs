//! What the unit tests share.

use featherplay_core::model::{LikeStatus, Track, TrackId, VideoType};

/// A song with id `id`, titled the same and nothing else known. A test sets the fields it
/// reads: `Track { title, ..track(id) }`.
pub fn track(id: &str) -> Track {
    Track {
        id: TrackId(id.into()),
        title: id.into(),
        artists: Vec::new(),
        album: None,
        duration: None,
        video_type: VideoType::Song,
        thumbnail: None,
        like: LikeStatus::Indifferent,
        counterpart: None,
    }
}
