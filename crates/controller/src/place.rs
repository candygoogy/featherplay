//! Where a pick leads or a queue came from, as the window shows it: an album, a playlist, an
//! artist, the history. The app maps it to its page (`From<Place> for Page`), which this
//! crate can't name: a page is the window's.

use featherplay_core::model::{AlbumId, ArtistId, PlaylistId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    Album(AlbumId),
    /// `"LM"` is Liked songs.
    Playlist(PlaylistId),
    Artist(ArtistId),
    /// The listening history.
    History,
}
