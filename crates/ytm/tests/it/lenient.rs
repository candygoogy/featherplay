//! One item ytmapi-rs 0.3.3 can't read must not fail its whole page: the item is read from
//! the raw JSON instead (a song row), or left out, and the rest of the list is kept.
//!
//! `search_songs_videos.json` is the "exciting" song search as the v0.1.0 release failed on
//! it (recorded 2026-11-03, scrubbed to its `contents`, without tracking tokens): rows 21 and
//! 23 are music videos, `Artists • 1.2M views • 2:36` with no album and no plays column.
//! `search_albums.json` and `search_artists.json` are the same query's other filters.

use std::path::PathBuf;
use std::time::Duration;

use featherplay_core::model::{ArtistId, TrackId, VideoType};
use featherplay_ytm::convert;
use serde_json::{Value, json};

const SEARCH_ROWS: &str = "/contents/tabbedSearchResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/1/musicShelfRenderer/contents";
const ARTIST_SECTIONS: &str = "/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents";

fn fixture(name: &str) -> String {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(format!("tests/fixtures/{name}.json"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Fixture `name` with `f` applied to the item at `pointer`.
fn mangled(
    name: &str,
    pointer: &str,
    f: impl FnOnce(&mut serde_json::Map<String, Value>),
) -> String {
    let mut v: Value = serde_json::from_str(&fixture(name)).expect("fixture json");
    let item = v
        .pointer_mut(pointer)
        .and_then(Value::as_object_mut)
        .unwrap_or_else(|| panic!("no item at {pointer}"));
    f(item);
    v.to_string()
}

/// The ids of the rows at `rows` in fixture `name`: each list row's video id or browse id.
fn row_ids(name: &str, rows: &str) -> Vec<String> {
    let v: Value = serde_json::from_str(&fixture(name)).expect("fixture json");
    v.pointer(rows)
        .and_then(Value::as_array)
        .expect("rows")
        .iter()
        .map(|row| {
            let r = row
                .as_object()
                .and_then(|o| o.values().next())
                .expect("renderer");
            r.pointer("/playlistItemData/videoId")
                .or_else(|| r.pointer("/navigationEndpoint/browseEndpoint/browseId"))
                .or_else(|| r.pointer("/title/runs/0/navigationEndpoint/browseEndpoint/browseId"))
                .and_then(Value::as_str)
                .expect("row id")
                .to_string()
        })
        .collect()
}

/// A list row ytmapi-rs can't read, and nor can we: no columns at all.
fn without_columns(r: &mut serde_json::Map<String, Value>) {
    let row = r
        .get_mut("musicResponsiveListItemRenderer")
        .and_then(Value::as_object_mut)
        .expect("a list row");
    row.remove("flexColumns");
}

// ---------------------------------------------------------------------------
// Search

#[test]
fn a_song_search_with_music_videos_keeps_every_row_in_order() {
    let songs = convert::search_songs(&fixture("search_songs_videos")).expect("parse");
    let ids: Vec<String> = songs.iter().map(|t| t.id.0.clone()).collect();
    assert_eq!(ids, row_ids("search_songs_videos", SEARCH_ROWS));
}

#[test]
fn a_music_video_in_the_song_search_is_read_from_its_row() {
    let songs = convert::search_songs(&fixture("search_songs_videos")).expect("parse");
    let video = &songs[21];
    assert_eq!(video.id, TrackId("QXirCw8wwIA".into()));
    assert_eq!(
        video.title,
        "Super Slowed // EEYUH! X FLUXXWAVE (Irokz Remix)"
    );
    assert_eq!(video.video_type, VideoType::MusicVideo);
    let artists: Vec<&str> = video.artists.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(artists, ["Irokz", "Clovis Reyes", "HR"]);
    assert!(video.artists.iter().all(|a| a.id.is_some()));
    assert_eq!(video.album, None);
    assert_eq!(video.duration, Some(Duration::from_secs(156)));
    assert!(video.thumbnail.is_some());
}

#[test]
fn the_songs_around_a_music_video_are_still_read_by_ytmapi() {
    let songs = convert::search_songs(&fixture("search_songs_videos")).expect("parse");
    let song = &songs[20];
    assert_eq!(song.id, TrackId("KWEsdF57hB4".into()));
    assert_eq!(song.video_type, VideoType::Song);
    assert_eq!(song.duration, Some(Duration::from_secs(273)));
    assert!(song.album.is_some());
}

#[test]
fn a_song_row_nobody_can_read_is_left_out_and_the_rest_kept() {
    let raw = mangled("search_songs", &format!("{SEARCH_ROWS}/5"), without_columns);
    let songs = convert::search_songs(&raw).expect("parse");
    let mut want = row_ids("search_songs", SEARCH_ROWS);
    want.remove(5);
    let ids: Vec<String> = songs.iter().map(|t| t.id.0.clone()).collect();
    assert_eq!(ids, want);
}

#[test]
fn rows_around_and_between_unreadable_ones_keep_their_places() {
    // Rows 4 and 5 unreadable by anyone, row 7 by ytmapi-rs only (no plays column).
    let mut v: Value = serde_json::from_str(&fixture("search_songs")).expect("fixture json");
    let rows = v
        .pointer_mut(SEARCH_ROWS)
        .and_then(Value::as_array_mut)
        .expect("rows");
    for i in [4, 5] {
        without_columns(rows[i].as_object_mut().expect("row"));
    }
    rows[7]["musicResponsiveListItemRenderer"]["flexColumns"]
        .as_array_mut()
        .expect("columns")
        .truncate(2);
    let songs = convert::search_songs(&v.to_string()).expect("parse");
    let mut want = row_ids("search_songs", SEARCH_ROWS);
    want.drain(4..6);
    let ids: Vec<String> = songs.iter().map(|t| t.id.0.clone()).collect();
    assert_eq!(ids, want);
}

#[test]
fn an_album_row_that_cant_be_read_is_left_out_and_the_rest_kept() {
    let raw = mangled(
        "search_albums",
        &format!("{SEARCH_ROWS}/2"),
        without_columns,
    );
    let albums = convert::search_albums(&raw).expect("parse");
    let mut want = row_ids("search_albums", SEARCH_ROWS);
    want.remove(2);
    let ids: Vec<String> = albums.iter().map(|a| a.id.0.clone()).collect();
    assert_eq!(ids, want);
}

#[test]
fn an_artist_row_that_cant_be_read_is_left_out_and_the_rest_kept() {
    let raw = mangled(
        "search_artists",
        &format!("{SEARCH_ROWS}/1"),
        without_columns,
    );
    let artists = convert::search_artists(&raw).expect("parse");
    let mut want = row_ids("search_artists", SEARCH_ROWS);
    want.remove(1);
    let ids: Vec<String> = artists.iter().map(|a| a.id.0.clone()).collect();
    assert_eq!(ids, want);
}

#[test]
fn a_playlist_row_that_cant_be_read_is_left_out_and_the_rest_kept() {
    let raw = mangled(
        "search_playlists",
        &format!("{SEARCH_ROWS}/3"),
        without_columns,
    );
    let playlists = convert::search_playlists(&raw).expect("parse");
    let mut want: Vec<String> = row_ids("search_playlists", SEARCH_ROWS)
        .into_iter()
        .map(|id| convert::bare_playlist_id(&id).to_string())
        .collect();
    want.remove(3);
    let ids: Vec<String> = playlists.iter().map(|p| p.id.0.clone()).collect();
    assert_eq!(ids, want);
}

#[test]
fn a_search_page_that_isnt_a_list_still_fails() {
    let raw = json!({ "contents": { "tabbedSearchResultsRenderer": {} } }).to_string();
    assert!(convert::search_songs(&raw).is_err());
}

// ---------------------------------------------------------------------------
// Library artists: a first page and a continuation, one row of each unreadable.

fn artist_row(id: &str, name: &str) -> Value {
    json!({ "musicResponsiveListItemRenderer": {
        "navigationEndpoint": { "browseEndpoint": { "browseId": id } },
        "flexColumns": [
            { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": name }] } } },
            { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": "12 songs" }] } } },
        ],
    } })
}

fn unreadable_artist_row(id: &str) -> Value {
    json!({ "musicResponsiveListItemRenderer": {
        "navigationEndpoint": { "browseEndpoint": { "browseId": id } },
    } })
}

fn library_artist_pages() -> Vec<String> {
    let first = json!({ "contents": { "singleColumnBrowseResultsRenderer": { "tabs": [{ "tabRenderer": { "content": {
        "sectionListRenderer": { "contents": [{ "musicShelfRenderer": {
            "contents": [
                artist_row("UC0000000000000000000001", "One"),
                unreadable_artist_row("UC0000000000000000000002"),
                artist_row("UC0000000000000000000003", "Three"),
            ],
            "continuations": [{ "nextContinuationData": { "continuation": "PAGE2" } }],
        } }] },
    } } }] } } });
    let second = json!({ "continuationContents": { "musicShelfContinuation": {
        "contents": [
            unreadable_artist_row("UC0000000000000000000004"),
            artist_row("UC0000000000000000000005", "Five"),
        ],
    } } });
    vec![first.to_string(), second.to_string()]
}

#[test]
fn library_artists_keep_the_rows_around_one_that_cant_be_read() {
    let artists = convert::library_artists(&library_artist_pages()).expect("parse");
    let ids: Vec<&str> = artists.iter().map(|a| a.id.0.as_str()).collect();
    assert_eq!(
        ids,
        [
            "UC0000000000000000000001",
            "UC0000000000000000000003",
            "UC0000000000000000000005"
        ]
    );
}

#[test]
fn library_artists_page_on_past_a_row_that_cant_be_read() {
    // The next page's token is read from the raw page, not from ytmapi-rs's parse of it.
    let pages = library_artist_pages();
    assert_eq!(
        convert::shelf_continuation(&pages[0]).as_deref(),
        Some("PAGE2")
    );
    assert_eq!(convert::shelf_continuation(&pages[1]), None);
}

// ---------------------------------------------------------------------------
// The artist page

#[test]
fn an_artist_top_song_ytmapi_cant_read_is_read_from_its_row() {
    // Without its menu, ytmapi-rs finds no like status; the row itself is fine.
    let raw = mangled(
        "artist",
        &format!(
            "{ARTIST_SECTIONS}/0/musicShelfRenderer/contents/1/musicResponsiveListItemRenderer"
        ),
        |r| {
            r.remove("menu");
        },
    );
    let artist = convert::artist(&raw).expect("parse");
    let ids: Vec<String> = artist.top_songs.iter().map(|t| t.id.0.clone()).collect();
    assert_eq!(
        ids,
        row_ids(
            "artist",
            &format!("{ARTIST_SECTIONS}/0/musicShelfRenderer/contents")
        )
    );
    let song = &artist.top_songs[1];
    assert_eq!(song.id, TrackId("Rgrt_8mXrK8".into()));
    assert!(!song.title.is_empty());
    // Linked as ytmapi-rs links the other rows' artist.
    assert_eq!(
        song.artists[0].id,
        Some(ArtistId("UCRr1xG_2WIDs18a6cIiCxeA".into()))
    );
    assert_eq!(song.artists[0], artist.top_songs[0].artists[0]);
}

#[test]
fn an_artist_album_that_cant_be_read_is_left_out_and_the_rest_kept() {
    let albums = format!("{ARTIST_SECTIONS}/1/musicCarouselShelfRenderer/contents");
    let raw = mangled(
        "artist",
        &format!("{albums}/4/musicTwoRowItemRenderer"),
        |r| {
            r.remove("menu");
        },
    );
    let artist = convert::artist(&raw).expect("parse");
    let mut want = row_ids("artist", &albums);
    want.remove(4);
    let ids: Vec<String> = artist.albums.iter().map(|a| a.id.0.clone()).collect();
    assert_eq!(ids, want);
}
