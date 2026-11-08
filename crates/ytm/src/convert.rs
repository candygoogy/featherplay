//! Raw API responses to `featherplay_core::model` types.
//!
//! Each function takes the raw JSON of one response (or the pages of one
//! list), parses it with ytmapi-rs where its typed parser works, and fills in
//! what ytmapi-rs 0.3.3 leaves out (`musicVideoType`, linked artists,
//! thumbnails, song counterparts) from the same raw JSON. The watch playlist
//! is parsed from raw JSON only: the typed parser fails on items without a
//! video id and drops counterparts.
//!
//! ytmapi-rs fails a whole list on one item it can't read. Every list it parses goes through
//! `process_lenient`, which takes such an item out of the page and parses the page again; a
//! song row is then read from the raw JSON in its place, anything else left out.

use std::collections::HashMap;
use std::time::Duration;

use featherplay_core::model::{
    AlbumId, AlbumRef, ArtistId, ArtistRef, LikeStatus, PlaylistId, Track, TrackId, VideoType,
};
use serde_json::Value;
use ytmapi_rs::auth::BrowserToken;
use ytmapi_rs::common::{ArtistChannelID, LikeStatus as YtLike, PlaylistID, Thumbnail, YoutubeID};
use ytmapi_rs::continuations::ParseFromContinuable;
use ytmapi_rs::error::ErrorKind;
use ytmapi_rs::parse::{
    AlbumResult, GetArtist, GetPlaylistDetails, ParsedSongAlbum, ParsedSongArtist,
    SearchResultAlbum, SearchResultArtist, SearchResultPlaylist, SearchResultSong,
};
use ytmapi_rs::process_json;
use ytmapi_rs::query::search::{
    AlbumsFilter, ArtistsFilter, FilteredSearch, PlaylistsFilter, SongsFilter,
};
use ytmapi_rs::query::{
    GetArtistQuery, GetContinuationsQuery, GetLibraryArtistsQuery, GetPlaylistDetailsQuery,
    PostQuery, Query, SearchQuery,
};

use crate::{
    AccountIdentity, AccountInfo, Album, AlbumSummary, ApiError, Artist, ArtistSummary,
    ChartArtist, Charts, Country, HistoryDay, HomeFeed, HomeItem, HomeShelf, Mood, MoodPage,
    MoodSection, PlainLyrics, PlaylistSummary, QuerySuggestion, SearchSuggestions, SuggestedResult,
    TimedLine, TimedLyrics, Trend,
};

const WATCH_PANEL: &str = "/contents/singleColumnMusicWatchNextResultsRenderer/tabbedRenderer/watchNextTabbedResultsRenderer/tabs/0/tabRenderer/content/musicQueueRenderer/content/playlistPanelRenderer/contents";
const WATCH_TABS: &str = "/contents/singleColumnMusicWatchNextResultsRenderer/tabbedRenderer/watchNextTabbedResultsRenderer/tabs";
const MUSIC_VIDEO_TYPE: &str =
    "/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType";
const GRID_CONTINUATION: &str = "/continuationContents/gridContinuation";
const SHELF_CONTINUATION: &str = "/continuationContents/musicShelfContinuation";
const HOME_CONTINUATION: &str = "/continuationContents/sectionListContinuation";
const PLAYLIST_HEADER_ITEM: &str = "/contents/twoColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/0";
const PLAYLIST_COLUMNS: &str = "/contents/twoColumnBrowseResultsRenderer";
const PLAYLIST_SHELF: &str = "/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents/0/musicPlaylistShelfRenderer";
const PLAYLIST_SHELF_ROWS: &str = "/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents/0/musicPlaylistShelfRenderer/contents";
const APPENDED_ROWS: &str =
    "/onResponseReceivedActions/0/appendContinuationItemsAction/continuationItems";
const CONTINUATION_TOKEN: &str =
    "/continuationItemRenderer/continuationEndpoint/continuationCommand/token";
const PLAY_WATCH_ENDPOINT: &str = "/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchEndpoint";
const ROW_THUMBNAILS: &str = "/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails";
const GREY_OUT: &str = "MUSIC_ITEM_RENDERER_DISPLAY_POLICY_GREY_OUT";
const ALBUM_HEADER: &str = "/contents/twoColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/0/musicResponsiveHeaderRenderer";
const ALBUM_ROWS: &str = "/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents/0/musicShelfRenderer/contents";
const ACCOUNT_HEADER: &str =
    "/actions/0/openPopupAction/popup/multiPageMenuRenderer/header/activeAccountHeaderRenderer";
const SWITCHER_SECTIONS: &str =
    "/actions/0/getMultiPageMenuAction/menu/multiPageMenuRenderer/sections";
const PAGE_TYPE: &str =
    "/browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType";

// ---------------------------------------------------------------------------
// Lyrics

/// The browse id of the Lyrics tab in a watch-next response, or `None` when the track
/// has no lyrics (the tab is there but unselectable, without an endpoint).
pub fn lyrics_browse_id(raw: &str) -> Result<Option<String>, ApiError> {
    let v = json(raw)?;
    check_error(&v)?;
    let tabs = v
        .pointer(WATCH_TABS)
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::Parse(format!("no watch tabs in {}", layout(&v))))?;
    // Found by its browse id's prefix rather than its place: the tabs' order varies.
    Ok(tabs.iter().find_map(|t| {
        t.pointer("/tabRenderer/endpoint/browseEndpoint/browseId")
            .and_then(Value::as_str)
            .filter(|id| id.starts_with("MPLY"))
            .map(str::to_string)
    }))
}

/// The plain lyrics on a Lyrics tab's browse page, or `None` when it says there are none.
pub fn lyrics(raw: &str) -> Result<Option<PlainLyrics>, ApiError> {
    let v = json(raw)?;
    check_error(&v)?;
    let shelf = v
        .pointer("/contents/sectionListRenderer/contents")
        .and_then(Value::as_array)
        .and_then(|items| {
            items
                .iter()
                .find_map(|i| i.get("musicDescriptionShelfRenderer"))
        });
    let Some(shelf) = shelf else {
        // "Lyrics not available" (a `messageRenderer`), or an empty page.
        return Ok(None);
    };
    let found = text(shelf.get("description"))
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
        .map(|words| PlainLyrics {
            text: words,
            source: text(shelf.get("footer")).filter(|s| !s.trim().is_empty()),
        });
    Ok(found)
}

/// Where the mobile client's Lyrics tab page keeps its lines, as ytmusicapi reads them.
const TIMED_LYRICS: &str =
    "/contents/elementRenderer/newElement/type/componentType/model/timedLyricsModel/lyricsData";

/// The timed lyrics on a Lyrics tab's page as the mobile client gets it, sorted by start, or
/// `None` when they aren't timed. Most songs come untimed (the same lines, without a
/// `cueRange`); as in ytmusicapi, one untimed line makes them all untimed.
pub fn timed_lyrics(raw: &str) -> Result<Option<TimedLyrics>, ApiError> {
    let v = json(raw)?;
    check_error(&v)?;
    let Some(data) = v.pointer(TIMED_LYRICS) else {
        return Ok(None);
    };
    let Some(raw_lines) = data.get("timedLyricsData").and_then(Value::as_array) else {
        return Ok(None);
    };
    let lines: Option<Vec<TimedLine>> = raw_lines.iter().map(timed_line).collect();
    let Some(mut lines) = lines.filter(|l| !l.is_empty()) else {
        return Ok(None);
    };
    // Stable, so lines sharing a start keep the page's order.
    lines.sort_by_key(|l| l.start);
    let source = data
        .get("sourceMessage")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Ok(Some(TimedLyrics { lines, source }))
}

/// One `timedLyricsData` entry, or `None` without a start time.
fn timed_line(v: &Value) -> Option<TimedLine> {
    let start = v
        .pointer("/cueRange/startTimeMilliseconds")
        .and_then(Value::as_str)?
        .parse()
        .ok()?;
    Some(TimedLine {
        start: Duration::from_millis(start),
        text: v
            .get("lyricLine")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string(),
    })
}

// ---------------------------------------------------------------------------
// Search

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

// ---------------------------------------------------------------------------
// Playlists (including liked songs) and library

/// Tracks from the first page and its continuations, read from raw JSON one row at a
/// time. ytmapi-rs 0.3.3 fails the whole playlist on one row it can't read, such as a
/// collaborative playlist's rows, which have their duration in a flex column and no fixed
/// column. Unavailable (greyed out, deleted) rows and uploads are dropped; a row without
/// a duration is kept without one.
///
/// A first page without a track shelf is an empty playlist. A continuation page that
/// isn't a list of rows is a parse error.
pub fn playlist_tracks(pages: &[String]) -> Result<Vec<Track>, ApiError> {
    if pages.first().is_some_and(|raw| unavailable(raw)) {
        return Err(ApiError::NotFound);
    }
    let mut out = Vec::new();
    for (i, raw) in pages.iter().enumerate() {
        let v = json(raw)?;
        check_error(&v)?;
        let rows = match playlist_rows(&v) {
            Some(rows) => rows,
            None if i == 0 && v.pointer(PLAYLIST_COLUMNS).is_some() => &[],
            None => {
                return Err(ApiError::Parse(format!(
                    "playlist page has no rows ({})",
                    layout(&v)
                )));
            }
        };
        out.extend(rows.iter().filter_map(playlist_row));
    }
    Ok(out)
}

/// The next-page token of a playlist page (first page or continuation): the last row's
/// continuation item.
pub fn playlist_continuation(raw: &str) -> Option<String> {
    let v: Value = serde_json::from_str(raw).ok()?;
    playlist_rows(&v)?
        .last()?
        .pointer(CONTINUATION_TOKEN)
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

/// A first page's track shelf rows, or the rows a continuation page appends.
fn playlist_rows(v: &Value) -> Option<&[Value]> {
    v.pointer(PLAYLIST_SHELF_ROWS)
        .or_else(|| v.pointer(APPENDED_ROWS))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
}

/// One playlist row: a song, video or episode. `None` for anything else (the
/// continuation item), and for rows that can't play.
fn playlist_row(item: &Value) -> Option<Track> {
    let r = item.get("musicResponsiveListItemRenderer")?;
    if greyed_out(r) {
        return None;
    }
    let title = first_run_text(flex_column(r, 0))?;
    if title == "Song deleted" {
        return None;
    }
    let watch = r.pointer(PLAY_WATCH_ENDPOINT);
    let id = watch
        .and_then(|w| w.get("videoId"))
        .or_else(|| r.pointer("/playlistItemData/videoId"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?;
    let kind = watch
        .and_then(|w| w.pointer(MUSIC_VIDEO_TYPE))
        .and_then(Value::as_str)
        .unwrap_or_default();
    // Uploads play from the owner's locker, which the stream resolver can't reach.
    if kind.ends_with("PRIVATELY_OWNED_TRACK") {
        return None;
    }
    let video_type = video_type(kind);
    let (mut artists, _) = byline(runs_of(flex_column(r, 1)));
    if artists.is_empty()
        && let Some(name) = first_run_text(flex_column(r, 1)).and_then(non_empty)
    {
        // A podcast episode names its show, which links to a podcast page.
        artists.push(ArtistRef { id: None, name });
    }
    let album = match video_type {
        VideoType::Song => flex_columns(r).skip(2).find_map(|c| {
            runs_of(Some(c)).iter().find_map(|run| {
                let (id, page) = browse_target(run)?;
                (page == "MUSIC_PAGE_TYPE_ALBUM" || id.starts_with("MPRE")).then(|| AlbumRef {
                    id: AlbumId(id.to_string()),
                    title: run
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string(),
                })
            })
        }),
        _ => None,
    };
    Some(Track {
        id: TrackId(id.to_string()),
        title,
        artists,
        album,
        duration: row_duration(r),
        video_type,
        thumbnail: best_raw(r.pointer(ROW_THUMBNAILS)),
        like: row_like(r),
        counterpart: None,
    })
}

/// The fixed column's length, else a flex column after the artists that reads as one
/// and links nowhere (collaborative playlists).
fn row_duration(r: &Value) -> Option<Duration> {
    let fixed = r
        .pointer("/fixedColumns/0/musicResponsiveListItemFixedColumnRenderer/text")
        .and_then(|t| text(Some(t)));
    fixed.as_deref().and_then(parse_duration).or_else(|| {
        flex_columns(r)
            .skip(2)
            .find_map(|c| match runs_of(Some(c)) {
                [run] if run.get("navigationEndpoint").is_none() => run
                    .get("text")
                    .and_then(Value::as_str)
                    .and_then(parse_duration),
                _ => None,
            })
    })
}

fn row_like(r: &Value) -> LikeStatus {
    let status = r
        .pointer("/menu/menuRenderer/topLevelButtons")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .find_map(|b| b.pointer("/likeButtonRenderer/likeStatus"))
        .and_then(Value::as_str);
    match status {
        Some("LIKE") => LikeStatus::Like,
        Some("DISLIKE") => LikeStatus::Dislike,
        _ => LikeStatus::Indifferent,
    }
}

fn greyed_out(r: &Value) -> bool {
    r.get("musicItemRendererDisplayPolicy")
        .and_then(Value::as_str)
        == Some(GREY_OUT)
}

/// A list row's flex column `i`: its `text`.
fn flex_column(r: &Value, i: usize) -> Option<&Value> {
    r.pointer(&format!(
        "/flexColumns/{i}/musicResponsiveListItemFlexColumnRenderer/text"
    ))
}

/// Every flex column's `text`, in order.
fn flex_columns(r: &Value) -> impl Iterator<Item = &Value> {
    r.get("flexColumns")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|c| c.pointer("/musicResponsiveListItemFlexColumnRenderer/text"))
}

fn runs_of(t: Option<&Value>) -> &[Value] {
    t.and_then(|t| t.get("runs"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

fn first_run_text(t: Option<&Value>) -> Option<String> {
    runs_of(t)
        .first()?
        .get("text")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// The header of a playlist's first page. `id` may carry the `VL` prefix.
///
/// ytmapi-rs 0.3.3 needs an author (strapline or facepile), which auto
/// playlists like Liked Music (`LM`) don't have. A header it can't parse is
/// read from the raw JSON instead.
pub fn playlist_summary(first_page: &str, id: &PlaylistId) -> Result<PlaylistSummary, ApiError> {
    if unavailable(first_page) {
        return Err(ApiError::NotFound);
    }
    let id = PlaylistId(bare_playlist_id(&id.0).to_string());
    let q = GetPlaylistDetailsQuery::new(PlaylistID::from_raw(""));
    let parsed: Result<GetPlaylistDetails, _> = process(first_page, &q);
    match parsed {
        Ok(d) => Ok(PlaylistSummary {
            id,
            title: d.title,
            author: non_empty(d.author),
            count: parse_count(&d.track_count_text),
            views: None,
            avatar: None,
            thumbnail: best(&d.thumbnails),
            owned: false,
        }),
        Err(ApiError::Parse(e)) => raw_playlist_header(first_page, id.clone())
            .or_else(|| album_playlist_header(first_page, id))
            .ok_or(ApiError::Parse(e)),
        Err(e) => Err(e),
    }
}

/// The header ytmapi-rs reads (plain, or inside the editable header of an
/// owned playlist), with the author optional. `None` without a title.
fn raw_playlist_header(raw: &str, id: PlaylistId) -> Option<PlaylistSummary> {
    let v: Value = serde_json::from_str(raw).ok()?;
    let item = v.pointer(PLAYLIST_HEADER_ITEM)?;
    let h = item.get("musicResponsiveHeaderRenderer").or_else(|| {
        item.pointer(
            "/musicEditablePlaylistDetailHeaderRenderer/header/musicResponsiveHeaderRenderer",
        )
    })?;
    let title = text(h.get("title")).and_then(non_empty)?;
    let author = text(h.get("straplineTextOne"))
        .or_else(|| {
            h.pointer("/facepile/avatarStackViewModel/text/content")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .and_then(non_empty);
    // `[views •] 41 songs • 2 hours`: the count is third from the end, as
    // ytmapi-rs reads it.
    let count = h
        .pointer("/secondSubtitle/runs")
        .and_then(Value::as_array)
        .and_then(|runs| runs.iter().rev().nth(2))
        .and_then(|run| run.get("text").and_then(Value::as_str))
        .and_then(parse_count);
    Some(PlaylistSummary {
        id,
        title,
        author,
        count,
        views: None,
        avatar: None,
        thumbnail: best_raw(h.pointer("/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails")),
        owned: false,
    })
}

/// An album's own playlist (`OLAK5uy_…`, as a pasted link opens it) has no header, only
/// rows: its title, artists and cover are its album's, read from the rows. `None` unless
/// every row with an album is from the same one.
fn album_playlist_header(raw: &str, id: PlaylistId) -> Option<PlaylistSummary> {
    let v: Value = serde_json::from_str(raw).ok()?;
    let tracks: Vec<Track> = playlist_rows(&v)?.iter().filter_map(playlist_row).collect();
    let first = tracks.first()?;
    let album = first.album.as_ref()?;
    if tracks
        .iter()
        .filter_map(|t| t.album.as_ref())
        .any(|a| a.id != album.id)
    {
        return None;
    }
    let artists: Vec<&str> = first.artists.iter().map(|a| a.name.as_str()).collect();
    let count = v
        .pointer(PLAYLIST_SHELF)
        .and_then(|shelf| shelf.get("collapsedItemCount"))
        .and_then(Value::as_u64)
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(tracks.len() as u32);
    Some(PlaylistSummary {
        id,
        title: album.title.clone(),
        author: non_empty(artists.join(", ")),
        count: Some(count),
        views: None,
        avatar: None,
        thumbnail: first.thumbnail.clone(),
        owned: false,
    })
}

/// Library playlists from raw JSON (first page, then grid continuations).
/// Liked Music and Episodes for Later are left out: they have their own views.
pub fn library_playlists(pages: &[String]) -> Result<Vec<PlaylistSummary>, ApiError> {
    grid_items(pages, library_playlist)
}

/// Library albums from raw JSON (first page, then grid continuations).
pub fn library_albums(pages: &[String]) -> Result<Vec<AlbumSummary>, ApiError> {
    grid_items(pages, library_album)
}

/// Converts the items of every grid page. A first page without a grid is an
/// empty library.
fn grid_items<T>(pages: &[String], f: impl Fn(&Value) -> Option<T>) -> Result<Vec<T>, ApiError> {
    let mut out = Vec::new();
    for (i, raw) in pages.iter().enumerate() {
        let v = json(raw)?;
        check_error(&v)?;
        let grid = if i == 0 {
            first_grid(&v)
        } else {
            v.pointer(GRID_CONTINUATION)
        };
        let Some(grid) = grid else {
            if i == 0 {
                continue;
            }
            return Err(ApiError::Parse("library continuation has no grid".into()));
        };
        let items = grid
            .get("items")
            .and_then(Value::as_array)
            .into_iter()
            .flatten();
        out.extend(items.filter_map(&f));
    }
    Ok(out)
}

/// The grid's next-page token, from a first page or a continuation page.
pub fn grid_continuation(raw: &str) -> Option<String> {
    let v: Value = serde_json::from_str(raw).ok()?;
    let grid = v.pointer(GRID_CONTINUATION).or_else(|| first_grid(&v))?;
    grid.pointer("/continuations/0/nextContinuationData/continuation")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn first_grid(v: &Value) -> Option<&Value> {
    first_in_section(v, "gridRenderer")
}

/// The first `renderer` in a library page's section list.
fn first_in_section<'v>(v: &'v Value, renderer: &str) -> Option<&'v Value> {
    let section = v.pointer(
        "/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer",
    )?;
    let mut found = None;
    walk(section, &mut |k, val| {
        if found.is_none() && k == renderer {
            found = Some(val);
        }
    });
    found
}

/// Subtitles vary: `Author • 12 tracks`, `12 tracks`, `Auto playlist`, ...
fn library_playlist(item: &Value) -> Option<PlaylistSummary> {
    let r = item.get("musicTwoRowItemRenderer")?;
    let browse = r
        .pointer("/title/runs/0/navigationEndpoint/browseEndpoint/browseId")
        .and_then(Value::as_str)?;
    let id = browse.strip_prefix("VL")?;
    if id == "LM" || id == "SE" {
        return None;
    }
    let runs = r
        .pointer("/subtitle/runs")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let values: Vec<&Value> = runs.iter().step_by(2).collect();
    let run_text = |run: &&Value| run.get("text").and_then(Value::as_str).map(str::to_string);
    let starts_with_digit = |t: &String| t.chars().next().is_some_and(|c| c.is_ascii_digit());
    let count = values
        .iter()
        .filter_map(run_text)
        .find(starts_with_digit)
        .and_then(|t| parse_count(&t));
    let author = values
        .iter()
        .find(|run| browse_target(run).is_some())
        .or_else(|| (values.len() == 2).then(|| values.first()).flatten())
        .and_then(run_text)
        .filter(|t| !starts_with_digit(t))
        .and_then(non_empty);
    Some(PlaylistSummary {
        id: PlaylistId(id.to_string()),
        title: text(r.get("title")).unwrap_or_default(),
        author,
        count,
        views: None,
        avatar: None,
        thumbnail: best_raw(
            r.pointer("/thumbnailRenderer/musicThumbnailRenderer/thumbnail/thumbnails"),
        ),
        owned: !has_library_toggle(r),
    })
}

/// The item's menu saves the playlist to the library or removes it from there ("Remove
/// playlist from library"): someone else's playlist. The account's own have no such toggle
/// (their menu deletes them instead).
fn has_library_toggle(r: &Value) -> bool {
    let items = r
        .pointer("/menu/menuRenderer/items")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    items.iter().any(|item| {
        let Some(toggle) = item.get("toggleMenuServiceItemRenderer") else {
            return false;
        };
        ["defaultServiceEndpoint", "toggledServiceEndpoint"]
            .iter()
            .any(|ep| {
                toggle
                    .pointer(&format!("/{ep}/likeEndpoint/target/playlistId"))
                    .is_some()
            })
    })
}

/// Library artists from raw JSON (first page, then shelf continuations). A row ytmapi-rs
/// can't read is left out.
pub fn library_artists(pages: &[String]) -> Result<Vec<ArtistSummary>, ApiError> {
    let mut out = Vec::new();
    for (items, extra) in parse_pages(pages, &GetLibraryArtistsQuery::default(), "library artists")?
    {
        out.extend(items.into_iter().map(|a| {
            let id = a.channel_id.get_raw().to_string();
            ArtistSummary {
                thumbnail: extra.rows.get(&id).and_then(|r| r.thumbnail.clone()),
                id: ArtistId(id),
                name: a.artist,
            }
        }));
    }
    Ok(out)
}

/// The next-page token of a library shelf (the artists), from a first page or a continuation
/// page. Read from the raw page: ytmapi-rs's stream reads it with its typed parser, so one row
/// it can't read would end the list there.
pub fn shelf_continuation(raw: &str) -> Option<String> {
    let v: Value = serde_json::from_str(raw).ok()?;
    let shelf = v
        .pointer(SHELF_CONTINUATION)
        .or_else(|| first_in_section(&v, "musicShelfRenderer"))?;
    shelf
        .pointer("/continuations/0/nextContinuationData/continuation")
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

/// Subtitle `Album • Artist • 1997`; the type word comes first.
fn library_album(item: &Value) -> Option<AlbumSummary> {
    let r = item.get("musicTwoRowItemRenderer")?;
    let id = r
        .pointer("/title/runs/0/navigationEndpoint/browseEndpoint/browseId")
        .or_else(|| r.pointer("/navigationEndpoint/browseEndpoint/browseId"))
        .and_then(Value::as_str)
        .filter(|id| id.starts_with("MPRE"))?;
    let runs = r
        .pointer("/subtitle/runs")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let values: Vec<&Value> = runs.iter().step_by(2).collect();
    let run_text = |run: &Value| {
        run.get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string()
    };
    let is_year = |t: &str| t.len() == 4 && t.chars().all(|c| c.is_ascii_digit());
    let year = values.iter().map(|r| run_text(r)).find(|t| is_year(t));
    let mut artists: Vec<ArtistRef> = values
        .iter()
        .filter_map(|run| {
            let (id, page) = browse_target(run)?;
            (page == "MUSIC_PAGE_TYPE_ARTIST" || id.starts_with("UC")).then(|| ArtistRef {
                id: Some(ArtistId(id.to_string())),
                name: run_text(run),
            })
        })
        .collect();
    if artists.is_empty() {
        artists = values
            .iter()
            .skip(1)
            .map(|r| run_text(r))
            .filter(|t| !t.trim().is_empty() && !is_year(t))
            .map(|name| ArtistRef { id: None, name })
            .collect();
    }
    Some(AlbumSummary {
        id: AlbumId(id.to_string()),
        title: text(r.get("title")).unwrap_or_default(),
        artists,
        year,
        thumbnail: best_raw(
            r.pointer("/thumbnailRenderer/musicThumbnailRenderer/thumbnail/thumbnails"),
        ),
    })
}

// ---------------------------------------------------------------------------
// Album and artist pages

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
    let releases_of = |r: Option<ytmapi_rs::parse::GetArtistAlbums>| -> Vec<AlbumSummary> {
        r.map(|r| r.results)
            .unwrap_or_default()
            .into_iter()
            .map(|a: AlbumResult| AlbumSummary {
                id: AlbumId(a.album_id.get_raw().to_string()),
                title: a.title,
                artists: vec![me.clone()],
                year: non_empty(a.year),
                thumbnail: best(&a.thumbnails),
            })
            .collect()
    };
    Ok(Artist {
        albums: releases_of(releases.albums),
        singles: releases_of(releases.singles),
        summary,
        top_songs,
    })
}

// ---------------------------------------------------------------------------
// Watch playlist (radio)

/// Tracks of the watch playlist. Items without a video id (automix
/// previews and the like) are skipped. A music video whose counterpart is a
/// song gets that song as its `counterpart` (its song version).
pub fn radio(raw: &str) -> Result<Vec<Track>, ApiError> {
    let v = json(raw)?;
    check_error(&v)?;
    let items = v
        .pointer(WATCH_PANEL)
        .and_then(Value::as_array)
        .ok_or_else(|| ApiError::Parse("watch playlist has no queue".into()))?;
    Ok(items.iter().filter_map(watch_item).collect())
}

/// The `seed` track itself from its watch playlist: what a pasted link shows. `None` when
/// the queue doesn't hold it, or there's no queue at all (an id that doesn't play).
pub fn seed_track(raw: &str, seed: &TrackId) -> Result<Option<Track>, ApiError> {
    let v = json(raw)?;
    check_error(&v)?;
    let Some(items) = v.pointer(WATCH_PANEL).and_then(Value::as_array) else {
        return Ok(None);
    };
    Ok(items.iter().filter_map(watch_item).find(|t| t.id == *seed))
}

fn watch_item(item: &Value) -> Option<Track> {
    let (primary, counterpart) = match item.get("playlistPanelVideoWrapperRenderer") {
        Some(w) => (
            w.pointer("/primaryRenderer/playlistPanelVideoRenderer")?,
            w.pointer("/counterpart/0/counterpartRenderer/playlistPanelVideoRenderer"),
        ),
        None => (item.get("playlistPanelVideoRenderer")?, None),
    };
    let mut track = panel_track(primary)?;
    if track.video_type == VideoType::MusicVideo
        && let Some(mut song) = counterpart.and_then(panel_track)
        && song.video_type == VideoType::Song
    {
        // The counterpart's byline names its artists without links; the video's has them.
        for a in song.artists.iter_mut().filter(|a| a.id.is_none()) {
            a.id = track
                .artists
                .iter()
                .find(|v| v.name == a.name)
                .and_then(|v| v.id.clone());
        }
        track.counterpart = Some(Box::new(song));
    }
    Some(track)
}

fn panel_track(r: &Value) -> Option<Track> {
    let id = r
        .get("videoId")
        .or_else(|| r.pointer("/navigationEndpoint/watchEndpoint/videoId"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?;
    let (artists, album) = byline(
        r.pointer("/longBylineText/runs")
            .or_else(|| r.pointer("/shortBylineText/runs"))
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default(),
    );
    let video_type = r
        .pointer("/navigationEndpoint/watchEndpoint")
        .and_then(|w| w.pointer(MUSIC_VIDEO_TYPE))
        .and_then(Value::as_str)
        .map(video_type)
        .unwrap_or(VideoType::Other);
    Some(Track {
        id: TrackId(id.to_string()),
        title: text(r.get("title")).unwrap_or_default(),
        artists,
        album,
        duration: text(r.get("lengthText"))
            .as_deref()
            .and_then(parse_duration),
        video_type,
        thumbnail: best_raw(r.pointer("/thumbnail/thumbnails")),
        // The watch playlist carries no rating.
        like: LikeStatus::Indifferent,
        counterpart: None,
    })
}

/// Artists and album from byline runs such as
/// `Artist • Album • 1997` or `Artist & Other • 139M views • 1.1M likes`.
/// Even runs are values, odd runs are separators. Linked runs are typed by
/// their browse page; unlinked runs count as artists only before the first
/// `•`.
fn byline(runs: &[Value]) -> (Vec<ArtistRef>, Option<AlbumRef>) {
    let mut artists = Vec::new();
    let mut album = None;
    let mut group = 0;
    for (i, run) in runs.iter().enumerate() {
        let t = run.get("text").and_then(Value::as_str).unwrap_or_default();
        if i % 2 == 1 {
            if t.trim() == "•" {
                group += 1;
            }
            continue;
        }
        match browse_target(run) {
            Some((id, page)) if page == "MUSIC_PAGE_TYPE_ALBUM" || id.starts_with("MPRE") => {
                album = Some(AlbumRef {
                    id: AlbumId(id.to_string()),
                    title: t.to_string(),
                });
            }
            Some((id, page))
                if page == "MUSIC_PAGE_TYPE_ARTIST"
                    || page == "MUSIC_PAGE_TYPE_USER_CHANNEL"
                    || id.starts_with("UC") =>
            {
                artists.push(ArtistRef {
                    id: Some(ArtistId(id.to_string())),
                    name: t.to_string(),
                });
            }
            Some(_) => {}
            None if group == 0 && !t.trim().is_empty() => artists.push(ArtistRef {
                id: None,
                name: t.to_string(),
            }),
            None => {}
        }
    }
    (artists, album)
}

/// (browseId, pageType) of a run's browse endpoint.
fn browse_target(run: &Value) -> Option<(&str, &str)> {
    let b = run.pointer("/navigationEndpoint/browseEndpoint")?;
    let id = b.get("browseId").and_then(Value::as_str)?;
    let page = b
        .pointer(PAGE_TYPE)
        .and_then(Value::as_str)
        .unwrap_or_default();
    Some((id, page))
}

// ---------------------------------------------------------------------------
// Account

/// The account menu's header: name, handle and the widest photo. A signed-out
/// response is `AuthExpired`; an empty name is a parse error.
pub fn account_info(raw: &str) -> Result<AccountInfo, ApiError> {
    let v = json(raw)?;
    check_error(&v)?;
    let header = v.pointer(ACCOUNT_HEADER).or_else(|| {
        let mut found = None;
        walk(&v, &mut |key, val| {
            if found.is_none() && key == "activeAccountHeaderRenderer" {
                found = Some(val);
            }
        });
        found
    });
    let Some(header) = header else {
        return Err(match logged_in(raw) {
            Some(false) => ApiError::AuthExpired,
            _ => ApiError::Parse("account menu has no account header".into()),
        });
    };
    let name = text(header.get("accountName"))
        .and_then(non_empty)
        .ok_or_else(|| ApiError::Parse("account menu has no account name".into()))?;
    Ok(AccountInfo {
        name,
        handle: text(header.get("channelHandle")).and_then(non_empty),
        photo: best_raw(header.pointer("/accountPhoto/thumbnails")),
    })
}

/// The account switcher (`getAccountSwitcherEndpoint`): every identity it lists, in its
/// order, once each. The Google account index comes from each item's sign-in URL
/// (`authuser`), the brand account from its `pageIdToken`. `current` is the identity the
/// request was sent as: it is the active one when listed, else the server's selection.
/// Disabled items are left out. A signed-out response is `AuthExpired`.
pub fn accounts(raw: &str, current: &crate::Identity) -> Result<Vec<AccountIdentity>, ApiError> {
    // The endpoint answers with the anti-JSON-hijacking prefix `)]}'` on its own line.
    let body = raw.trim_start();
    let body = match body.strip_prefix(")]}'") {
        Some(rest) => rest,
        None => body,
    };
    let v = json(body)?;
    check_error(&v)?;
    let data = v.get("data").unwrap_or(&v);
    let logged_out = data
        .pointer("/responseContext/mainAppWebResponseContext/loggedOut")
        .and_then(Value::as_bool);
    if logged_out == Some(true) {
        return Err(ApiError::AuthExpired);
    }
    let mut sections: Vec<&Value> = Vec::new();
    match data.pointer(SWITCHER_SECTIONS).and_then(Value::as_array) {
        Some(list) => sections.extend(
            list.iter()
                .filter_map(|s| s.get("accountSectionListRenderer")),
        ),
        None => walk(data, &mut |key, val| {
            if key == "accountSectionListRenderer" {
                sections.push(val);
            }
        }),
    }
    let mut out: Vec<AccountIdentity> = Vec::new();
    let mut selected = Vec::new();
    for section in sections {
        let section_email =
            text(section.pointer("/header/googleAccountHeaderRenderer/email")).and_then(non_empty);
        let groups = section.get("contents").and_then(Value::as_array);
        for group in groups.into_iter().flatten() {
            let Some(group) = group.get("accountItemSectionRenderer") else {
                continue;
            };
            let email = text(group.pointer("/header/accountItemSectionHeaderRenderer/title"))
                .and_then(non_empty)
                .or_else(|| section_email.clone());
            let items = group.get("contents").and_then(Value::as_array);
            for item in items.into_iter().flatten() {
                let Some(item) = item.get("accountItem") else {
                    continue;
                };
                let flag = |k: &str| item.get(k).and_then(Value::as_bool).unwrap_or(false);
                if flag("isDisabled") {
                    continue;
                }
                let Some(name) = text(item.get("accountName")).and_then(non_empty) else {
                    continue;
                };
                let is_selected = flag("isSelected");
                let Some(identity) = switcher_identity(item, is_selected) else {
                    continue;
                };
                if let Some(seen) = out.iter().position(|a| a.identity == identity) {
                    if is_selected {
                        selected.push(seen);
                    }
                    continue;
                }
                if is_selected {
                    selected.push(out.len());
                }
                out.push(AccountIdentity {
                    name,
                    handle: text(item.get("channelHandle")).and_then(non_empty),
                    byline: text(item.get("accountByline")).and_then(non_empty),
                    has_channel: flag("hasChannel"),
                    photo: best_raw(item.pointer("/accountPhoto/thumbnails")),
                    account: email.clone(),
                    active: false,
                    identity,
                });
            }
        }
    }
    if out.is_empty() {
        return Err(ApiError::Parse("account switcher lists no accounts".into()));
    }
    let explicit = (!current.is_default())
        .then(|| out.iter().position(|a| &a.identity == current))
        .flatten();
    match explicit {
        Some(i) => out[i].active = true,
        None => {
            for i in selected {
                out[i].active = true;
            }
        }
    }
    Ok(out)
}

/// An item's identity: `authuser` from its sign-in URL, the page id from its
/// `pageIdToken` (or the URL's `pageid`). Without an index, only the server's own
/// selection is usable (as the default identity).
fn switcher_identity(item: &Value, selected: bool) -> Option<crate::Identity> {
    let tokens = item
        .pointer("/serviceEndpoint/selectActiveIdentityEndpoint/supportedTokens")
        .and_then(Value::as_array);
    let mut page_id = None;
    let mut signin = None;
    for t in tokens.into_iter().flatten() {
        if let Some(p) = t.pointer("/pageIdToken/pageId").and_then(Value::as_str) {
            page_id = Some(p.to_string());
        }
        if let Some(u) = t
            .pointer("/accountSigninToken/signinUrl")
            .and_then(Value::as_str)
        {
            signin = Some(u);
        }
    }
    let page_id = page_id.or_else(|| signin.and_then(|u| query_param(u, "pageid")));
    match signin
        .and_then(|u| query_param(u, "authuser"))
        .and_then(|u| u.parse::<u32>().ok())
    {
        Some(user) => crate::Identity::new(user, page_id.as_deref()),
        None if selected && page_id.is_none() => Some(crate::Identity::default()),
        None => None,
    }
}

/// The first `name=value` in a URL's query string, undecoded.
fn query_param(url: &str, name: &str) -> Option<String> {
    let query = url.split_once('?')?.1;
    let query = query.split('#').next().unwrap_or_default();
    query
        .split('&')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_string())
}

// ---------------------------------------------------------------------------
// Home feed

/// Shelves of the Home feed: the first page, then any continuation pages.
/// Mood chips, unknown shelf kinds, shelves that don't read, items that map to nothing
/// (podcast episodes, user channels) and shelves left empty are dropped, and so is "Music
/// videos for you" (`music_videos_only`).
///
/// Only the first page can fail, and only when it isn't a Home answer at all (not JSON,
/// an API error, or a layout we don't know). A first page without shelves (an identity
/// with no history yet) is an empty Home. A continuation that doesn't read loses only its
/// own shelves.
pub fn home(pages: &[String]) -> Result<Vec<HomeShelf>, ApiError> {
    let Some((first, rest)) = pages.split_first() else {
        return Ok(Vec::new());
    };
    let mut out = feed_page(first)?;
    for raw in rest {
        if let Ok(more) = feed_page(raw) {
            out.extend(more);
        }
    }
    Ok(out)
}

/// The feed from its first pages (`home`), with the token of the page after the last.
pub fn home_feed(pages: &[String]) -> Result<HomeFeed, ApiError> {
    Ok(HomeFeed {
        shelves: home(pages)?,
        more: pages.last().and_then(|p| home_continuation(p)),
    })
}

/// One later page of the feed (`YtMusic::home_more`) and the token after it. Unlike the
/// continuation `home` reads with the first page, a page that doesn't read is an error, so
/// Home can offer to try it again.
pub fn home_more(raw: &str) -> Result<HomeFeed, ApiError> {
    Ok(HomeFeed {
        shelves: feed_page(raw)?,
        more: home_continuation(raw),
    })
}

/// A page of the Home feed: its shelves (`home_page`) but "Music videos for you".
fn feed_page(raw: &str) -> Result<Vec<HomeShelf>, ApiError> {
    shelves_page(raw, |shelf| !music_videos_only(shelf))
}

/// Home's "Music videos for you": every card a music video on a 16:9 thumbnail. It's built
/// around the picture, and featherplay plays only the sound (intros, skits and effects
/// included). Told by its cards, not its title, which follows the UI language; a genre's
/// shelf with a music video among other cards stays.
fn music_videos_only(shelf: &Value) -> bool {
    let Some(cards) = shelf
        .get("musicCarouselShelfRenderer")
        .and_then(|r| r.get("contents"))
        .and_then(Value::as_array)
        .filter(|c| !c.is_empty())
    else {
        return false;
    };
    cards.iter().all(|card| {
        let Some(r) = card.get("musicTwoRowItemRenderer") else {
            return false;
        };
        let wide = r.get("aspectRatio").and_then(Value::as_str)
            == Some("MUSIC_TWO_ROW_ITEM_THUMBNAIL_ASPECT_RATIO_RECTANGLE_16_9");
        let kind = r
            .pointer("/navigationEndpoint/watchEndpoint")
            .and_then(|w| w.pointer(MUSIC_VIDEO_TYPE))
            .and_then(Value::as_str)
            .map(video_type);
        wide && kind == Some(VideoType::MusicVideo)
    })
}

/// A page of shelves as Home has them: Home, and Explore's pages shaped like it.
fn home_page(raw: &str) -> Result<Vec<HomeShelf>, ApiError> {
    shelves_page(raw, |_| true)
}

fn shelves_page(raw: &str, keep: impl Fn(&Value) -> bool) -> Result<Vec<HomeShelf>, ApiError> {
    let v = json(raw)?;
    check_error(&v)?;
    let Some(section) = home_section(&v) else {
        if no_contents(&v) {
            return Ok(Vec::new());
        }
        return Err(ApiError::Parse(format!(
            "home page in an unknown layout ({})",
            layout(&v)
        )));
    };
    let shelves = section.get("contents").and_then(Value::as_array);
    Ok(shelves
        .into_iter()
        .flatten()
        .filter(|shelf| keep(shelf))
        .filter_map(home_shelf)
        .collect())
}

/// The next-page token of a Home page, first page or continuation.
pub fn home_continuation(raw: &str) -> Option<String> {
    let v: Value = serde_json::from_str(raw).ok()?;
    home_section(&v)?
        .pointer("/continuations/0/nextContinuationData/continuation")
        .and_then(Value::as_str)
        .map(str::to_string)
}

fn home_section(v: &Value) -> Option<&Value> {
    v.pointer(HOME_CONTINUATION).or_else(|| {
        v.pointer(
            "/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer",
        )
    })
}

fn home_shelf(shelf: &Value) -> Option<HomeShelf> {
    let r = shelf
        .get("musicCarouselShelfRenderer")
        .or_else(|| shelf.get("musicImmersiveCarouselShelfRenderer"))?;
    // musicCarouselShelfBasicHeaderRenderer, or the immersive variant.
    let header = r.get("header")?.as_object()?.values().next()?;
    let items: Vec<HomeItem> = r
        .get("contents")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(home_item)
        .collect();
    if items.is_empty() {
        return None;
    }
    Some(HomeShelf {
        title: text(header.get("title")).and_then(non_empty)?,
        strapline: text(header.get("strapline")).and_then(non_empty),
        items,
    })
}

fn home_item(item: &Value) -> Option<HomeItem> {
    if let Some(r) = item.get("musicTwoRowItemRenderer") {
        return two_row_item(item, r);
    }
    list_item(item.get("musicResponsiveListItemRenderer")?)
}

/// A card: a video (watch endpoint), or an album, playlist or artist page.
fn two_row_item(item: &Value, r: &Value) -> Option<HomeItem> {
    let thumbnail =
        best_raw(r.pointer("/thumbnailRenderer/musicThumbnailRenderer/thumbnail/thumbnails"));
    let title = text(r.get("title")).unwrap_or_default();
    let subtitle = r
        .pointer("/subtitle/runs")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let nav = r.get("navigationEndpoint")?;
    if let Some(w) = nav.get("watchEndpoint") {
        let (artists, album) = byline(subtitle);
        return watch_track(w, title, artists, album, thumbnail).map(HomeItem::Track);
    }
    let (id, page) = browse_target(r)?;
    match page {
        "MUSIC_PAGE_TYPE_ALBUM" | "MUSIC_PAGE_TYPE_AUDIOBOOK" => {
            library_album(item).map(HomeItem::Album)
        }
        "MUSIC_PAGE_TYPE_PLAYLIST" => Some(HomeItem::Playlist(PlaylistSummary {
            id: PlaylistId(bare_playlist_id(id).to_string()),
            title,
            author: card_author(subtitle),
            count: card_count(subtitle),
            views: card_views(subtitle),
            // A community playlist: the creator's round photo, in the collage's corner.
            avatar: best_raw(
                r.pointer("/thumbnailCornerOverlay/musicThumbnailRenderer/thumbnail/thumbnails"),
            ),
            thumbnail,
            owned: false,
        })),
        "MUSIC_PAGE_TYPE_ARTIST" => Some(HomeItem::Artist(ArtistSummary {
            id: ArtistId(id.to_string()),
            name: title,
            thumbnail,
        })),
        _ => None,
    }
}

/// A row (Quick picks): title column with the watch endpoint, then
/// `Artist • 3.8M plays`, then the album.
fn list_item(r: &Value) -> Option<HomeItem> {
    let column = |i: usize| {
        r.pointer(&format!(
            "/flexColumns/{i}/musicResponsiveListItemFlexColumnRenderer/text"
        ))
    };
    let runs = |i: usize| {
        column(i)
            .and_then(|t| t.get("runs"))
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
    };
    let w = runs(0)
        .iter()
        .find_map(|run| run.pointer("/navigationEndpoint/watchEndpoint"))
        .or_else(|| r.pointer("/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchEndpoint"))?;
    let (artists, mut album) = byline(runs(1));
    if album.is_none() {
        album = byline(runs(2)).1;
    }
    let thumbnail = best_raw(r.pointer("/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails"));
    let title = text(column(0)).unwrap_or_default();
    watch_track(w, title, artists, album, thumbnail).map(HomeItem::Track)
}

fn watch_track(
    w: &Value,
    title: String,
    artists: Vec<ArtistRef>,
    album: Option<AlbumRef>,
    thumbnail: Option<String>,
) -> Option<Track> {
    let id = w
        .get("videoId")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?;
    Some(Track {
        id: TrackId(id.to_string()),
        title,
        artists,
        album,
        duration: None,
        video_type: w
            .pointer(MUSIC_VIDEO_TYPE)
            .and_then(Value::as_str)
            .map(video_type)
            .unwrap_or(VideoType::Other),
        thumbnail,
        like: LikeStatus::Indifferent,
        counterpart: None,
    })
}

/// The linked channel in a playlist card's subtitle (`celine moradi • 234K
/// views`). Mixes list artists without links and have no author.
fn card_author(runs: &[Value]) -> Option<String> {
    runs.iter()
        .find(|run| browse_target(run).is_some())
        .and_then(|run| run.get("text").and_then(Value::as_str))
        .map(str::to_string)
        .and_then(non_empty)
}

/// `52 songs` or `12 tracks` in a card subtitle; views don't count.
fn card_count(runs: &[Value]) -> Option<u32> {
    runs.iter()
        .filter_map(|run| run.get("text").and_then(Value::as_str))
        .find(|t| {
            let t = t.to_ascii_lowercase();
            t.chars().next().is_some_and(|c| c.is_ascii_digit())
                && (t.contains("song") || t.contains("track"))
        })
        .and_then(parse_count)
}

/// `234K views` in a card subtitle (a community playlist), as it reads there.
fn card_views(runs: &[Value]) -> Option<String> {
    runs.iter()
        .filter_map(|run| run.get("text").and_then(Value::as_str))
        .map(str::trim)
        .find(|t| t.chars().next().is_some_and(|c| c.is_ascii_digit()) && t.ends_with("views"))
        .map(str::to_string)
}

// ---------------------------------------------------------------------------
// Explore: new releases, charts, moods and genres. The pages read like Home: a section list
// of carousels, so their shelves go through `home_page`.

/// New releases (`FEmusic_new_releases`): "Albums & singles", then "Music videos". The
/// untitled grid at the top (two release playlists) is left out, as Home leaves out what
/// it can't title.
pub fn new_releases(raw: &str) -> Result<Vec<HomeShelf>, ApiError> {
    home_page(raw)
}

/// The charts (`FEmusic_charts`): the country picker, the video charts (playlists) and the
/// top artists, ranked. A signed-in account may also get song shelves; they read as Home's.
pub fn charts(raw: &str) -> Result<Charts, ApiError> {
    let v = json(raw)?;
    check_error(&v)?;
    let section = home_section(&v)
        .ok_or_else(|| ApiError::Parse(format!("charts in an unknown layout ({})", layout(&v))))?;
    let sections = section
        .get("contents")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let mut charts = Charts {
        shelves: sections.iter().filter_map(home_shelf).collect(),
        artists: sections
            .iter()
            .filter_map(|s| s.pointer("/musicCarouselShelfRenderer/contents"))
            .filter_map(Value::as_array)
            .flatten()
            .filter_map(chart_artist)
            .collect(),
        ..Charts::default()
    };
    let (country, countries) = chart_countries(section);
    charts.country = country;
    charts.countries = countries;
    Ok(charts)
}

/// The country menu: the shown country's code, and every option. The shown one is the
/// option without a command (picking it changes nothing).
fn chart_countries(section: &Value) -> (Option<String>, Vec<Country>) {
    let mut shown = None;
    let mut out = Vec::new();
    walk(section, &mut |key, r| {
        if key != "musicMultiSelectMenuItemRenderer" {
            return;
        }
        let code = r
            .get("formItemEntityKey")
            .and_then(Value::as_str)
            .and_then(country_code);
        let (Some(code), Some(name)) = (code, text(r.get("title")).and_then(non_empty)) else {
            return;
        };
        if r.get("selectedCommand").is_none() && shown.is_none() {
            shown = Some(code.clone());
        }
        out.push(Country { code, name });
    });
    (shown, out)
}

/// The country code in a chart menu option's form key: URL-escaped base64 of a small
/// message holding `explore_charts_country_menu_<digits><CODE>`.
fn country_code(key: &str) -> Option<String> {
    const MARK: &[u8] = b"country_menu_";
    let bytes = base64(&key.replace("%3D", "="))?;
    let at = bytes.windows(MARK.len()).position(|w| w == MARK)? + MARK.len();
    let rest = &bytes[at..];
    let digits = rest.iter().take_while(|b| b.is_ascii_digit()).count();
    match rest.get(digits..digits + 2) {
        Some(code) if code.iter().all(u8::is_ascii_uppercase) => {
            Some(String::from_utf8_lossy(code).into_owned())
        }
        _ => None,
    }
}

/// Standard base64, padded or not. `None` for anything else.
fn base64(s: &str) -> Option<Vec<u8>> {
    let value = |c: u8| -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' | b'-' => 62,
            b'/' | b'_' => 63,
            _ => return None,
        } as u32)
    };
    let mut out = Vec::with_capacity(s.len() * 3 / 4);
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in s.trim_end_matches('=').bytes() {
        acc = (acc << 6) | value(c)?;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// A top artist's row: the rank and trend in the index column, the name, the subscribers.
fn chart_artist(item: &Value) -> Option<ChartArtist> {
    let r = item.get("musicResponsiveListItemRenderer")?;
    let index = r.pointer("/customIndexColumn/musicCustomIndexColumnRenderer")?;
    let (id, page) = browse_target(r)?;
    if page != "MUSIC_PAGE_TYPE_ARTIST" {
        return None;
    }
    let rank = text(index.get("text"))?.trim().parse().ok()?;
    let trend = match index.pointer("/icon/iconType").and_then(Value::as_str) {
        Some("ARROW_DROP_UP") => Trend::Up,
        Some("ARROW_DROP_DOWN") => Trend::Down,
        _ => Trend::Same,
    };
    Some(ChartArtist {
        rank,
        artist: ArtistSummary {
            id: ArtistId(id.to_string()),
            name: first_run_text(flex_column(r, 0)).and_then(non_empty)?,
            thumbnail: best_raw(r.pointer(ROW_THUMBNAILS)),
        },
        subscribers: first_run_text(flex_column(r, 1)).and_then(non_empty),
        trend,
    })
}

/// Moods and genres (`FEmusic_moods_and_genres`): each titled grid of buttons, a section.
pub fn moods(raw: &str) -> Result<Vec<MoodSection>, ApiError> {
    let v = json(raw)?;
    check_error(&v)?;
    let section = home_section(&v)
        .ok_or_else(|| ApiError::Parse(format!("moods in an unknown layout ({})", layout(&v))))?;
    let grids = section.get("contents").and_then(Value::as_array);
    Ok(grids
        .into_iter()
        .flatten()
        .filter_map(|s| s.get("gridRenderer"))
        .filter_map(|g| {
            let title = text(g.pointer("/header/gridHeaderRenderer/title")).and_then(non_empty)?;
            let moods: Vec<Mood> = g
                .get("items")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(mood)
                .collect();
            (!moods.is_empty()).then_some(MoodSection { title, moods })
        })
        .collect())
}

fn mood(item: &Value) -> Option<Mood> {
    let b = item.get("musicNavigationButtonRenderer")?;
    Some(Mood {
        title: text(b.get("buttonText")).and_then(non_empty)?,
        params: b
            .pointer("/clickCommand/browseEndpoint/params")
            .and_then(Value::as_str)
            .filter(|p| !p.is_empty())?
            .to_string(),
    })
}

/// A mood or genre's playlists (`FEmusic_moods_and_genres_category` with its params): the
/// page's title and its shelves.
pub fn mood_playlists(raw: &str) -> Result<MoodPage, ApiError> {
    let v = json(raw)?;
    let title = text(v.pointer("/header/musicHeaderRenderer/title"))
        .and_then(non_empty)
        .unwrap_or_default();
    Ok(MoodPage {
        title,
        shelves: home_page(raw)?,
    })
}

// ---------------------------------------------------------------------------
// History

/// The listening history (`FEmusic_history`): one shelf of song rows per day, newest first.
/// Signed out, the page only asks to sign in: `AuthExpired`.
pub fn history(raw: &str) -> Result<Vec<HistoryDay>, ApiError> {
    if logged_in(raw) == Some(false) {
        return Err(ApiError::AuthExpired);
    }
    let v = json(raw)?;
    check_error(&v)?;
    let Some(section) = home_section(&v) else {
        if no_contents(&v) {
            return Ok(Vec::new());
        }
        return Err(ApiError::Parse(format!(
            "history in an unknown layout ({})",
            layout(&v)
        )));
    };
    let sections = section
        .get("contents")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let days: Vec<HistoryDay> = sections
        .iter()
        .filter_map(|s| s.get("musicShelfRenderer"))
        .filter_map(|shelf| {
            let tracks: Vec<Track> = shelf
                .get("contents")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(playlist_row)
                .collect();
            let title = text(shelf.get("title")).and_then(non_empty)?;
            (!tracks.is_empty()).then_some(HistoryDay { title, tracks })
        })
        .collect();
    let asks_to_sign_in = sections.iter().any(|s| {
        s.pointer("/itemSectionRenderer/contents/0/messageRenderer")
            .is_some()
    });
    if days.is_empty() && asks_to_sign_in {
        return Err(ApiError::AuthExpired);
    }
    Ok(days)
}

// ---------------------------------------------------------------------------
// Small conversions

/// `"7:10"` or `"1:02:03"`.
pub fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let mut secs: u64 = 0;
    let mut parts = 0;
    for p in s.split(':') {
        secs = secs
            .checked_mul(60)?
            .checked_add(p.trim().parse::<u64>().ok()?)?;
        parts += 1;
    }
    (parts <= 3).then(|| Duration::from_secs(secs))
}

/// `MUSIC_VIDEO_TYPE_ATV` and friends, with or without the prefix.
pub fn video_type(s: &str) -> VideoType {
    match s.trim_start_matches("MUSIC_VIDEO_TYPE_") {
        "ATV" => VideoType::Song,
        "OMV" | "OFFICIAL_SOURCE_MUSIC" => VideoType::MusicVideo,
        "UGC" => VideoType::UserVideo,
        "PODCAST_EPISODE" => VideoType::Episode,
        _ => VideoType::Other,
    }
}

/// The `logged_in` tracking param: `Some(false)` for a signed-out response,
/// `None` when the response doesn't say.
pub fn logged_in(raw: &str) -> Option<bool> {
    let v: Value = serde_json::from_str(raw).ok()?;
    let services = v
        .pointer("/responseContext/serviceTrackingParams")?
        .as_array()?;
    services
        .iter()
        .filter_map(|s| s.get("params").and_then(Value::as_array))
        .flatten()
        .find(|p| p.get("key").and_then(Value::as_str) == Some("logged_in"))
        .and_then(|p| p.get("value").and_then(Value::as_str))
        .map(|v| v == "1")
}

/// The playlist id without the `VL` browse prefix.
pub fn bare_playlist_id(id: &str) -> &str {
    id.strip_prefix("VL").unwrap_or(id)
}

/// The browse id of a playlist: `VL` + the bare id.
pub fn browse_playlist_id(id: &str) -> String {
    format!("VL{}", bare_playlist_id(id))
}

/// The watch playlist of a playlist's radio: `RDAMPL` + the bare id, as ytmusicapi's
/// `get_watch_playlist(playlistId, radio=True)` asks for it.
pub fn playlist_radio_id(id: &str) -> String {
    format!("RDAMPL{}", bare_playlist_id(id))
}

/// Maps ytmapi-rs errors without echoing response bodies or headers.
pub fn map_error(e: ytmapi_rs::Error) -> ApiError {
    map_kind(e.into_kind())
}

fn map_kind(kind: ErrorKind) -> ApiError {
    match kind {
        ErrorKind::OtherErrorCodeInResponse { code, .. } => code_error(code),
        ErrorKind::Header | ErrorKind::OAuthTokenExpired { .. } => ApiError::AuthExpired,
        ErrorKind::Web { message } if message.starts_with("Error parsing response header") => {
            ApiError::Network("invalid response header".into())
        }
        ErrorKind::Web { message } => ApiError::Network(message),
        ErrorKind::Io(e) => ApiError::Network(e.to_string()),
        ErrorKind::JsonParsing(j) => {
            ApiError::Parse(format!("unexpected response at {}", j.get_json_and_key().1))
        }
        ErrorKind::InvalidResponse { .. } => ApiError::Parse("response is not valid JSON".into()),
        ErrorKind::ApiStatusFailed => ApiError::Network("the API reported STATUS_FAILED".into()),
        ErrorKind::InvalidUserAgent(_) => ApiError::Network("user agent rejected".into()),
        ErrorKind::NoVisitorData | ErrorKind::UnableToParseYtCfg { .. } => {
            ApiError::Network("could not read the YouTube Music page config".into())
        }
        _ => ApiError::Network("unexpected client error".into()),
    }
}

fn code_error(code: u64) -> ApiError {
    match code {
        401 | 403 => ApiError::AuthExpired,
        404 => ApiError::NotFound,
        c => ApiError::Network(format!("API error code {c}")),
    }
}

fn check_error(v: &Value) -> Result<(), ApiError> {
    match v.get("error") {
        Some(e) => Err(e
            .get("code")
            .and_then(Value::as_u64)
            .map(code_error)
            .unwrap_or_else(|| ApiError::Network("API error without a code".into()))),
        None => Ok(()),
    }
}

/// A browse answer with nothing in it: no contents, header or continuation, just the
/// response context (and a `noindex` microformat). The API answers so for a page this
/// identity can't see, such as another account's private playlist after a switch.
fn unavailable(raw: &str) -> bool {
    serde_json::from_str::<Value>(raw).is_ok_and(|v| no_contents(&v))
}

fn no_contents(v: &Value) -> bool {
    const CONTENT: [&str; 5] = [
        "contents",
        "continuationContents",
        "header",
        "onResponseReceivedActions",
        "error",
    ];
    v.as_object().is_some_and(|o| {
        o.contains_key("responseContext") && !CONTENT.iter().any(|k| o.contains_key(*k))
    })
}

/// The renderer names on the way to a page's content, for an error message: never values.
fn layout(v: &Value) -> String {
    let mut names = Vec::new();
    let mut at = v.get("contents");
    while let Some(Value::Object(o)) = at
        && names.len() < 4
    {
        let Some((k, next)) = o.iter().next() else {
            break;
        };
        names.push(k.as_str());
        at = Some(next);
    }
    if names.is_empty() {
        "no contents".into()
    } else {
        names.join(" > ")
    }
}

fn json(raw: &str) -> Result<Value, ApiError> {
    serde_json::from_str(raw).map_err(|_| ApiError::Parse("response is not valid JSON".into()))
}

fn process<Q: Query<BrowserToken>>(raw: &str, q: &Q) -> Result<Q::Output, ApiError> {
    process_json::<Q, BrowserToken>(raw.to_string(), q).map_err(map_error)
}

/// Parses the first page with `Q` and the rest as its continuations, leaving out the items
/// ytmapi-rs can't read (`process_lenient`). `what` names the list in the log.
fn parse_pages<Q>(pages: &[String], q: &Q, what: &str) -> Result<Vec<(Q::Output, Extra)>, ApiError>
where
    Q: Query<BrowserToken> + PostQuery,
    Q::Output: ParseFromContinuable<Q>,
{
    let cq = GetContinuationsQuery::new_mock_unchecked(q);
    pages
        .iter()
        .enumerate()
        .map(|(i, raw)| {
            let extra = Extra::from_raw(raw)?;
            let (items, skipped) = if i == 0 {
                process_lenient(raw, q)?
            } else {
                process_lenient(raw, &cq)?
            };
            log_left_out(&format!("{what}, page {}", i + 1), &skipped);
            Ok((items, extra))
        })
        .collect()
}

/// The renderers of the list items ytmapi-rs parses one by one. One it fails inside is taken
/// out of its list (`process_lenient`).
const LIST_ITEMS: [&str; 3] = [
    "musicResponsiveListItemRenderer",
    "musicTwoRowItemRenderer",
    "musicMultiRowListItemRenderer",
];

/// The most items `process_lenient` takes out of one page. A page with more it can't read
/// is in a shape it doesn't know at all.
const MAX_SKIPPED: usize = 100;

/// A list item ytmapi-rs couldn't read, taken out of its page by `process_lenient`.
struct Skipped {
    /// Its place in its list, as the page has it.
    index: usize,
    /// One of `LIST_ITEMS`.
    renderer: &'static str,
    /// The item as the page has it, `{renderer: {...}}`.
    item: Value,
    /// Where in the page ytmapi-rs failed: a JSON pointer, names and indices only.
    at: String,
}

/// `process`, but one item of a list ytmapi-rs fails inside (a row of a shape ytmapi-rs
/// 0.3.3 doesn't expect, `…/contents/20/musicResponsiveListItemRenderer/…`) is taken out of
/// the page and the page parsed again, at most `MAX_SKIPPED` times. Returns what ytmapi-rs
/// read and the items taken out, in the order they were taken. A failure outside every list
/// item (the page itself) is the error it was.
fn process_lenient<Q: Query<BrowserToken>>(
    raw: &str,
    q: &Q,
) -> Result<(Q::Output, Vec<Skipped>), ApiError> {
    let mut skipped = Vec::new();
    // The page as it's parsed: the raw text first, the edited JSON once an item is out.
    let mut page: Option<Value> = None;
    // Per list (a pointer), the original indices taken out, ascending.
    let mut taken: HashMap<String, Vec<usize>> = HashMap::new();
    loop {
        let text = match &page {
            None => raw.to_string(),
            Some(v) => v.to_string(),
        };
        let kind = match process_json::<Q, BrowserToken>(text, q) {
            Ok(out) => return Ok((out, skipped)),
            Err(e) => e.into_kind(),
        };
        let at = match &kind {
            ErrorKind::JsonParsing(j) => j.get_json_and_key().1.clone(),
            _ => return Err(map_kind(kind)),
        };
        let Some((list, index, renderer)) = item_at(&at) else {
            return Err(map_kind(kind));
        };
        if skipped.len() >= MAX_SKIPPED {
            return Err(map_kind(kind));
        }
        let v = match &mut page {
            Some(v) => v,
            None => page.insert(json(raw)?),
        };
        let Some(items) = v
            .pointer_mut(list)
            .and_then(Value::as_array_mut)
            .filter(|items| index < items.len())
        else {
            return Err(map_kind(kind));
        };
        let item = items.remove(index);
        let gone = taken.entry(list.to_string()).or_default();
        let original = original_index(gone, index);
        gone.insert(gone.partition_point(|&i| i < original), original);
        // The pointer as the page has it: the item's original index.
        let rest = &at[list.len() + 1 + index.to_string().len()..];
        let at = format!("{list}/{original}{rest}");
        skipped.push(Skipped {
            index: original,
            renderer,
            item,
            at,
        });
    }
}

/// The list item a ytmapi-rs error pointer is inside: the list's pointer, the item's index in
/// it and its renderer, for the innermost `/<index>/<one of LIST_ITEMS>` in `at`.
fn item_at(at: &str) -> Option<(&str, usize, &'static str)> {
    let mut found: Option<(usize, &str, usize, &'static str)> = None;
    for renderer in LIST_ITEMS {
        for (start, _) in at.match_indices(renderer) {
            let end = start + renderer.len();
            let whole =
                at[..start].ends_with('/') && (end == at.len() || at[end..].starts_with('/'));
            if !whole {
                continue;
            }
            let Some((list, index)) = at[..start - 1].rsplit_once('/') else {
                continue;
            };
            let Ok(index) = index.parse::<usize>() else {
                continue;
            };
            if found.is_none_or(|(s, ..)| start > s) {
                found = Some((start, list, index, renderer));
            }
        }
    }
    found.map(|(_, list, index, renderer)| (list, index, renderer))
}

/// The original index of what is now at `current` in a list from which the items at the
/// original indices `taken` (ascending) were removed.
fn original_index(taken: &[usize], current: usize) -> usize {
    let mut index = current;
    for &t in taken {
        if t <= index {
            index += 1;
        } else {
            break;
        }
    }
    index
}

/// Logs one line per item left out of `what`: its index and where ytmapi-rs failed.
fn log_left_out(what: &str, skipped: &[Skipped]) {
    for s in skipped {
        log::warn!("{what}: item {} left out, unreadable at {}", s.index, s.at);
    }
}

/// `kept` (what ytmapi-rs read, one per item it didn't skip, in order) with each skipped
/// item read by `read` back in its place, or left out when `read` can't either. Every
/// skipped item must be from the list `kept` came from. Logs one line per skipped item.
fn merge_skipped<T>(
    what: &str,
    kept: Vec<T>,
    mut skipped: Vec<Skipped>,
    read: impl Fn(&Value) -> Option<T>,
) -> Vec<T> {
    skipped.sort_by_key(|s| s.index);
    let mut out = Vec::with_capacity(kept.len() + skipped.len());
    let mut kept = kept.into_iter();
    // Original indices placed so far.
    let mut next = 0;
    for s in skipped {
        out.extend(kept.by_ref().take(s.index.saturating_sub(next)));
        next = s.index + 1;
        match read(&s.item) {
            Some(t) => {
                log::warn!(
                    "{what}: item {} read from the page, ytmapi-rs failed at {}",
                    s.index,
                    s.at
                );
                out.push(t);
            }
            None => log_left_out(what, std::slice::from_ref(&s)),
        }
    }
    out.extend(kept);
    out
}

fn album_summary(a: SearchResultAlbum, extra: &Extra) -> AlbumSummary {
    let id = a.album_id.get_raw().to_string();
    AlbumSummary {
        artists: extra.artists_or(&id, &a.artist),
        year: non_empty(a.year),
        thumbnail: best(&a.thumbnails),
        title: a.title,
        id: AlbumId(id),
    }
}

fn artist_ref(a: ParsedSongArtist) -> ArtistRef {
    ArtistRef {
        id: a.id.map(|i| ArtistId(i.get_raw().to_string())),
        name: a.name,
    }
}

fn album_ref(a: ParsedSongAlbum) -> AlbumRef {
    AlbumRef {
        id: AlbumId(a.id.get_raw().to_string()),
        title: a.name,
    }
}

fn like(s: &YtLike) -> LikeStatus {
    match s {
        YtLike::Liked => LikeStatus::Like,
        YtLike::Disliked => LikeStatus::Dislike,
        YtLike::Indifferent => LikeStatus::Indifferent,
    }
}

/// The widest thumbnail's URL.
fn best(t: &[Thumbnail]) -> Option<String> {
    t.iter().max_by_key(|t| t.width).map(|t| t.url.clone())
}

fn best_raw(t: Option<&Value>) -> Option<String> {
    t?.as_array()?
        .iter()
        .max_by_key(|t| t.get("width").and_then(Value::as_u64).unwrap_or(0))
        .and_then(|t| t.get("url").and_then(Value::as_str))
        .map(str::to_string)
}

/// Text of `{simpleText}` or `{runs: [{text}]}`.
fn text(v: Option<&Value>) -> Option<String> {
    let v = v?;
    if let Some(s) = v.get("simpleText").and_then(Value::as_str) {
        return Some(s.to_string());
    }
    let runs = v.get("runs")?.as_array()?;
    Some(
        runs.iter()
            .filter_map(|r| r.get("text").and_then(Value::as_str))
            .collect(),
    )
}

/// The leading number of `"52 tracks"` or `"1,234 songs"`.
fn parse_count(s: &str) -> Option<u32> {
    let first = s.split_whitespace().next()?;
    let digits: String = first.chars().filter(|c| *c != ',' && *c != '.').collect();
    digits.parse().ok()
}

fn non_empty(s: String) -> Option<String> {
    let t = s.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// What ytmapi-rs drops, read from the same raw response.
#[derive(Default)]
struct Extra {
    /// videoId -> type, from every watch endpoint (first one wins).
    types: HashMap<String, VideoType>,
    /// List rows by videoId (songs) or browseId (albums, artists).
    rows: HashMap<String, Row>,
}

struct Row {
    /// Linked artists in the second column.
    artists: Vec<ArtistRef>,
    thumbnail: Option<String>,
}

impl Extra {
    fn from_raw(raw: &str) -> Result<Extra, ApiError> {
        let v = json(raw)?;
        check_error(&v)?;
        let mut extra = Extra::default();
        walk(&v, &mut |key, val| match key {
            "watchEndpoint" => {
                if let Some(id) = val.get("videoId").and_then(Value::as_str)
                    && let Some(t) = val.pointer(MUSIC_VIDEO_TYPE).and_then(Value::as_str)
                {
                    extra
                        .types
                        .entry(id.to_string())
                        .or_insert_with(|| video_type(t));
                }
            }
            "musicResponsiveListItemRenderer" => {
                let key = val
                    .pointer("/playlistItemData/videoId")
                    .or_else(|| val.pointer("/navigationEndpoint/browseEndpoint/browseId"))
                    .and_then(Value::as_str);
                if let Some(key) = key {
                    let artists = val
                        .pointer(
                            "/flexColumns/1/musicResponsiveListItemFlexColumnRenderer/text/runs",
                        )
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|run| {
                            let (id, page) = browse_target(run)?;
                            (page == "MUSIC_PAGE_TYPE_ARTIST"
                                || page == "MUSIC_PAGE_TYPE_USER_CHANNEL")
                                .then(|| ArtistRef {
                                    id: Some(ArtistId(id.to_string())),
                                    name: run
                                        .get("text")
                                        .and_then(Value::as_str)
                                        .unwrap_or_default()
                                        .to_string(),
                                })
                        })
                        .collect();
                    let thumbnail = best_raw(
                        val.pointer("/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails"),
                    );
                    extra
                        .rows
                        .entry(key.to_string())
                        .or_insert(Row { artists, thumbnail });
                }
            }
            _ => {}
        });
        Ok(extra)
    }

    fn video_type_or(&self, id: &str, default: VideoType) -> VideoType {
        self.types.get(id).copied().unwrap_or(default)
    }

    /// Linked artists of the row, else the plain artist text as one artist.
    fn artists_or(&self, key: &str, plain: &str) -> Vec<ArtistRef> {
        match self.rows.get(key) {
            Some(r) if !r.artists.is_empty() => r.artists.clone(),
            _ => vec![ArtistRef {
                id: None,
                name: plain.to_string(),
            }],
        }
    }
}

/// Calls `f(key, value)` for every object entry, depth first.
fn walk<'a>(v: &'a Value, f: &mut impl FnMut(&'a str, &'a Value)) {
    match v {
        Value::Object(m) => {
            for (k, x) in m {
                f(k, x);
                walk(x, f);
            }
        }
        Value::Array(a) => {
            for x in a {
                walk(x, f);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROW: &str = "/contents/x/sectionListRenderer/contents/1/musicShelfRenderer/contents/20/musicResponsiveListItemRenderer/flexColumns/1/musicResponsiveListItemFlexColumnRenderer/text/runs";

    #[test]
    fn an_error_inside_a_row_names_the_row_and_its_list() {
        assert_eq!(
            item_at(ROW),
            Some((
                "/contents/x/sectionListRenderer/contents/1/musicShelfRenderer/contents",
                20,
                "musicResponsiveListItemRenderer"
            ))
        );
    }

    #[test]
    fn an_error_at_a_row_itself_names_it() {
        let at = "/a/contents/3/musicTwoRowItemRenderer";
        assert_eq!(
            item_at(at),
            Some(("/a/contents", 3, "musicTwoRowItemRenderer"))
        );
    }

    #[test]
    fn the_innermost_item_is_the_one_named() {
        let at = "/l/0/musicTwoRowItemRenderer/inner/7/musicResponsiveListItemRenderer/title";
        assert_eq!(
            item_at(at),
            Some((
                "/l/0/musicTwoRowItemRenderer/inner",
                7,
                "musicResponsiveListItemRenderer"
            ))
        );
    }

    #[test]
    fn an_error_outside_every_item_names_none() {
        assert_eq!(
            item_at("/contents/x/sectionListRenderer/contents/1/musicShelfRenderer"),
            None
        );
    }

    #[test]
    fn a_renderer_counts_only_as_a_whole_segment_after_an_index() {
        assert_eq!(item_at("/l/2/musicTwoRowItemRendererX/a"), None);
        assert_eq!(item_at("/l/header/musicTwoRowItemRenderer/a"), None);
    }

    #[test]
    fn indices_after_taken_items_map_back_to_the_page() {
        assert_eq!(original_index(&[], 4), 4);
        assert_eq!(original_index(&[4, 5], 4), 6);
        assert_eq!(original_index(&[4, 5], 3), 3);
        assert_eq!(original_index(&[0, 2], 1), 3);
    }
}
