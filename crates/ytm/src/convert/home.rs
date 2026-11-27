//! Home feed

use featherplay_core::model::{
    AlbumRef, ArtistId, ArtistRef, LikeStatus, PlaylistId, Track, TrackId, VideoType,
};
use serde_json::Value;

use super::playlists::{flex_column, library_album, runs_of};
use super::radio::{browse_target, byline};
use super::{
    MUSIC_VIDEO_TYPE, Page, bare_playlist_id, best_raw, check_error, json, layout, logged_in_of,
    no_contents, non_empty, parse_count, text, video_type,
};
use crate::{ApiError, ArtistSummary, HomeFeed, HomeItem, HomeShelf, PlaylistSummary};

const HOME_CONTINUATION: &str = "/continuationContents/sectionListContinuation";

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
    Ok(home_feed(pages)?.shelves)
}

/// The feed from its first pages (`home`), with the token of the page after the last.
pub fn home_feed(pages: &[String]) -> Result<HomeFeed, ApiError> {
    let Some((first, rest)) = pages.split_first() else {
        return Ok(HomeFeed::default());
    };
    let mut feed = home_more(first)?;
    for raw in rest {
        feed.more_from(feed_page(raw));
    }
    Ok(feed)
}

impl HomeFeed {
    /// Adds a continuation page's shelves, unless they don't read, and takes its token.
    pub(crate) fn more_from(&mut self, page: Page<Vec<HomeShelf>>) {
        if let Ok(shelves) = page.items {
            self.shelves.extend(shelves);
        }
        self.more = page.token;
    }
}

/// One later page of the feed (`YtMusic::home_more`) and the token after it. Unlike the
/// continuation `home` reads with the first page, a page that doesn't read is an error, so
/// Home can offer to try it again.
pub fn home_more(raw: &str) -> Result<HomeFeed, ApiError> {
    let page = feed_page(raw);
    Ok(HomeFeed {
        shelves: page.items?,
        more: page.token,
    })
}

/// A page of the Home feed, parsed once: its shelves (`home_page`) but "Music videos for
/// you", and the next page's token (`home_continuation`).
pub(crate) fn feed_page(raw: &str) -> Page<Vec<HomeShelf>> {
    let Ok(v) = serde_json::from_str::<Value>(raw) else {
        return Page::not_json();
    };
    Page {
        logged_in: logged_in_of(&v),
        items: shelves_of(&v, |shelf| !music_videos_only(shelf)),
        token: continuation_of(&v),
    }
}

/// Home's "Music videos for you": it offers Play all (a watch endpoint behind its header's
/// button), and every card is a video on a 16:9 thumbnail, music videos and the odd user
/// upload. It's built around the picture, and featherplay plays only the sound (intros,
/// skits and effects included). Told by its shape, not its title, which follows the UI
/// language. "Forgotten favorites", also all videos, offers no Play all (each card starts
/// its own radio), and stays; so do Quick picks (rows, not cards), a daily discover's Play
/// all over square song cards, and a genre's shelf
/// (other cards beside its music videos, and More rather than Play all).
fn music_videos_only(shelf: &Value) -> bool {
    let Some(r) = shelf.get("musicCarouselShelfRenderer") else {
        return false;
    };
    let play_all = r
        .pointer("/header/musicCarouselShelfBasicHeaderRenderer/moreContentButton/buttonRenderer/navigationEndpoint")
        .is_some_and(|e| e.get("watchEndpoint").is_some());
    if !play_all {
        return false;
    }
    let Some(cards) = r
        .get("contents")
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
        wide && matches!(kind, Some(VideoType::MusicVideo | VideoType::UserVideo))
    })
}

/// A page of shelves as Home has them: Home, and Explore's pages shaped like it.
pub(super) fn home_page(raw: &str) -> Result<Vec<HomeShelf>, ApiError> {
    shelves_of(&json(raw)?, |_| true)
}

/// The shelves of a page shaped like Home's, parsed, that `keep` keeps.
pub(super) fn shelves_of(
    v: &Value,
    keep: impl Fn(&Value) -> bool,
) -> Result<Vec<HomeShelf>, ApiError> {
    check_error(v)?;
    let Some(section) = home_section(v) else {
        if no_contents(v) {
            return Ok(Vec::new());
        }
        return Err(ApiError::Parse(format!(
            "home page in an unknown layout ({})",
            layout(v)
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
    continuation_of(&serde_json::from_str(raw).ok()?)
}

fn continuation_of(v: &Value) -> Option<String> {
    home_section(v)?
        .pointer("/continuations/0/nextContinuationData/continuation")
        .and_then(Value::as_str)
        .map(str::to_string)
}

pub(super) fn home_section(v: &Value) -> Option<&Value> {
    v.pointer(HOME_CONTINUATION).or_else(|| {
        v.pointer(
            "/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer",
        )
    })
}

pub(super) fn home_shelf(shelf: &Value) -> Option<HomeShelf> {
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

pub(super) fn home_item(item: &Value) -> Option<HomeItem> {
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
        let (artists, album) = byline(without_kind(subtitle));
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

/// A song or video card's subtitle past the kind word some shelves put first (Listen
/// again's `Song • Cece Natalie`, `Video • Artist • 3M views`), which isn't an artist.
/// Told by its shape, as the word follows the account's language: one unlinked run, a `•`,
/// then a group that links an artist. Other subtitles (`Artist • 907M views`, an unlinked
/// artist included) are left whole.
fn without_kind(runs: &[Value]) -> &[Value] {
    let unlinked = |run: &Value| browse_target(run).is_none();
    let is_dot = |run: &Value| run.get("text").and_then(Value::as_str).map(str::trim) == Some("•");
    let rest = match runs {
        [first, dot, rest @ ..] if unlinked(first) && is_dot(dot) => rest,
        _ => return runs,
    };
    // The group after the dot: values at even places, up to the next `•`.
    let links_an_artist = rest
        .iter()
        .enumerate()
        .take_while(|(i, run)| i % 2 == 0 || !is_dot(run))
        .filter(|(i, _)| i % 2 == 0)
        .any(|(_, run)| {
            browse_target(run).is_some_and(|(id, page)| {
                page == "MUSIC_PAGE_TYPE_ARTIST"
                    || page == "MUSIC_PAGE_TYPE_USER_CHANNEL"
                    || id.starts_with("UC")
            })
        });
    if links_an_artist { rest } else { runs }
}

/// A row (Quick picks): title column with the watch endpoint, then
/// `Artist • 3.8M plays`, then the album.
fn list_item(r: &Value) -> Option<HomeItem> {
    let column = |i: usize| flex_column(r, i);
    let runs = |i: usize| runs_of(column(i));
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

pub(super) fn watch_track(
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
pub(super) fn card_author(runs: &[Value]) -> Option<String> {
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
pub(super) fn card_views(runs: &[Value]) -> Option<String> {
    runs.iter()
        .filter_map(|run| run.get("text").and_then(Value::as_str))
        .map(str::trim)
        .find(|t| t.chars().next().is_some_and(|c| c.is_ascii_digit()) && t.ends_with("views"))
        .map(str::to_string)
}
