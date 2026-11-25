//! Album and artist pages

use featherplay_core::model::{AlbumId, AlbumRef, ArtistId, ArtistRef, Track, TrackId, VideoType};
use serde_json::Value;
use ytmapi_rs::common::{ArtistChannelID, YoutubeID};
use ytmapi_rs::parse::{AlbumResult, GetArtist};
use ytmapi_rs::query::GetArtistQuery;

use super::explore::base64;
use super::home::{card_views, home_item, home_shelf};
use super::playlists::{
    first_run_text, flex_column, greyed_out, playlist_row, row_duration, row_like, runs_of,
};
use super::radio::browse_target;
use super::{
    Extra, MUSIC_VIDEO_TYPE, PLAY_WATCH_ENDPOINT, ROW_THUMBNAILS, SINGLE, Skipped, album_ref,
    artist_ref, best, best_raw, check_error, json, layout, like, log_left_out, merge_skipped,
    non_empty, process_lenient, unavailable, video_type,
};
use crate::{
    Album, AlbumSummary, ApiError, Artist, ArtistSummary, ArtistVideo, HomeItem, HomeShelf,
};

const ALBUM_HEADER: &str = "/contents/twoColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/0/musicResponsiveHeaderRenderer";
const ARTIST_SECTIONS: &str = "/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents";
const CAROUSEL_TITLE_RUN: &str =
    "/musicCarouselShelfRenderer/header/musicCarouselShelfBasicHeaderRenderer/title/runs/0";
const ALBUM_ROWS: &str = "/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents/0/musicShelfRenderer/contents";

/// The album page does not repeat its own id, so the caller passes it.
/// Tracks inherit the album's artists (unless a row lists its own), the
/// album ref and the album cover.
///
/// Read from raw JSON one row at a time, like playlists: ytmapi-rs 0.3.3 fails the
/// whole album on one row without a duration or plays column. Greyed-out rows are
/// dropped; a row without a duration is kept without one.
pub fn album(raw: &str, id: &AlbumId) -> Result<Album, ApiError> {
    if unavailable(raw) {
        return Err(ApiError::NotFound);
    }
    let v = json(raw)?;
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
pub fn artist(raw: &str) -> Result<Artist, ApiError> {
    if unavailable(raw) {
        return Err(ApiError::NotFound);
    }
    let extra = Extra::from_raw(raw)?;
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
    };
    let typed_albums = !artist.albums.is_empty();
    for (shelf, items) in artist_shelves(&json(raw)?, &me.name) {
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

/// The artist page's carousels, in its order, each read as Home reads a shelf, with what
/// it holds: each card, and the views it says (a video's). `artist` is the page's artist's
/// name.
fn artist_shelves(v: &Value, artist: &str) -> Vec<(Shelf, Cards)> {
    v.pointer(ARTIST_SECTIONS)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|section| {
            let shelf = home_shelf(section)?;
            let kind = english_shelf(&shelf.title)
                .unwrap_or_else(|| shelf_by_shape(section, &shelf, artist));
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
            Some((kind, cards))
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
fn shelf_by_shape(section: &Value, shelf: &HomeShelf, artist: &str) -> Shelf {
    let header = section.pointer(CAROUSEL_TITLE_RUN);
    let target = header.and_then(browse_target);
    match target {
        Some((_, "MUSIC_PAGE_TYPE_ARTIST_DISCOGRAPHY")) => {
            let params = header
                .and_then(|h| h.pointer("/navigationEndpoint/browseEndpoint/params"))
                .and_then(Value::as_str)
                .unwrap_or_default();
            return match discography_kind(params) {
                Some(1) => Shelf::Albums,
                Some(2) => Shelf::Singles,
                _ => Shelf::Other,
            };
        }
        // The account's own songs and albums of the artist.
        Some((id, _)) if id.starts_with("MPLA") => return Shelf::Other,
        _ => {}
    }
    let items = &shelf.items;
    let all = |f: fn(&HomeItem) -> bool| items.iter().all(f);
    if all(|i| matches!(i, HomeItem::Artist(_))) {
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
        let own = items.iter().any(|i| match i {
            HomeItem::Playlist(p) => p.author.as_deref().map(bare_name) == Some(bare_name(artist)),
            _ => false,
        });
        if own || target.is_some() {
            Shelf::Playlists
        } else {
            Shelf::FeaturedOn
        }
    } else if all(|i| matches!(i, HomeItem::Track(_))) {
        match target {
            Some((_, "MUSIC_PAGE_TYPE_ARTIST")) => Shelf::Live,
            _ => Shelf::Videos,
        }
    } else {
        Shelf::Other
    }
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
