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

mod account;
mod album_artist;
mod explore;
mod history;
mod home;
mod lyrics;
mod playlists;
mod radio;
mod search;

pub use account::{account_info, accounts};
pub use album_artist::{album, artist};
pub use explore::{charts, mood_playlists, moods, new_releases};
pub use history::history;
pub use home::{home, home_continuation, home_feed, home_more};
pub use lyrics::{lyrics, lyrics_browse_id, timed_lyrics};
pub use playlists::{
    PlaylistPaging, PlaylistRest, grid_continuation, library_albums, library_artists,
    library_playlists, playlist_continuation, playlist_first, playlist_summary, playlist_tracks,
    shelf_continuation,
};
pub use radio::{radio, seed_track};
pub use search::{
    search_albums, search_artists, search_playlists, search_songs, search_suggestions,
};

use std::collections::HashMap;
use std::time::Duration;

use featherplay_core::model::{AlbumId, AlbumRef, ArtistId, ArtistRef, LikeStatus, VideoType};
use serde_json::Value;
use ytmapi_rs::auth::BrowserToken;
use ytmapi_rs::common::{LikeStatus as YtLike, Thumbnail, YoutubeID};
use ytmapi_rs::continuations::ParseFromContinuable;
use ytmapi_rs::error::ErrorKind;
use ytmapi_rs::parse::{ParsedSongAlbum, ParsedSongArtist, SearchResultAlbum};
use ytmapi_rs::process_json;
use ytmapi_rs::query::{GetContinuationsQuery, PostQuery, Query};

use self::radio::browse_target;
use crate::{AlbumSummary, ApiError};

const MUSIC_VIDEO_TYPE: &str =
    "/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType";
const PLAY_WATCH_ENDPOINT: &str = "/overlay/musicItemThumbnailOverlayRenderer/content/musicPlayButtonRenderer/playNavigationEndpoint/watchEndpoint";
const ROW_THUMBNAILS: &str = "/thumbnail/musicThumbnailRenderer/thumbnail/thumbnails";

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
