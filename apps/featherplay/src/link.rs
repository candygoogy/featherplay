//! A link pasted into the search field: a song or video, a playlist, an album or an artist
//! on the music site or the video site. Pure and offline. Only the id is kept: tracking
//! parameters (`si`, `pp`, `feature`), timestamps and anything else in the link are
//! dropped here and never sent anywhere. Every id is checked for its shape before any
//! request; anything else (another site, a bad id, plain text) is no link, and the text is
//! searched as typed.

use featherplay_core::model::{AlbumId, ArtistId, PlaylistId, TrackId};

use crate::router::Page;

/// What a link points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    Track(TrackId),
    Playlist(PlaylistId),
    Album(AlbumId),
    Artist(ArtistId),
}

/// Longer text isn't parsed: no link of ours comes near it.
const MAX_LEN: usize = 2048;

/// Where a link can come from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Site {
    /// `youtu.be/ID`.
    Short,
    /// The video site, `www.` and `m.` included.
    Video,
    /// The music site.
    Music,
}

/// The link in `text`, if it is one of ours. The text is one token, with or without
/// `http://` or `https://`, on one of our hosts (no user, no port); the id is the only thing
/// taken from it.
pub fn parse(text: &str) -> Option<Link> {
    let text = text.trim();
    if text.is_empty() || text.len() > MAX_LEN || text.contains(char::is_whitespace) {
        return None;
    }
    let url = if text.contains("://") {
        reqwest::Url::parse(text).ok()?
    } else {
        reqwest::Url::parse(&format!("https://{text}")).ok()?
    };
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return None;
    }
    let site = match url.host_str()? {
        "youtu.be" => Site::Short,
        "youtube.com" | "www.youtube.com" | "m.youtube.com" => Site::Video,
        "music.youtube.com" => Site::Music,
        _ => return None,
    };
    // A trailing slash leaves an empty last segment.
    let segments: Vec<&str> = url.path_segments()?.filter(|s| !s.is_empty()).collect();
    let param = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    match (site, segments.as_slice()) {
        (Site::Short, [id]) => video(id),
        (Site::Short, _) => None,
        (_, ["watch"]) => match param("v") {
            Some(id) => video(&id),
            None => playlist(&param("list")?),
        },
        (_, ["shorts", id]) => video(id),
        (_, ["playlist"]) => playlist(&param("list")?),
        (Site::Music, ["channel", id]) => artist(id),
        (Site::Music, ["browse", id]) => album(id),
        _ => None,
    }
}

/// Ids are made of letters, digits, `_` and `-`.
fn id_chars(id: &str) -> bool {
    id.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// A video id: 11 characters.
fn video(id: &str) -> Option<Link> {
    (id.len() == 11 && id_chars(id)).then(|| Link::Track(TrackId(id.into())))
}

/// A playlist id: Liked songs (`LM`), or a known prefix and enough characters after it
/// (`PL` lists, `OLAK5uy_` album lists, `RD` mixes and curated lists, `UU` uploads,
/// `FL` favourites).
fn playlist(id: &str) -> Option<Link> {
    const PREFIXES: [&str; 5] = ["PL", "OLAK5uy_", "RD", "UU", "FL"];
    let shaped =
        (13..=64).contains(&id.len()) && id_chars(id) && PREFIXES.iter().any(|p| id.starts_with(p));
    (id == "LM" || shaped).then(|| Link::Playlist(PlaylistId(id.into())))
}

/// An artist's channel id: `UC` and 22 characters.
fn artist(id: &str) -> Option<Link> {
    (id.len() == 24 && id.starts_with("UC") && id_chars(id))
        .then(|| Link::Artist(ArtistId(id.into())))
}

/// An album's browse id: `MPRE` and the rest (`MPREb_` and 11 characters today).
fn album(id: &str) -> Option<Link> {
    ((10..=40).contains(&id.len()) && id.starts_with("MPRE") && id_chars(id))
        .then(|| Link::Album(AlbumId(id.into())))
}

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

/// The song's link as the Search page shows it: just the id.
pub fn track_link(id: &TrackId) -> String {
    format!("https://music.youtube.com/watch?v={}", id.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "UjiL0RxrDAI";

    fn track(id: &str) -> Option<Link> {
        Some(Link::Track(TrackId(id.into())))
    }

    fn playlist(id: &str) -> Option<Link> {
        Some(Link::Playlist(PlaylistId(id.into())))
    }

    #[test]
    fn song_and_video_links_give_the_track() {
        let links = [
            "https://youtu.be/UjiL0RxrDAI",
            "https://youtu.be/UjiL0RxrDAI?si=Abc123_xyz-TOKEN",
            "http://youtu.be/UjiL0RxrDAI",
            "youtu.be/UjiL0RxrDAI",
            "https://youtube.com/watch?v=UjiL0RxrDAI",
            "https://www.youtube.com/watch?v=UjiL0RxrDAI",
            "https://m.youtube.com/watch?v=UjiL0RxrDAI",
            "https://music.youtube.com/watch?v=UjiL0RxrDAI",
            "music.youtube.com/watch?v=UjiL0RxrDAI&si=x",
            "www.youtube.com/watch?v=UjiL0RxrDAI",
            "https://WWW.YouTube.com/watch?v=UjiL0RxrDAI",
            "https://www.youtube.com/watch?feature=share&v=UjiL0RxrDAI&pp=ygUEZGFmdA%3D%3D",
            "https://www.youtube.com/watch?v=UjiL0RxrDAI&t=42s",
            "https://youtu.be/UjiL0RxrDAI?t=42",
            "https://www.youtube.com/watch?v=UjiL0RxrDAI#t=1m",
            "https://youtube.com/shorts/UjiL0RxrDAI",
            "https://www.youtube.com/shorts/UjiL0RxrDAI?feature=share",
            "https://m.youtube.com/shorts/UjiL0RxrDAI/",
            "  https://youtu.be/UjiL0RxrDAI  ",
        ];
        for link in links {
            assert_eq!(parse(link), track(ID), "{link}");
        }
    }

    #[test]
    fn a_watch_link_with_a_list_plays_the_track() {
        assert_eq!(
            parse("https://music.youtube.com/watch?v=UjiL0RxrDAI&list=RDAMVMUjiL0RxrDAI"),
            track(ID)
        );
        assert_eq!(
            parse(
                "https://www.youtube.com/watch?list=PLx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG&v=UjiL0RxrDAI&index=3"
            ),
            track(ID)
        );
    }

    #[test]
    fn playlist_links_give_the_playlist() {
        let cases = [
            (
                "https://music.youtube.com/playlist?list=PLx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG",
                "PLx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG",
            ),
            (
                "https://www.youtube.com/playlist?list=PLx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG&si=abc",
                "PLx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG",
            ),
            (
                "youtube.com/playlist?list=PLx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG",
                "PLx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG",
            ),
            (
                "http://m.youtube.com/playlist?list=PLx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG",
                "PLx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG",
            ),
            // An album's playlist.
            (
                "https://music.youtube.com/playlist?list=OLAK5uy_nMr9h2VlS-2PULNz3M3XVXQj_P3C2bqaY",
                "OLAK5uy_nMr9h2VlS-2PULNz3M3XVXQj_P3C2bqaY",
            ),
            // A curated playlist of the music site.
            (
                "https://music.youtube.com/playlist?list=RDCLAK5uy_kmPRjHDECIcuVwnKsx2Ng7fyNgFKWNJFs",
                "RDCLAK5uy_kmPRjHDECIcuVwnKsx2Ng7fyNgFKWNJFs",
            ),
            // A watch link with only a list opens the list.
            (
                "https://www.youtube.com/watch?list=PLx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG",
                "PLx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG",
            ),
            ("https://music.youtube.com/playlist?list=LM", "LM"),
        ];
        for (link, id) in cases {
            assert_eq!(parse(link), playlist(id), "{link}");
        }
    }

    #[test]
    fn artist_and_album_links_on_the_music_site() {
        assert_eq!(
            parse("https://music.youtube.com/channel/UCX5Rv4Lv4h3E2Vh7RmQ9GfQ?si=abc"),
            Some(Link::Artist(ArtistId("UCX5Rv4Lv4h3E2Vh7RmQ9GfQ".into())))
        );
        assert_eq!(
            parse("music.youtube.com/browse/MPREb_dMHztPsrDgN"),
            Some(Link::Album(AlbumId("MPREb_dMHztPsrDgN".into())))
        );
    }

    #[test]
    fn bad_ids_are_no_link() {
        let links = [
            // Video ids are 11 characters of [A-Za-z0-9_-].
            "https://youtu.be/UjiL0RxrDA",
            "https://youtu.be/UjiL0RxrDAIx",
            "https://youtu.be/UjiL0Rx.DAI",
            "https://youtu.be/UjiL0Rx%20AI",
            "https://www.youtube.com/watch?v=",
            "https://www.youtube.com/watch?v=UjiL0RxrDA%22",
            "https://www.youtube.com/shorts/",
            "https://youtu.be/",
            "https://youtu.be/UjiL0RxrDAI/more",
            // Playlists start with a known prefix.
            "https://www.youtube.com/playlist?list=XXx0sYbCqOb8TBPRdmBHs5Iftvv9TPboYG",
            "https://www.youtube.com/playlist?list=PL",
            "https://www.youtube.com/playlist?list=PLx0sYbC%27Ob8TBPRdmBHs5Iftvv9TPboYG",
            "https://www.youtube.com/playlist",
            // Channels are UC and 22 characters; albums MPRE and the rest.
            "https://music.youtube.com/channel/UCX5Rv4Lv4h3E2Vh7RmQ9Gf",
            "https://music.youtube.com/channel/XXX5Rv4Lv4h3E2Vh7RmQ9GfQ",
            "https://music.youtube.com/browse/FEmusic_home",
            "https://music.youtube.com/browse/MPRE",
            // Pages that aren't one item.
            "https://www.youtube.com/",
            "https://music.youtube.com/explore",
            "https://www.youtube.com/results?search_query=daft+punk",
        ];
        for link in links {
            assert_eq!(parse(link), None, "{link}");
        }
    }

    #[test]
    fn other_sites_and_plain_text_are_no_link() {
        let texts = [
            "daft punk",
            "moby",
            "UjiL0RxrDAI",
            "",
            "   ",
            "https://example.com/watch?v=UjiL0RxrDAI",
            "https://youtube.com.example.com/watch?v=UjiL0RxrDAI",
            "https://notyoutube.com/watch?v=UjiL0RxrDAI",
            "https://music.youtube.com@example.com/watch?v=UjiL0RxrDAI",
            "https://user:pass@www.youtube.com/watch?v=UjiL0RxrDAI",
            "https://www.youtube.com:8080/watch?v=UjiL0RxrDAI",
            "https://youtube.com./watch?v=UjiL0RxrDAI",
            "ftp://youtu.be/UjiL0RxrDAI",
            "javascript:alert(1)",
            "file:///C:/watch?v=UjiL0RxrDAI",
            "https://youtu.be/UjiL0RxrDAI and more",
            "https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC",
            // Artists and albums are only taken from the music site.
            "https://www.youtube.com/channel/UCX5Rv4Lv4h3E2Vh7RmQ9GfQ",
            "https://www.youtube.com/browse/MPREb_dMHztPsrDgN",
        ];
        for text in texts {
            assert_eq!(parse(text), None, "{text:?}");
        }
    }

    #[test]
    fn a_very_long_text_is_no_link() {
        let long = format!("https://youtu.be/{ID}?si={}", "a".repeat(5000));
        assert_eq!(parse(&long), None);
    }

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
