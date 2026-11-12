//! Playlists (including liked songs) and library

use std::collections::HashSet;
use std::time::Duration;

use featherplay_core::model::{
    AlbumId, AlbumRef, ArtistId, ArtistRef, LikeStatus, PlaylistId, Track, TrackId, VideoType,
};
use serde_json::Value;
use ytmapi_rs::common::{PlaylistID, YoutubeID};
use ytmapi_rs::parse::GetPlaylistDetails;
use ytmapi_rs::query::{GetLibraryArtistsQuery, GetPlaylistDetailsQuery};

use super::radio::{browse_target, byline};
use super::{
    MUSIC_VIDEO_TYPE, PLAY_WATCH_ENDPOINT, ROW_THUMBNAILS, bare_playlist_id, best, best_raw,
    check_error, json, layout, no_contents, non_empty, parse_count, parse_duration, parse_pages,
    process, text, unavailable, video_type, walk,
};
use crate::{AlbumSummary, ApiError, ArtistSummary, Playlist, PlaylistSummary};

const GRID_CONTINUATION: &str = "/continuationContents/gridContinuation";
const SHELF_CONTINUATION: &str = "/continuationContents/musicShelfContinuation";
const PLAYLIST_HEADER_ITEM: &str = "/contents/twoColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/0";
const PLAYLIST_COLUMNS: &str = "/contents/twoColumnBrowseResultsRenderer";
const PLAYLIST_SHELF: &str = "/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents/0/musicPlaylistShelfRenderer";
const PLAYLIST_SHELF_ROWS: &str = "/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents/0/musicPlaylistShelfRenderer/contents";
const APPENDED_ROWS: &str =
    "/onResponseReceivedActions/0/appendContinuationItemsAction/continuationItems";
const CONTINUATION_TOKEN: &str =
    "/continuationItemRenderer/continuationEndpoint/continuationCommand/token";
const GREY_OUT: &str = "MUSIC_ITEM_RENDERER_DISPLAY_POLICY_GREY_OUT";

/// Tracks from the first page and its continuations, read from raw JSON one row at a
/// time. ytmapi-rs 0.3.3 fails the whole playlist on one row it can't read, such as a
/// collaborative playlist's rows, which have their duration in a flex column and no fixed
/// column. Unavailable (greyed out, deleted) rows and uploads are dropped; a row without
/// a duration is kept without one.
///
/// A first page without a track shelf is an empty playlist. A continuation with nothing in
/// it is the end of the list: a mix's last row always offers a next page, and the answer
/// to the last one is empty. Any other continuation that isn't a list of rows is a parse
/// error. A row the service hands out a second time (`Listed`) is left out.
pub fn playlist_tracks(pages: &[String]) -> Result<Vec<Track>, ApiError> {
    let mut paging = PlaylistPaging::default();
    let mut out = Vec::new();
    for raw in pages {
        out.extend(paging.read(raw)?.0);
    }
    Ok(out)
}

/// The rows a playlist's pages have listed so far, to tell the ones that come a second
/// time. A mix's continuations go round at its end: asked for what follows its last rows,
/// the service hands some of them out again, as entries it has already listed
/// (`playlistSetVideoId` numbers a list's entries). A row is new when its entry or its
/// video is: a playlist that has a song twice has it as two entries.
#[derive(Default)]
struct Listed {
    entries: HashSet<String>,
    videos: HashSet<String>,
}

impl Listed {
    /// Notes `row` and says whether it's new. A row that doesn't name its entry and its
    /// video (a collaborative playlist's, the continuation item) always is.
    fn add(&mut self, row: &Value) -> bool {
        let data = row.pointer("/musicResponsiveListItemRenderer/playlistItemData");
        let id = |key: &str| data.and_then(|d| d.get(key)).and_then(Value::as_str);
        let (Some(entry), Some(video)) = (id("playlistSetVideoId"), id("videoId")) else {
            return true;
        };
        // Both are noted, whatever the first one says.
        let new_entry = self.entries.insert(entry.to_string());
        let new_video = self.videos.insert(video.to_string());
        new_entry || new_video
    }
}

/// The next-page token of a playlist page (first page or continuation): the last row's
/// continuation item.
pub fn playlist_continuation(raw: &str) -> Option<String> {
    let v: Value = serde_json::from_str(raw).ok()?;
    continuation_token(playlist_rows(&v)?)
}

fn continuation_token(rows: &[Value]) -> Option<String> {
    rows.last()?
        .pointer(CONTINUATION_TOKEN)
        .and_then(Value::as_str)
        .filter(|t| !t.is_empty())
        .map(str::to_string)
}

/// Follows a playlist's pages to the end of its list, a page at a time as they come, the
/// first one included: `read` for its new songs and the next page's token, or `next_token`
/// for the token alone.
#[derive(Default)]
pub struct PlaylistPaging {
    listed: Listed,
    /// The pages read so far.
    pages: usize,
}

impl PlaylistPaging {
    /// The songs on `raw` not listed before, as `playlist_tracks` reads them, and the token
    /// of the page after it, or `None` at the end of the list. A mix has no last page: every
    /// one ends with a token. Its list ends at an answer with no rows, or at a page whose
    /// rows were all listed before (`Listed`), whose token would only bring them round again.
    pub fn read(&mut self, raw: &str) -> Result<(Vec<Track>, Option<String>), ApiError> {
        let first = self.pages == 0;
        self.pages += 1;
        if first && unavailable(raw) {
            return Err(ApiError::NotFound);
        }
        let v = json(raw)?;
        check_error(&v)?;
        let rows = match playlist_rows(&v) {
            Some(rows) => rows,
            None if first && v.pointer(PLAYLIST_COLUMNS).is_some() => return Ok((vec![], None)),
            None if !first && no_contents(&v) => return Ok((vec![], None)),
            None => {
                return Err(ApiError::Parse(format!(
                    "playlist page has no rows ({})",
                    layout(&v)
                )));
            }
        };
        let (new, again): (Vec<_>, Vec<_>) = rows
            .iter()
            .filter(|row| row.get("musicResponsiveListItemRenderer").is_some())
            .partition(|row| self.listed.add(row));
        let token = if new.is_empty() && !again.is_empty() {
            None
        } else {
            continuation_token(rows)
        };
        Ok((new.into_iter().filter_map(playlist_row).collect(), token))
    }

    /// The token of the page after `raw`, as `read` gives it; `None` for a page it can't
    /// read, too.
    pub fn next_token(&mut self, raw: &str) -> Option<String> {
        self.read(raw).ok()?.1
    }
}

/// A playlist's first page, `raw`, read: the header and its songs, and where the list goes
/// on (`PlaylistRest`), if it does.
pub fn playlist_first(
    raw: &str,
    id: &PlaylistId,
) -> Result<(Playlist, Option<PlaylistRest>), ApiError> {
    let summary = playlist_summary(raw, id)?;
    let mut paging = PlaylistPaging::default();
    let (tracks, token) = paging.read(raw)?;
    let rest = token.map(|token| PlaylistRest {
        id: id.clone(),
        paging,
        token,
    });
    let first = Playlist {
        summary,
        tracks,
        pages: 1,
    };
    Ok((first, rest))
}

/// Where a playlist read so far goes on (`playlist_first`): the next page's token, and the
/// rows listed so far, so the pages after leave out the ones they hand out again. Read on
/// with `YtMusic::playlist_rest`, or a page at a time with `read`.
pub struct PlaylistRest {
    id: PlaylistId,
    paging: PlaylistPaging,
    token: String,
}

impl PlaylistRest {
    pub fn id(&self) -> &PlaylistId {
        &self.id
    }

    /// The pages read so far, the first included.
    pub fn pages(&self) -> usize {
        self.paging.pages
    }

    /// What the next page is asked for with.
    pub fn token(&self) -> &str {
        &self.token
    }

    /// Reads `raw`, the answer to `token`: its new songs, and where the list goes on after
    /// it, if it does (`PlaylistPaging::read`).
    pub fn read(mut self, raw: &str) -> Result<(Vec<Track>, Option<PlaylistRest>), ApiError> {
        let (tracks, next) = self.paging.read(raw)?;
        Ok((tracks, next.map(|token| PlaylistRest { token, ..self })))
    }
}

impl std::fmt::Debug for PlaylistRest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlaylistRest")
            .field("id", &self.id)
            .field("pages", &self.pages())
            .finish_non_exhaustive()
    }
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
pub(super) fn playlist_row(item: &Value) -> Option<Track> {
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
pub(super) fn row_duration(r: &Value) -> Option<Duration> {
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

pub(super) fn row_like(r: &Value) -> LikeStatus {
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

pub(super) fn greyed_out(r: &Value) -> bool {
    r.get("musicItemRendererDisplayPolicy")
        .and_then(Value::as_str)
        == Some(GREY_OUT)
}

/// A list row's flex column `i`: its `text`.
pub(super) fn flex_column(r: &Value, i: usize) -> Option<&Value> {
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

pub(super) fn runs_of(t: Option<&Value>) -> &[Value] {
    t.and_then(|t| t.get("runs"))
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
}

pub(super) fn first_run_text(t: Option<&Value>) -> Option<String> {
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
pub(super) fn library_album(item: &Value) -> Option<AlbumSummary> {
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
