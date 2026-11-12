//! Search

use featherplay_core::model::{
    AlbumId, ArtistId, LikeStatus, PlaylistId, Track, TrackId, VideoType,
};
use serde_json::Value;
use ytmapi_rs::common::YoutubeID;
use ytmapi_rs::parse::{
    SearchResultAlbum, SearchResultArtist, SearchResultPlaylist, SearchResultSong,
};
use ytmapi_rs::query::SearchQuery;
use ytmapi_rs::query::search::{
    AlbumsFilter, ArtistsFilter, FilteredSearch, PlaylistsFilter, SongsFilter,
};

use super::home::{card_author, watch_track};
use super::playlists::{flex_column, playlist_row, runs_of};
use super::radio::{browse_target, byline};
use super::{
    Extra, PLAY_WATCH_ENDPOINT, ROW_THUMBNAILS, album_ref, album_summary, bare_playlist_id, best,
    best_raw, check_error, json, log_left_out, merge_skipped, non_empty, parse_count,
    parse_duration, process_lenient, text,
};
use crate::{
    AlbumSummary, ApiError, ArtistSummary, HomeItem, PlaylistSummary, QuerySuggestion,
    SearchSuggestions, SuggestedResult,
};

/// A row ytmapi-rs can't read is read from the raw JSON (`search_song_row`): the Songs
/// filter also lists music videos, which have no album and no plays column.
pub fn search_songs(raw: &str) -> Result<Vec<Track>, ApiError> {
    if no_results(raw) {
        return Ok(Vec::new());
    }
    let extra = Extra::from_raw(raw)?;
    let q: SearchQuery<FilteredSearch<SongsFilter>> = "".into();
    let (songs, skipped): (Vec<SearchResultSong>, _) = process_lenient(raw, &q)?;
    let songs = songs
        .into_iter()
        .map(|s| {
            let id = s.video_id.get_raw().to_string();
            Track {
                artists: extra.artists_or(&id, &s.artist),
                album: s.album.map(album_ref),
                duration: parse_duration(&s.duration),
                video_type: extra.video_type_or(&id, VideoType::Song),
                thumbnail: best(&s.thumbnails),
                like: LikeStatus::Indifferent,
                counterpart: None,
                title: s.title,
                id: TrackId(id),
            }
        })
        .collect();
    Ok(merge_skipped(
        "search songs",
        songs,
        skipped,
        search_song_row,
    ))
}

/// A filtered search that found nothing: its page has no result shelf, only messages ("No
/// results for …"), which ytmapi-rs 0.3.3 can't read.
fn no_results(raw: &str) -> bool {
    let Ok(v) = serde_json::from_str::<Value>(raw) else {
        return false;
    };
    let sections = v
        .pointer("/contents/tabbedSearchResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents")
        .and_then(Value::as_array);
    sections.is_some_and(|s| {
        !s.is_empty()
            && s.iter().all(|section| {
                section
                    .pointer("/itemSectionRenderer/contents")
                    .and_then(Value::as_array)
                    .is_some_and(|items| items.iter().all(|i| i.get("messageRenderer").is_some()))
            })
    })
}

/// A song search row from the raw JSON: `playlist_row`, with the album and the length from
/// the byline (`Artists • Album • 3:20`, or `Artists • 1.2M views • 2:36` for a video).
fn search_song_row(item: &Value) -> Option<Track> {
    let mut track = playlist_row(item)?;
    let runs = runs_of(flex_column(item.get("musicResponsiveListItemRenderer")?, 1));
    if track.album.is_none() && track.video_type == VideoType::Song {
        track.album = byline(runs).1;
    }
    if track.duration.is_none() {
        track.duration = runs
            .last()
            .filter(|run| run.get("navigationEndpoint").is_none())
            .and_then(|run| run.get("text").and_then(Value::as_str))
            .and_then(parse_duration);
    }
    Some(track)
}

pub fn search_albums(raw: &str) -> Result<Vec<AlbumSummary>, ApiError> {
    if no_results(raw) {
        return Ok(Vec::new());
    }
    let extra = Extra::from_raw(raw)?;
    let q: SearchQuery<FilteredSearch<AlbumsFilter>> = "".into();
    let (albums, skipped): (Vec<SearchResultAlbum>, _) = process_lenient(raw, &q)?;
    log_left_out("search albums", &skipped);
    Ok(albums
        .into_iter()
        .map(|a| album_summary(a, &extra))
        .collect())
}

pub fn search_artists(raw: &str) -> Result<Vec<ArtistSummary>, ApiError> {
    if no_results(raw) {
        return Ok(Vec::new());
    }
    let q: SearchQuery<FilteredSearch<ArtistsFilter>> = "".into();
    let (artists, skipped): (Vec<SearchResultArtist>, _) = process_lenient(raw, &q)?;
    log_left_out("search artists", &skipped);
    Ok(artists
        .into_iter()
        .map(|a| ArtistSummary {
            id: ArtistId(a.browse_id.get_raw().to_string()),
            name: a.artist,
            thumbnail: best(&a.thumbnails),
        })
        .collect())
}

/// Podcasts in the results are dropped.
pub fn search_playlists(raw: &str) -> Result<Vec<PlaylistSummary>, ApiError> {
    if no_results(raw) {
        return Ok(Vec::new());
    }
    let q: SearchQuery<FilteredSearch<PlaylistsFilter>> = "".into();
    let (playlists, skipped): (Vec<SearchResultPlaylist>, _) = process_lenient(raw, &q)?;
    log_left_out("search playlists", &skipped);
    Ok(playlists
        .into_iter()
        .filter_map(|p| match p {
            SearchResultPlaylist::Featured(f) => Some(PlaylistSummary {
                id: PlaylistId(bare_playlist_id(f.playlist_id.get_raw()).to_string()),
                title: f.title,
                author: non_empty(f.author),
                count: parse_count(&f.songs),
                views: None,
                avatar: None,
                thumbnail: best(&f.thumbnails),
                owned: false,
            }),
            SearchResultPlaylist::Community(c) => Some(PlaylistSummary {
                id: PlaylistId(bare_playlist_id(c.playlist_id.get_raw()).to_string()),
                title: c.title,
                author: non_empty(c.author),
                count: None,
                views: None,
                avatar: None,
                thumbnail: best(&c.thumbnails),
                owned: false,
            }),
            _ => None,
        })
        .collect())
}

/// The suggestions while a query is typed (`music/get_search_suggestions`): a section of
/// completions (predictions, and signed in, past searches), then one of direct results.
/// Read from raw JSON: ytmapi-rs 0.3.3 parses the completions only. A row that opens
/// nothing featherplay shows (a podcast, an episode) is left out; a reply without sections
/// (a query too short to suggest for) has none.
pub fn search_suggestions(raw: &str) -> Result<SearchSuggestions, ApiError> {
    let v = json(raw)?;
    check_error(&v)?;
    let mut out = SearchSuggestions::default();
    let items = v
        .get("contents")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|s| s.pointer("/searchSuggestionsSectionRenderer/contents"))
        .filter_map(Value::as_array)
        .flatten();
    for item in items {
        if let Some(r) = item.get("searchSuggestionRenderer") {
            out.queries.extend(query_suggestion(r, false));
        } else if let Some(r) = item.get("historySuggestionRenderer") {
            out.queries.extend(query_suggestion(r, true));
        } else if let Some(r) = item.get("musicResponsiveListItemRenderer") {
            out.results.extend(suggested_result(r));
        }
    }
    Ok(out)
}

/// A completion: the query its search endpoint runs (else its runs' text), and how much of
/// it was typed (the leading bold runs).
fn query_suggestion(r: &Value, history: bool) -> Option<QuerySuggestion> {
    let runs = r
        .pointer("/suggestion/runs")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let shown: String = runs.iter().map(run_text).collect();
    let text = r
        .pointer("/navigationEndpoint/searchEndpoint/query")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or(shown);
    let typed: usize = runs
        .iter()
        .take_while(|run| run.get("bold").and_then(Value::as_bool) == Some(true))
        .map(|run| run_text(run).len())
        .sum();
    let typed = if text.is_char_boundary(typed) {
        typed
    } else {
        0
    };
    non_empty(text).map(|text| QuerySuggestion {
        typed: typed.min(text.len()),
        text,
        history,
    })
}

fn run_text(run: &Value) -> &str {
    run.get("text").and_then(Value::as_str).unwrap_or_default()
}

/// A direct result row: a song or video (a watch endpoint), or an album, artist or
/// playlist page. Its second column starts with the kind (`Song • Daft Punk • 111M
/// plays`), but for an artist (`68.2M monthly audience`).
fn suggested_result(r: &Value) -> Option<SuggestedResult> {
    let title = text(flex_column(r, 0)).unwrap_or_default();
    let subtitle = text(flex_column(r, 1)).unwrap_or_default();
    let runs = runs_of(flex_column(r, 1));
    // The byline past the kind word and its separator.
    let byline_runs = if runs.len() >= 3 && runs[0].get("navigationEndpoint").is_none() {
        &runs[2..]
    } else {
        runs
    };
    let thumbnail = best_raw(r.pointer(ROW_THUMBNAILS));
    let item = if let Some(w) = r
        .pointer("/navigationEndpoint/watchEndpoint")
        .or_else(|| r.pointer(PLAY_WATCH_ENDPOINT))
    {
        let (artists, mut album) = byline(byline_runs);
        if album.is_none() {
            album = byline(runs_of(flex_column(r, 2))).1;
        }
        HomeItem::Track(watch_track(w, title, artists, album, thumbnail)?)
    } else {
        let (id, page) = browse_target(r)?;
        match page {
            "MUSIC_PAGE_TYPE_ALBUM" | "MUSIC_PAGE_TYPE_AUDIOBOOK" => {
                HomeItem::Album(AlbumSummary {
                    id: AlbumId(id.to_string()),
                    title,
                    artists: byline(byline_runs).0,
                    year: None,
                    thumbnail,
                })
            }
            "MUSIC_PAGE_TYPE_ARTIST" => HomeItem::Artist(ArtistSummary {
                id: ArtistId(id.to_string()),
                name: title,
                thumbnail,
            }),
            "MUSIC_PAGE_TYPE_PLAYLIST" => HomeItem::Playlist(PlaylistSummary {
                id: PlaylistId(bare_playlist_id(id).to_string()),
                title,
                author: card_author(byline_runs),
                count: None,
                views: None,
                avatar: None,
                thumbnail,
                owned: false,
            }),
            _ => return None,
        }
    };
    Some(SuggestedResult { item, subtitle })
}
