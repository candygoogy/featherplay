//! What a search for a link opens (`search_page`). The link itself is parsed in the
//! controller crate (`featherplay_controller::link`), offline, keeping only its id.

pub use featherplay_controller::link::{Link, parse, track_link};

use crate::router::Page;

/// The page a search for `query` opens: a playlist, album or artist link opens that page; a
/// song link shows that one song on the Search page, under its link without tracking
/// parameters; anything else searches the text.
pub fn search_page(query: &str) -> Page {
    match parse(query) {
        Some(Link::Track(id)) => Page::Search {
            query: track_link(&id),
        },
        Some(Link::Playlist(id)) => Page::Playlist { id },
        Some(Link::Album(id)) => Page::Album { id },
        Some(Link::Artist(id)) => Page::Artist { id },
        None => Page::Search {
            query: query.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use featherplay_core::model::{AlbumId, ArtistId, PlaylistId};

    use super::*;

    #[test]
    fn a_song_link_searches_its_bare_link() {
        assert_eq!(
            search_page("https://youtu.be/UjiL0RxrDAI?si=Abc123"),
            Page::Search {
                query: "https://music.youtube.com/watch?v=UjiL0RxrDAI".into()
            }
        );
    }

    #[test]
    fn playlist_album_and_artist_links_open_their_page() {
        assert_eq!(
            search_page(
                "https://music.youtube.com/playlist?list=OLAK5uy_nMr9h2VlS-2PULNz3M3XVXQj_P3C2bqaY&si=x"
            ),
            Page::Playlist {
                id: PlaylistId("OLAK5uy_nMr9h2VlS-2PULNz3M3XVXQj_P3C2bqaY".into())
            }
        );
        assert_eq!(
            search_page("https://music.youtube.com/browse/MPREb_dMHztPsrDgN"),
            Page::Album {
                id: AlbumId("MPREb_dMHztPsrDgN".into())
            }
        );
        assert_eq!(
            search_page("https://music.youtube.com/channel/UCX5Rv4Lv4h3E2Vh7RmQ9GfQ"),
            Page::Artist {
                id: ArtistId("UCX5Rv4Lv4h3E2Vh7RmQ9GfQ".into())
            }
        );
    }

    #[test]
    fn text_searches_as_typed() {
        assert_eq!(
            search_page("daft punk"),
            Page::Search {
                query: "daft punk".into()
            }
        );
    }
}
