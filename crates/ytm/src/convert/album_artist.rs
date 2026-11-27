//! Album and artist pages

use featherplay_core::model::{
    AlbumId, AlbumRef, ArtistId, ArtistRef, PlaylistId, Track, TrackId, VideoType,
};
use serde_json::Value;
use ytmapi_rs::common::{ArtistChannelID, YoutubeID};
use ytmapi_rs::parse::{AlbumResult, GetArtist};
use ytmapi_rs::query::GetArtistQuery;

use super::explore::base64;
use super::home::{card_views, home_item, home_shelf};
use super::playlists::{
    first_run_text, flex_column, greyed_out, playlist_row, row_duration, row_like, runs_of,
};
use super::radio::{PAGE_TYPE, browse_target};
use super::{
    Extra, MUSIC_VIDEO_TYPE, PLAY_WATCH_ENDPOINT, ROW_THUMBNAILS, SINGLE, Skipped, album_ref,
    artist_ref, available, bare_playlist_id, best, best_raw, check_error, layout, like,
    log_left_out, merge_skipped, non_empty, process_lenient, video_type,
};
use crate::{
    Album, AlbumSummary, ApiError, Artist, ArtistSummary, ArtistVideo, BrowseRef, HomeItem,
    HomeShelf,
};

const ALBUM_HEADER: &str = "/contents/twoColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/0/musicResponsiveHeaderRenderer";
const ARTIST_SECTIONS: &str = "/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents";
const CAROUSEL_TITLE_RUN: &str =
    "/musicCarouselShelfRenderer/header/musicCarouselShelfBasicHeaderRenderer/title/runs/0";
/// Where a shelf goes on, the first found: a carousel's title link, then its "More" button;
/// the songs shelf's title link, then its "Show all".
const SHELF_MORE: [&str; 4] = [
    "/header/musicCarouselShelfBasicHeaderRenderer/title/runs/0/navigationEndpoint/browseEndpoint",
    "/header/musicCarouselShelfBasicHeaderRenderer/moreContentButton/buttonRenderer/navigationEndpoint/browseEndpoint",
    "/title/runs/0/navigationEndpoint/browseEndpoint",
    "/bottomEndpoint/browseEndpoint",
];
const DISCOGRAPHY: &str = "MUSIC_PAGE_TYPE_ARTIST_DISCOGRAPHY";
const ALBUM_ROWS: &str = "/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents/0/musicShelfRenderer/contents";

/// The album page does not repeat its own id, so the caller passes it.
/// Tracks inherit the album's artists (unless a row lists its own), the
/// album ref and the album cover.
///
/// Read from raw JSON one row at a time, like playlists: ytmapi-rs 0.3.3 fails the
/// whole album on one row without a duration or plays column. Greyed-out rows are
/// dropped; a row without a duration is kept without one.
pub fn album(raw: &str, id: &AlbumId) -> Result<Album, ApiError> {
    let v = available(raw)?;
    check_error(&v)?;
    let h = v
        .pointer(ALBUM_HEADER)
        .ok_or_else(|| ApiError::Parse(format!("album has no header ({})", layout(&v))))?;
    let title = first_run_text(h.get("title"))
        .ok_or_else(|| ApiError::Parse("album has no title".into()))?;
    let artists = runs_of(h.get("straplineTextOne"))
        .iter()
        .step_by(2)
        .map(|run| ArtistRef {
            id: run
                .pointer("/navigationEndpoint/browseEndpoint/browseId")
                .and_then(Value::as_str)
                .map(|id| ArtistId(id.to_string())),
            name: run
                .get("text")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        })
        .collect();
    let summary = AlbumSummary {
        id: id.clone(),
        title,
        artists,
        year: runs_of(h.get("subtitle"))
            .get(2)
            .and_then(|r| r.get("text"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .and_then(non_empty),
        thumbnail: best_raw(h.pointer(ROW_THUMBNAILS)),
        // The header's subtitle starts with the kind: `Single • 2024`.
        single: runs_of(h.get("subtitle"))
            .first()
            .and_then(|r| r.get("text"))
            .and_then(Value::as_str)
            == Some(SINGLE),
    };
    let album_ref = AlbumRef {
        id: id.clone(),
        title: summary.title.clone(),
    };
    let rows = v
        .pointer(ALBUM_ROWS)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let tracks = rows
        .iter()
        .filter_map(|item| {
            let r = item.get("musicResponsiveListItemRenderer")?;
            if greyed_out(r) {
                return None;
            }
            let title = first_run_text(flex_column(r, 0))?;
            let watch = r.pointer(PLAY_WATCH_ENDPOINT);
            let tid = watch
                .and_then(|w| w.get("videoId"))
                .or_else(|| r.pointer("/playlistItemData/videoId"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())?;
            let video_type = watch
                .and_then(|w| w.pointer(MUSIC_VIDEO_TYPE))
                .or_else(|| {
                    runs_of(flex_column(r, 0)).first().and_then(|run| {
                        run.pointer("/navigationEndpoint/watchEndpoint")
                            .and_then(|w| w.pointer(MUSIC_VIDEO_TYPE))
                    })
                })
                .and_then(Value::as_str)
                .map(video_type)
                .unwrap_or(VideoType::Song);
            let own: Vec<ArtistRef> = runs_of(flex_column(r, 1))
                .iter()
                .filter_map(|run| {
                    let (aid, page) = browse_target(run)?;
                    (page == "MUSIC_PAGE_TYPE_ARTIST" || page == "MUSIC_PAGE_TYPE_USER_CHANNEL")
                        .then(|| ArtistRef {
                            id: Some(ArtistId(aid.to_string())),
                            name: run
                                .get("text")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_string(),
                        })
                })
                .collect();
            Some(Track {
                artists: if own.is_empty() {
                    summary.artists.clone()
                } else {
                    own
                },
                album: Some(album_ref.clone()),
                duration: row_duration(r),
                video_type,
                thumbnail: summary.thumbnail.clone(),
                like: row_like(r),
                counterpart: None,
                title,
                id: TrackId(tid.to_string()),
            })
        })
        .collect();
    Ok(Album { summary, tracks })
}

/// Top songs carry no duration on the artist page. A top song ytmapi-rs can't read is read
/// from its row (`playlist_row`); an album it can't read is left out.
///
/// ytmapi-rs 0.3.3 reads the top songs and the albums, and only from a page in English (it
/// knows a shelf by its title); it leaves the singles, videos, playlists and related
/// artists out. Those are read here from the raw carousels (`artist_shelves`), and the
/// albums too when ytmapi-rs found none. A release that two shelves show is listed once.
///
/// Each shelf shows its first items only; where it goes on (`songs_all`, `albums_all`, …)
/// is read from its header, so in any language.
///
/// The page is parsed once here, and all of it that ytmapi-rs doesn't read is read before
/// ytmapi-rs parses it: one parsed page at a time.
pub fn artist(raw: &str) -> Result<Artist, ApiError> {
    let v = available(raw)?;
    let extra = Extra::of(&v)?;
    // The page's one list of rows is the top songs.
    let songs_all = v
        .pointer(ARTIST_SECTIONS)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find_map(|s| s.get("musicShelfRenderer"))
        .and_then(shelf_more)
        .and_then(playlist_more);
    let shelves = artist_shelves(&v);
    drop(v);
    let q = GetArtistQuery::new(ArtistChannelID::from_raw(""));
    let (a, skipped): (GetArtist, _) = process_lenient(raw, &q)?;
    // The songs shelf is the page's one list of rows; ytmapi-rs reads cards only in the
    // albums carousel.
    let (song_rows, cards): (Vec<Skipped>, Vec<Skipped>) = skipped
        .into_iter()
        .partition(|s| s.renderer == "musicResponsiveListItemRenderer");
    log_left_out("artist albums", &cards);
    let summary = ArtistSummary {
        id: ArtistId(a.channel_id.get_raw().to_string()),
        name: a.name,
        thumbnail: best(&a.thumbnails),
    };
    let me = ArtistRef {
        id: Some(summary.id.clone()),
        name: summary.name.clone(),
    };
    let releases = a.top_releases;
    let top_songs = releases
        .songs
        .map(|s| s.results)
        .unwrap_or_default()
        .into_iter()
        .map(|s| {
            let id = s.video_id.get_raw().to_string();
            Track {
                artists: s.artists.into_iter().map(artist_ref).collect(),
                album: Some(album_ref(s.album)),
                duration: None,
                video_type: extra.video_type_or(&id, VideoType::Song),
                thumbnail: extra.rows.get(&id).and_then(|r| r.thumbnail.clone()),
                like: like(&s.like_status),
                counterpart: None,
                title: s.title,
                id: TrackId(id),
            }
        })
        .collect();
    let top_songs = merge_skipped("artist top songs", top_songs, song_rows, playlist_row);
    let releases_of =
        |r: Option<ytmapi_rs::parse::GetArtistAlbums>, single: bool| -> Vec<AlbumSummary> {
            r.map(|r| r.results)
                .unwrap_or_default()
                .into_iter()
                .map(|a: AlbumResult| AlbumSummary {
                    id: AlbumId(a.album_id.get_raw().to_string()),
                    title: a.title,
                    artists: vec![me.clone()],
                    year: non_empty(a.year),
                    thumbnail: best(&a.thumbnails),
                    single,
                })
                .collect()
        };
    let mut artist = Artist {
        albums: releases_of(releases.albums, false),
        singles: releases_of(releases.singles, true),
        summary,
        top_songs,
        videos: Vec::new(),
        live: Vec::new(),
        playlists: Vec::new(),
        featured_on: Vec::new(),
        related: Vec::new(),
        songs_all,
        albums_all: None,
        singles_all: None,
        videos_all: None,
    };
    let typed_albums = !artist.albums.is_empty();
    for (kind, items, more) in shelves {
        let shelf = kind.of(&me.name);
        match (shelf, more) {
            (Shelf::Albums, Some((to, page))) if page == DISCOGRAPHY => {
                artist.albums_all = Some(to);
            }
            (Shelf::Singles, Some((to, page))) if page == DISCOGRAPHY => {
                artist.singles_all = Some(to);
            }
            (Shelf::Videos, Some((to, page))) => artist.videos_all = playlist_more((to, &page)),
            _ => {}
        }
        for (item, views) in items {
            let video = |track| ArtistVideo { track, views };
            match (shelf, item) {
                (Shelf::Albums, HomeItem::Album(a)) if !typed_albums => {
                    release(&mut artist, a, &me, false);
                }
                (Shelf::Singles, HomeItem::Album(a)) => release(&mut artist, a, &me, true),
                (Shelf::Videos, HomeItem::Track(t)) => artist.videos.push(video(t)),
                (Shelf::Live, HomeItem::Track(t)) => artist.live.push(video(t)),
                (Shelf::Playlists, HomeItem::Playlist(p)) => artist.playlists.push(p),
                (Shelf::FeaturedOn, HomeItem::Playlist(p)) => artist.featured_on.push(p),
                (Shelf::Related, HomeItem::Artist(a)) => artist.related.push(a),
                _ => {}
            }
        }
    }
    Ok(artist)
}

/// Adds a release to the albums, or with `single` to the singles, unless either has it
/// already. A card that names no artist is the page's artist's.
fn release(artist: &mut Artist, mut a: AlbumSummary, me: &ArtistRef, single: bool) {
    if artist
        .albums
        .iter()
        .chain(&artist.singles)
        .any(|b| b.id == a.id)
    {
        return;
    }
    if a.artists.is_empty() {
        a.artists = vec![me.clone()];
    }
    if single {
        artist.singles.push(a);
    } else {
        artist.albums.push(a);
    }
}

/// A carousel's cards, each with the views it says (a video's).
type Cards = Vec<(HomeItem, Option<String>)>;

/// Where a shelf goes on: the page, and its type.
type More<'v> = (BrowseRef, &'v str);

/// `More`, owned: kept once the page is gone.
type Goes = (BrowseRef, String);

/// Where a shelf goes on (`SHELF_MORE`): the page, and its type (`MUSIC_PAGE_TYPE_…`, or
/// empty when not said).
fn shelf_more(r: &Value) -> Option<More<'_>> {
    let b = SHELF_MORE.iter().find_map(|p| r.pointer(p))?;
    let id = b
        .get("browseId")
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())?;
    let to = BrowseRef {
        id: id.to_string(),
        params: b.get("params").and_then(Value::as_str).map(str::to_string),
    };
    Some((
        to,
        b.pointer(PAGE_TYPE)
            .and_then(Value::as_str)
            .unwrap_or_default(),
    ))
}

/// The playlist a shelf goes on to (the songs', the videos'), as a bare id.
fn playlist_more((to, page): More<'_>) -> Option<PlaylistId> {
    (page == "MUSIC_PAGE_TYPE_PLAYLIST" || to.id.starts_with("VL"))
        .then(|| PlaylistId(bare_playlist_id(&to.id).to_string()))
}

/// What one of the artist page's carousels holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Shelf {
    Albums,
    /// "Singles & EPs".
    Singles,
    Videos,
    /// "Live performances".
    Live,
    /// "Playlists by …", the artist's own.
    Playlists,
    /// "Featured on": others' playlists.
    FeaturedOn,
    /// "Fans might also like".
    Related,
    /// What the page has besides ("From your library", a channel's podcasts), or a shelf
    /// not known.
    Other,
}

/// A shelf's kind as the page tells it, before the artist's name is known (ytmapi-rs reads
/// it).
enum Kind {
    Is(Shelf),
    /// Playlists whose title links nowhere: the artist's own (`Shelf::Playlists`) when one of
    /// these authors (`bare_name`s) is the artist, else `Shelf::FeaturedOn`.
    ByAuthors(Vec<String>),
}

impl Kind {
    /// The shelf, on the page of the artist called `artist`.
    fn of(self, artist: &str) -> Shelf {
        match self {
            Kind::Is(shelf) => shelf,
            Kind::ByAuthors(authors) if authors.iter().any(|a| a == bare_name(artist)) => {
                Shelf::Playlists
            }
            Kind::ByAuthors(_) => Shelf::FeaturedOn,
        }
    }
}

/// The artist page's carousels, in its order, each read as Home reads a shelf, with what
/// it holds: each card, and the views it says (a video's); and where it goes on
/// (`shelf_more`).
fn artist_shelves(v: &Value) -> Vec<(Kind, Cards, Option<Goes>)> {
    v.pointer(ARTIST_SECTIONS)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|section| {
            let shelf = home_shelf(section)?;
            let kind = english_shelf(&shelf.title)
                .map_or_else(|| shelf_by_shape(section, &shelf), Kind::Is);
            let cards = section
                .pointer("/musicCarouselShelfRenderer/contents")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|card| {
                    let runs = card
                        .pointer("/musicTwoRowItemRenderer/subtitle/runs")
                        .and_then(Value::as_array)
                        .map(Vec::as_slice)
                        .unwrap_or_default();
                    Some((home_item(card)?, card_views(runs)))
                })
                .collect();
            let more = section
                .get("musicCarouselShelfRenderer")
                .and_then(shelf_more)
                .map(|(to, page)| (to, page.to_string()));
            Some((kind, cards, more))
        })
        .collect()
}

/// A shelf as a page in English titles it (checked 2026-11-25 on seven artists' pages).
fn english_shelf(title: &str) -> Option<Shelf> {
    let t = title.trim().to_lowercase();
    Some(match t.as_str() {
        "albums" => Shelf::Albums,
        "singles" | "singles & eps" | "singles and eps" | "eps" => Shelf::Singles,
        "videos" => Shelf::Videos,
        "live performances" => Shelf::Live,
        "featured on" => Shelf::FeaturedOn,
        "fans might also like" => Shelf::Related,
        "from your library" | "latest episodes" | "podcasts" => Shelf::Other,
        _ if t.starts_with("playlists") => Shelf::Playlists,
        _ => return None,
    })
}

/// A shelf told by its shape, for a page in another language: where its title links (the
/// discography says which releases it lists), and what its cards are.
fn shelf_by_shape(section: &Value, shelf: &HomeShelf) -> Kind {
    let header = section.pointer(CAROUSEL_TITLE_RUN);
    let target = header.and_then(browse_target);
    match target {
        Some((_, "MUSIC_PAGE_TYPE_ARTIST_DISCOGRAPHY")) => {
            let params = header
                .and_then(|h| h.pointer("/navigationEndpoint/browseEndpoint/params"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            return Kind::Is(match discography_kind(params) {
                Some(1) => Shelf::Albums,
                Some(2) => Shelf::Singles,
                _ => Shelf::Other,
            });
        }
        // The account's own songs and albums of the artist.
        Some((id, _)) if id.starts_with("MPLA") => return Kind::Is(Shelf::Other),
        _ => {}
    }
    let items = &shelf.items;
    let all = |f: fn(&HomeItem) -> bool| items.iter().all(f);
    Kind::Is(if all(|i| matches!(i, HomeItem::Artist(_))) {
        Shelf::Related
    } else if all(|i| matches!(i, HomeItem::Album(_))) {
        // A single's card says what it is before its year (`Single • 2026`); an album's
        // has the year alone.
        if kind_words(section) {
            Shelf::Singles
        } else {
            Shelf::Albums
        }
    } else if all(|i| matches!(i, HomeItem::Playlist(_))) {
        if target.is_some() {
            Shelf::Playlists
        } else {
            let authors = items.iter().filter_map(|i| match i {
                HomeItem::Playlist(p) => p.author.as_deref().map(|a| bare_name(a).to_string()),
                _ => None,
            });
            return Kind::ByAuthors(authors.collect());
        }
    } else if all(|i| matches!(i, HomeItem::Track(_))) {
        match target {
            Some((_, "MUSIC_PAGE_TYPE_ARTIST")) => Shelf::Live,
            _ => Shelf::Videos,
        }
    } else {
        Shelf::Other
    })
}

/// A name without the spaces and direction marks a card may wrap it in (`DJ TYNA \u{200e} `).
fn bare_name(name: &str) -> &str {
    name.trim_matches(|c: char| c.is_whitespace() || c == '\u{200e}' || c == '\u{200f}')
}

/// Some card of the carousel has a kind word before its year: more than one run in its
/// subtitle.
fn kind_words(section: &Value) -> bool {
    section
        .pointer("/musicCarouselShelfRenderer/contents")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|item| item.pointer("/musicTwoRowItemRenderer/subtitle/runs"))
        .filter_map(Value::as_array)
        .any(|runs| runs.len() > 1)
}

/// Which releases an artist's discography page lists, from its `params` (URL-encoded
/// base64 of a protobuf): 1 albums, 2 singles and EPs, as field 1 of its field 15
/// (`ggMHegUIARoBAQ==` and `ggMIegYIAhoCAQI=`). `None` for params of another shape.
fn discography_kind(params: &str) -> Option<u8> {
    let bytes = base64(params.trim_end_matches("%3D").trim_end_matches('='))?;
    bytes
        .windows(4)
        .find(|w| w[0] == 0x7a && w[2] == 0x08)
        .map(|w| w[3])
}
