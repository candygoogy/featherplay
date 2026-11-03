//! The search suggestions under the top bar's field (`convert::search_suggestions`), from
//! the recorded, scrubbed reply for "daft pu" and from small hand-made replies.

use std::path::PathBuf;

use featherplay_core::model::{AlbumId, ArtistId, PlaylistId, TrackId, VideoType};
use featherplay_ytm::{HomeItem, QuerySuggestion, SearchSuggestions, convert};
use serde_json::{Value, json};

fn recorded() -> SearchSuggestions {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/search_suggestions.json");
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    convert::search_suggestions(&raw).expect("parse")
}

/// A reply with `queries` as the first section and `results` as the second.
fn reply(queries: Vec<Value>, results: Vec<Value>) -> String {
    let section =
        |items: Vec<Value>| json!({ "searchSuggestionsSectionRenderer": { "contents": items } });
    json!({ "contents": [section(queries), section(results)] }).to_string()
}

fn browse(id: &str, page: &str) -> Value {
    json!({ "browseEndpoint": {
        "browseId": id,
        "browseEndpointContextSupportedConfigs": {
            "browseEndpointContextMusicConfig": { "pageType": page }
        }
    } })
}

fn column(runs: Value) -> Value {
    json!({ "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": runs } } })
}

fn thumbnail() -> Value {
    json!({ "musicThumbnailRenderer": { "thumbnail": { "thumbnails": [
        { "url": "https://example.test/60", "width": 60, "height": 60 },
        { "url": "https://example.test/120", "width": 120, "height": 120 }
    ] } } })
}

#[test]
fn the_completions_come_in_order_with_their_full_query() {
    let texts: Vec<String> = recorded().queries.into_iter().map(|q| q.text).collect();
    assert_eq!(
        texts,
        [
            "daft punk",
            "daft punk veridis quo",
            "daft punk instant crush",
            "daft punk around the world",
            "daft punk get lucky",
            "daft punk one more time"
        ]
    );
}

#[test]
fn a_completion_marks_the_typed_part() {
    assert_eq!(
        recorded().queries[0],
        QuerySuggestion {
            text: "daft punk".into(),
            typed: "daft pu".len(),
            history: false,
        }
    );
}

#[test]
fn the_direct_results_follow_the_reply() {
    let kinds: Vec<&str> = recorded()
        .results
        .iter()
        .map(|r| match r.item {
            HomeItem::Track(_) => "track",
            HomeItem::Album(_) => "album",
            HomeItem::Playlist(_) => "playlist",
            HomeItem::Artist(_) => "artist",
        })
        .collect();
    assert_eq!(kinds, ["artist", "track", "track", "track", "track"]);
}

#[test]
fn an_artist_result_has_its_channel_photo_and_audience() {
    let first = &recorded().results[0];
    let HomeItem::Artist(artist) = &first.item else {
        panic!("not an artist: {first:?}");
    };
    assert_eq!(artist.id, ArtistId("UCRr1xG_2WIDs18a6cIiCxeA".into()));
    assert_eq!(artist.name, "Daft Punk");
    assert!(
        artist
            .thumbnail
            .as_deref()
            .is_some_and(|u| u.contains("w120-h120"))
    );
    assert_eq!(first.subtitle, "68.2M monthly audience");
}

#[test]
fn a_song_result_plays_with_its_artist_and_album() {
    let second = &recorded().results[1];
    let HomeItem::Track(track) = &second.item else {
        panic!("not a track: {second:?}");
    };
    assert_eq!(track.id, TrackId("qe8Q7mjxjig".into()));
    assert_eq!(track.title, "Veridis Quo");
    assert_eq!(track.video_type, VideoType::Song);
    assert_eq!(track.artists.len(), 1);
    assert_eq!(track.artists[0].name, "Daft Punk");
    assert_eq!(
        track.album.as_ref().map(|a| &a.id),
        Some(&AlbumId("MPREb_DYSzDJvUSDu".into()))
    );
    assert!(track.thumbnail.is_some());
    assert_eq!(second.subtitle, "Song • Daft Punk • 111M plays");
}

#[test]
fn a_past_search_is_a_completion_marked_as_history() {
    let raw = reply(
        vec![json!({ "historySuggestionRenderer": {
            "suggestion": { "runs": [{ "text": "air", "bold": true }, { "text": " moon safari" }] },
            "navigationEndpoint": { "searchEndpoint": { "query": "air moon safari" } }
        } })],
        vec![],
    );
    let got = convert::search_suggestions(&raw).expect("parse");
    assert_eq!(
        got.queries,
        [QuerySuggestion {
            text: "air moon safari".into(),
            typed: 3,
            history: true,
        }]
    );
}

#[test]
fn an_album_result_opens_the_album() {
    let raw = reply(
        vec![],
        vec![json!({ "musicResponsiveListItemRenderer": {
            "thumbnail": thumbnail(),
            "navigationEndpoint": browse("MPREb_dMHztPsrDgN", "MUSIC_PAGE_TYPE_ALBUM"),
            "flexColumns": [
                column(json!([{ "text": "Homework" }])),
                column(json!([
                    { "text": "Album" }, { "text": " • " },
                    { "text": "Daft Punk", "navigationEndpoint": browse("UCRr1xG_2WIDs18a6cIiCxeA", "MUSIC_PAGE_TYPE_ARTIST") },
                    { "text": " • " }, { "text": "1997" }
                ]))
            ]
        } })],
    );
    let got = convert::search_suggestions(&raw).expect("parse");
    let HomeItem::Album(album) = &got.results[0].item else {
        panic!("not an album: {:?}", got.results);
    };
    assert_eq!(album.id, AlbumId("MPREb_dMHztPsrDgN".into()));
    assert_eq!(album.title, "Homework");
    assert_eq!(album.artists.len(), 1);
    assert_eq!(album.artists[0].name, "Daft Punk");
    assert_eq!(album.thumbnail.as_deref(), Some("https://example.test/120"));
    assert_eq!(got.results[0].subtitle, "Album • Daft Punk • 1997");
}

#[test]
fn a_playlist_result_opens_the_playlist() {
    let raw = reply(
        vec![],
        vec![json!({ "musicResponsiveListItemRenderer": {
            "thumbnail": thumbnail(),
            "navigationEndpoint": browse("VLPLabc123", "MUSIC_PAGE_TYPE_PLAYLIST"),
            "flexColumns": [
                column(json!([{ "text": "French Touch" }])),
                column(json!([{ "text": "Playlist" }, { "text": " • " }, { "text": "Someone" }]))
            ]
        } })],
    );
    let got = convert::search_suggestions(&raw).expect("parse");
    let HomeItem::Playlist(p) = &got.results[0].item else {
        panic!("not a playlist: {:?}", got.results);
    };
    assert_eq!(p.id, PlaylistId("PLabc123".into()));
    assert_eq!(p.title, "French Touch");
}

#[test]
fn a_row_it_cannot_open_is_left_out() {
    let raw = reply(
        vec![],
        vec![json!({ "musicResponsiveListItemRenderer": {
            "flexColumns": [column(json!([{ "text": "A podcast" }]))],
            "navigationEndpoint": browse("MPSPxyz", "MUSIC_PAGE_TYPE_PODCAST_SHOW_DETAIL_PAGE")
        } })],
    );
    assert!(
        convert::search_suggestions(&raw)
            .expect("parse")
            .results
            .is_empty()
    );
}

#[test]
fn a_reply_without_sections_has_no_suggestions() {
    let got = convert::search_suggestions(&json!({ "responseContext": {} }).to_string());
    assert_eq!(got, Ok(SearchSuggestions::default()));
}
