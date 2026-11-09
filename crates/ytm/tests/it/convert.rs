//! Conversions from recorded, scrubbed API responses (tests/fixtures).

use std::path::PathBuf;
use std::time::Duration;

use featherplay_core::model::{AlbumId, AlbumRef, ArtistId, PlaylistId, TrackId, VideoType};
use featherplay_ytm::{ApiError, HomeItem, HomeShelf, Trend, convert};
use serde_json::{Value, json};

const PANEL: &str = "/contents/singleColumnMusicWatchNextResultsRenderer/tabbedRenderer/watchNextTabbedResultsRenderer/tabs/0/tabRenderer/content/musicQueueRenderer/content/playlistPanelRenderer/contents";

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn fixture(name: &str) -> String {
    let path = fixtures_dir().join(format!("{name}.json"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[test]
fn search_song_has_atv_and_duration() {
    let songs = convert::search_songs(&fixture("search_songs")).expect("parse");
    assert_eq!(songs.len(), 20);
    let song = songs
        .iter()
        .find(|t| t.id == TrackId("Jb6gcoR266U".into()))
        .expect("Around the World in results");
    assert_eq!(song.title, "Around the World");
    assert_eq!(song.video_type, VideoType::Song);
    // The fixture shows 7:10.
    assert_eq!(song.duration, Some(Duration::from_secs(430)));
    assert_eq!(song.artists[0].name, "Daft Punk");
    assert_eq!(
        song.artists[0].id,
        Some(ArtistId("UCRr1xG_2WIDs18a6cIiCxeA".into()))
    );
    assert_eq!(
        song.album,
        Some(AlbumRef {
            id: AlbumId("MPREb_dMHztPsrDgN".into()),
            title: "Homework".into()
        })
    );
    assert!(song.thumbnail.is_some());
}

#[test]
fn album_tracks_inherit_album_ref() {
    let id = AlbumId("MPREb_dMHztPsrDgN".into());
    let album = convert::album(&fixture("album"), &id).expect("parse");
    assert_eq!(album.summary.id, id);
    assert_eq!(album.summary.title, "Homework");
    assert_eq!(album.tracks.len(), 16);
    let want = AlbumRef {
        id: id.clone(),
        title: "Homework".into(),
    };
    for t in &album.tracks {
        assert_eq!(t.album.as_ref(), Some(&want), "{}", t.id.0);
        assert_eq!(t.artists, album.summary.artists, "{}", t.id.0);
        assert_eq!(t.thumbnail, album.summary.thumbnail, "{}", t.id.0);
        assert!(t.duration.is_some(), "{}", t.id.0);
    }
    assert!(
        album
            .tracks
            .iter()
            .any(|t| t.id == TrackId("H9gERrxL78U".into()))
    );
}

#[test]
fn radio_skips_items_without_video_id() {
    let raw = fixture("radio");
    let plain = convert::radio(&raw).expect("parse");
    assert_eq!(plain.len(), 16);

    // Add two items without a video id: an automix preview, and a panel
    // renderer whose id fields are missing.
    let mut v: Value = serde_json::from_str(&raw).unwrap();
    let items = v.pointer_mut(PANEL).unwrap().as_array_mut().unwrap();
    let mut no_id = items
        .iter()
        .find(|i| i.get("playlistPanelVideoRenderer").is_some())
        .expect("a plain panel item")
        .clone();
    let r = no_id
        .get_mut("playlistPanelVideoRenderer")
        .unwrap()
        .as_object_mut()
        .unwrap();
    r.remove("videoId");
    r.remove("navigationEndpoint");
    items.insert(
        1,
        json!({ "automixPreviewVideoRenderer": { "content": {} } }),
    );
    items.push(no_id);

    let tracks = convert::radio(&v.to_string()).expect("parse");
    assert_eq!(tracks.len(), plain.len());
    assert_eq!(tracks, plain);
}

#[test]
fn music_video_has_song_version() {
    // Radio seeded on the music video: primaries are OMVs with ATV counterparts.
    let tracks = convert::radio(&fixture("radio_mv")).expect("parse");
    let mv = tracks
        .iter()
        .find(|t| t.id == TrackId("K0HSD_i2DvA".into()))
        .expect("seed music video");
    assert_eq!(mv.video_type, VideoType::MusicVideo);
    // The song comes whole: what plays in the video's place is shown as the song.
    let song = mv.song_version().expect("song version");
    assert_eq!(song.id, TrackId("Jb6gcoR266U".into()));
    assert_eq!(song.video_type, VideoType::Song);
    assert_eq!(song.title, "Around the World");
    assert!(song.thumbnail.is_some() && song.duration.is_some());
    // The counterpart's byline is unlinked; the artist link is the video's.
    assert_eq!(song.artists, mv.artists);
    assert!(song.artists[0].id.is_some());

    // Radio seeded on the song: an ATV primary keeps no counterpart.
    let tracks = convert::radio(&fixture("radio")).expect("parse");
    let song = tracks
        .iter()
        .find(|t| t.id == TrackId("Jb6gcoR266U".into()))
        .expect("seed song");
    assert_eq!(song.video_type, VideoType::Song);
    assert_eq!(song.counterpart, None);
}

#[test]
fn a_watch_response_gives_its_seed_track() {
    let seed = TrackId("Jb6gcoR266U".into());
    let track = convert::seed_track(&fixture("radio"), &seed)
        .expect("parse")
        .expect("the seed is in its own watch queue");
    assert_eq!(track.id, seed);
    assert_eq!(track.title, "Around the World");
    assert!(track.duration.is_some());
    assert!(track.thumbnail.is_some());
    assert!(!track.artists.is_empty());
}

#[test]
fn a_music_video_seed_stays_the_video() {
    let seed = TrackId("K0HSD_i2DvA".into());
    let track = convert::seed_track(&fixture("radio_mv"), &seed)
        .expect("parse")
        .expect("seed music video");
    assert_eq!(track.id, seed);
    assert_eq!(track.video_type, VideoType::MusicVideo);
}

#[test]
fn a_watch_response_without_the_seed_gives_none() {
    let other = TrackId("zzzzzzzzzzz".into());
    assert_eq!(
        convert::seed_track(&fixture("radio"), &other).expect("parse"),
        None
    );
    // An unplayable id: the response has no queue at all.
    assert_eq!(convert::seed_track("{}", &other).expect("parse"), None);
}

/// The Home fixture holds three raw pages: the first page and one continuation, as `home`
/// fetches them, then the page after them, as Home fetches it on scroll.
fn home_fixture() -> Vec<String> {
    let pages: Vec<Value> = serde_json::from_str(&fixture("home")).expect("home fixture");
    pages.iter().map(Value::to_string).collect()
}

/// The first page and its continuation.
fn home_pages() -> Vec<String> {
    home_fixture().into_iter().take(2).collect()
}

fn home() -> Vec<HomeShelf> {
    convert::home(&home_pages()).expect("parse")
}

#[test]
fn home_parses_shelves() {
    let shelves = home();
    let titles: Vec<&str> = shelves.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(
        titles,
        [
            "Quick picks",
            "Fresh finds, old favorites",
            "Listen together",
            "Ari Abdul",
            "Isabel LaRosa",
            "Albums for you",
        ]
    );
    for s in &shelves {
        assert!(!s.items.is_empty(), "{} has no items", s.title);
    }

    let HomeItem::Track(pick) = &shelves[0].items[0] else {
        panic!("first quick pick is not a track");
    };
    assert_eq!(pick.id, TrackId("FZ_vOw0Q1gY".into()));
    assert_eq!(pick.title, "Sleepless");
    assert_eq!(pick.video_type, VideoType::Song);
    assert_eq!(pick.artists[0].name, "Dutch Melrose");
    assert_eq!(
        pick.artists[0].id,
        Some(ArtistId("UCPcIemLUpr25Ri-A4k9v7Tg".into()))
    );
    assert_eq!(
        pick.album,
        Some(AlbumRef {
            id: AlbumId("MPREb_jfaxomyyc6x".into()),
            title: "Sleepless".into()
        })
    );
    assert!(pick.thumbnail.is_some());
    assert_eq!(shelves[0].items.len(), 4);

    let playlists: Vec<_> = shelves[1]
        .items
        .iter()
        .map(|i| match i {
            HomeItem::Playlist(p) => p,
            other => panic!("not a playlist: {other:?}"),
        })
        .collect();
    assert_eq!(
        playlists[0].id,
        PlaylistId("RDTMAK5uy_mZtXeU08kxXJOUhL0ETdAuZTh1z7aAFAo".into())
    );
    assert_eq!(playlists[0].title, "Archive Mix");
    assert!(playlists[0].thumbnail.is_some());
    assert!(
        playlists
            .iter()
            .any(|p| p.id == PlaylistId("LM".into()) && p.title == "Liked Music")
    );

    // A live station: a user video, not a song.
    let HomeItem::Track(live) = &shelves[2].items[0] else {
        panic!("station is not a track");
    };
    assert_eq!(live.id, TrackId("S5CA1GnmSso".into()));
    assert_eq!(live.video_type, VideoType::UserVideo);

    let HomeItem::Playlist(community) = &shelves[3].items[0] else {
        panic!("not a playlist");
    };
    assert_eq!(community.author.as_deref(), Some("celine moradi"));

    let HomeItem::Artist(artist) = &shelves[4].items[0] else {
        panic!("not an artist");
    };
    assert_eq!(artist.id, ArtistId("UCn81HiOrP7OO3SkdGvR6HPA".into()));
    assert_eq!(artist.name, "Nessa Barrett");

    let HomeItem::Album(album) = &shelves[5].items[0] else {
        panic!("not an album");
    };
    assert_eq!(album.id, AlbumId("MPREb_Wp9Aj8HpTsB".into()));
    assert_eq!(album.title, "HIT ME HARD AND SOFT");
    assert_eq!(album.artists[0].name, "Billie Eilish");
}

/// "Music videos for you" (in the fixture's continuation, two of its cards) is left out:
/// featherplay plays only the sound. It's told by its shape, a Play all over videos on 16:9
/// thumbnails, since its title follows the UI language. Its "Play all" goes with it.
#[test]
fn home_leaves_out_the_music_videos_shelf() {
    let pages = home_pages();
    assert!(
        pages[1].contains("Music videos for you"),
        "the fixture has it"
    );
    let shelves = home();
    assert!(shelves.iter().all(|s| s.title != "Music videos for you"));
    let ids = shelves
        .iter()
        .flat_map(|s| &s.items)
        .filter_map(|i| match i {
            HomeItem::Track(t) => Some(t.id.0.as_str()),
            _ => None,
        });
    assert!(
        ids.into_iter()
            .all(|id| id != "roPiy2JydwA" && id != "bzPr09snprY")
    );
}

/// A shelf with a music video among other cards (a genre's) stays.
#[test]
fn home_keeps_a_shelf_with_other_cards_beside_music_videos() {
    let mut second: Value = serde_json::from_str(&home_pages()[1]).unwrap();
    let shelf = second
        .pointer_mut(
            "/continuationContents/sectionListContinuation/contents/1/musicCarouselShelfRenderer",
        )
        .unwrap();
    shelf["contents"][1]["musicTwoRowItemRenderer"]["aspectRatio"] =
        json!("MUSIC_TWO_ROW_ITEM_THUMBNAIL_ASPECT_RATIO_SQUARE");
    let pages = [home_pages()[0].clone(), second.to_string()];
    let shelves = convert::home(&pages).expect("parse");
    assert!(shelves.iter().any(|s| s.title == "Music videos for you"));
}

/// "Forgotten favorites" on a first page and "Music videos for you" on its continuation,
/// both every card a video on a 16:9 thumbnail, cut to a few cards. The last music video
/// is a user upload, as in a feed where the shelf still showed.
fn video_shelves_pages() -> Vec<Value> {
    serde_json::from_str(&fixture("home_video_shelves")).expect("fixture")
}

fn video_shelves_titles(pages: &[Value]) -> Vec<String> {
    let pages: Vec<String> = pages.iter().map(Value::to_string).collect();
    let shelves = convert::home(&pages).expect("parse");
    shelves.into_iter().map(|s| s.title).collect()
}

/// "Music videos for you" goes with a user upload among its music videos too: it's told by
/// the Play all its header offers (a watch endpoint), which "Forgotten favorites" doesn't.
#[test]
fn home_leaves_out_the_music_videos_shelf_with_a_user_upload() {
    let pages = video_shelves_pages();
    let last_card = "/continuationContents/sectionListContinuation/contents/0/musicCarouselShelfRenderer/contents/3/musicTwoRowItemRenderer/navigationEndpoint/watchEndpoint/watchEndpointMusicSupportedConfigs/watchEndpointMusicConfig/musicVideoType";
    assert_eq!(
        pages[1].pointer(last_card),
        Some(&json!("MUSIC_VIDEO_TYPE_UGC")),
        "the fixture has a user upload in the shelf"
    );
    assert_eq!(video_shelves_titles(&pages), ["Forgotten favorites"]);
}

/// "Forgotten favorites" stays when every card is a music video on a 16:9 thumbnail: it
/// offers no Play all.
#[test]
fn home_keeps_forgotten_favorites_of_music_videos_only() {
    let mut pages = video_shelves_pages();
    let cards = pages[0]
        .pointer_mut("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/0/musicCarouselShelfRenderer/contents")
        .and_then(Value::as_array_mut)
        .expect("Forgotten favorites' cards");
    for card in cards {
        card["musicTwoRowItemRenderer"]["navigationEndpoint"]["watchEndpoint"]["watchEndpointMusicSupportedConfigs"]
            ["watchEndpointMusicConfig"]["musicVideoType"] = json!("MUSIC_VIDEO_TYPE_OMV");
    }
    assert!(video_shelves_titles(&pages).contains(&"Forgotten favorites".to_string()));
}

#[test]
fn home_strapline_is_scrubbed() {
    let account = std::env::var("FEATHERPLAY_ACCOUNT_NAME")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let shelves = home();
    for s in &shelves {
        let Some(strap) = &s.strapline else { continue };
        if let Some(a) = &account {
            // The message names the shelf only, never the account.
            assert!(
                !strap.contains(a.as_str()),
                "{} strapline has the account name",
                s.title
            );
        }
        // A personal strapline ("Listen again" shows the account name) must be
        // scrubbed; the others are fixed labels.
        assert!(
            ["REDACTED", "STATION", "SIMILAR TO"].contains(&strap.as_str()),
            "{} has an unexpected strapline",
            s.title
        );
    }
    let straps: Vec<Option<&str>> = shelves.iter().map(|s| s.strapline.as_deref()).collect();
    assert_eq!(
        straps,
        [
            None,
            None,
            Some("STATION"),
            Some("SIMILAR TO"),
            Some("SIMILAR TO"),
            None
        ]
    );
}

#[test]
fn home_continuation_token() {
    let pages = home_pages();
    assert_eq!(
        convert::home_continuation(&pages[0]).as_deref(),
        Some("FIRST_PAGE_TOKEN")
    );
    assert_eq!(
        convert::home_continuation(&pages[1]).as_deref(),
        Some("SECOND_PAGE_TOKEN")
    );
}

#[test]
fn the_feed_points_to_the_page_after_its_last() {
    let feed = convert::home_feed(&home_pages()).expect("parse");
    assert_eq!(feed.shelves, home());
    assert_eq!(feed.more.as_deref(), Some("SECOND_PAGE_TOKEN"));
}

#[test]
fn a_later_page_reads_on_its_own_or_fails() {
    let page = &home_fixture()[2];
    let more = convert::home_more(page).expect("parse");
    assert!(!more.shelves.is_empty());
    assert_eq!(more.more, None, "the recording ends here");
    // A page that doesn't read is an error, so Home can offer to try it again.
    assert!(convert::home_more("<html>").is_err());
    assert!(convert::home_more("{\"unexpected\": 1}").is_err());
}

/// The recorded "Trending community playlists" shelf (scrubbed: the creators' names,
/// channel ids and photos are placeholders).
fn community() -> Value {
    serde_json::from_str(&home_fixture()[2]).expect("page")
}

fn community_playlists(page: &Value) -> Vec<featherplay_ytm::PlaylistSummary> {
    let more = convert::home_more(&page.to_string()).expect("parse");
    assert_eq!(more.shelves[0].title, "Trending community playlists");
    more.shelves[0]
        .items
        .iter()
        .map(|i| match i {
            HomeItem::Playlist(p) => p.clone(),
            other => panic!("not a playlist: {other:?}"),
        })
        .collect()
}

#[test]
fn a_community_playlist_keeps_its_collage_and_wears_its_creators_avatar() {
    let playlists = community_playlists(&community());
    assert_eq!(playlists.len(), 3);
    let p = &playlists[0];
    assert_eq!(p.title, "Hit 2026 - Canzoni del Momento 2026");
    assert_eq!(p.author.as_deref(), Some("Creator 1"));
    assert_eq!(p.views.as_deref(), Some("1.6M views"));
    assert!(
        p.thumbnail
            .as_deref()
            .is_some_and(|u| u.contains("fixture-collage-0"))
    );
    assert!(
        p.avatar
            .as_deref()
            .is_some_and(|u| u.contains("fixture-avatar-0=s96"))
    );
}

#[test]
fn a_community_playlist_without_an_avatar_has_none() {
    let mut page = community();
    let item = page
        .pointer_mut("/continuationContents/sectionListContinuation/contents/0/musicCarouselShelfRenderer/contents/0/musicTwoRowItemRenderer")
        .and_then(Value::as_object_mut)
        .expect("item");
    item.remove("thumbnailCornerOverlay");
    let p = &community_playlists(&page)[0];
    assert_eq!(p.avatar, None);
    assert_eq!(p.author.as_deref(), Some("Creator 1"));
    assert!(p.thumbnail.is_some());
}

#[test]
fn home_skips_unknown_items_and_empty_shelves() {
    let plain = home();
    let mut first: Value = serde_json::from_str(&home_pages()[0]).unwrap();
    let shelves = first
        .pointer_mut("/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents")
        .unwrap()
        .as_array_mut()
        .unwrap();
    let unknown = json!({ "musicMultiRowListItemRenderer": { "title": { "runs": [{ "text": "Episode" }] } } });
    shelves[0]["musicCarouselShelfRenderer"]["contents"]
        .as_array_mut()
        .unwrap()
        .insert(1, unknown.clone());
    shelves.insert(
        1,
        json!({ "musicCarouselShelfRenderer": {
            "header": { "musicCarouselShelfBasicHeaderRenderer": { "title": { "runs": [{ "text": "Podcasts" }] } } },
            "contents": [unknown]
        } }),
    );
    shelves.push(json!({ "musicTastebuilderShelfRenderer": {} }));
    let pages = [first.to_string(), home_pages()[1].clone()];
    assert_eq!(convert::home(&pages).expect("parse"), plain);
}

const HOME_SECTION: &str =
    "/contents/singleColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer";

/// Shelves of the first page only.
fn first_page_shelves() -> Vec<HomeShelf> {
    convert::home(&home_pages()[..1]).expect("first page")
}

#[test]
fn home_skips_a_shelf_it_cannot_read() {
    let plain = home();
    let mut first: Value = serde_json::from_str(&home_pages()[0]).unwrap();
    let shelves = first
        .pointer_mut(&format!("{HOME_SECTION}/contents"))
        .unwrap()
        .as_array_mut()
        .unwrap();
    // A header that isn't an object, a shelf without a title, items that aren't a list.
    shelves.insert(
        1,
        json!({ "musicCarouselShelfRenderer": { "header": "broken", "contents": [] } }),
    );
    let mut untitled = shelves[0].clone();
    untitled["musicCarouselShelfRenderer"]["header"] =
        json!({ "musicCarouselShelfBasicHeaderRenderer": {} });
    shelves.insert(2, untitled);
    shelves.insert(
        3,
        json!({ "musicImmersiveCarouselShelfRenderer": {
            "header": { "musicImmersiveCarouselShelfBasicHeaderRenderer": { "title": { "runs": [{ "text": "Odd" }] } } },
            "contents": { "not": "a list" }
        } }),
    );
    let pages = [first.to_string(), home_pages()[1].clone()];
    assert_eq!(convert::home(&pages).expect("parse"), plain);
}

/// A fresh identity (no history yet): the section list is there, with no shelves in it.
#[test]
fn home_without_shelves_is_empty_not_an_error() {
    let mut first: Value = serde_json::from_str(&home_pages()[0]).unwrap();
    let section = first.pointer_mut(HOME_SECTION).unwrap();
    section.as_object_mut().unwrap().remove("contents");
    section.as_object_mut().unwrap().remove("continuations");
    assert_eq!(convert::home(&[first.to_string()]), Ok(vec![]));

    let mut only_tastebuilder: Value = serde_json::from_str(&home_pages()[0]).unwrap();
    *only_tastebuilder
        .pointer_mut(&format!("{HOME_SECTION}/contents"))
        .unwrap() = json!([{ "musicTastebuilderShelfRenderer": {} }]);
    assert_eq!(convert::home(&[only_tastebuilder.to_string()]), Ok(vec![]));
}

/// A browse answer with nothing in it (as for a page this identity can't see) is an empty
/// Home, not an unexpected response.
#[test]
fn home_with_no_contents_is_empty() {
    let raw = fixture("playlist_unavailable");
    assert_eq!(convert::home(&[raw]), Ok(vec![]));
}

/// Only the first page decides: a continuation that doesn't read is dropped, and the
/// shelves already read are kept.
#[test]
fn home_keeps_the_first_page_when_a_continuation_is_broken() {
    let first = home_pages()[0].clone();
    let shelves = first_page_shelves();
    assert!(!shelves.is_empty());

    assert_eq!(
        convert::home(&[first.clone(), "<html>".into()]),
        Ok(shelves.clone())
    );
    let mut empty_continuation: Value = serde_json::from_str(&home_pages()[1]).unwrap();
    empty_continuation["continuationContents"]["sectionListContinuation"]
        .as_object_mut()
        .unwrap()
        .remove("contents");
    assert_eq!(
        convert::home(&[first.clone(), empty_continuation.to_string()]),
        Ok(shelves.clone())
    );
    let error = json!({ "error": { "code": 500 } }).to_string();
    assert_eq!(convert::home(&[first, error]), Ok(shelves));
}

/// A first page in a layout we don't know is still an error: it isn't shown as empty.
#[test]
fn home_in_an_unknown_layout_is_a_parse_error() {
    let raw = json!({ "contents": { "someNewBrowseResultsRenderer": {} } }).to_string();
    assert!(matches!(convert::home(&[raw]), Err(ApiError::Parse(_))));
    assert!(matches!(
        convert::home(&["<html>".into()]),
        Err(ApiError::Parse(_))
    ));
}

/// Reopening another identity's private playlist after a switch: the answer has no
/// contents at all (scrubbed recording). That's "not available", not a parse failure.
#[test]
fn a_playlist_this_identity_cannot_see_is_not_found() {
    let raw = fixture("playlist_unavailable");
    let id = PlaylistId("PLx".into());
    assert_eq!(
        convert::playlist_summary(&raw, &id),
        Err(ApiError::NotFound)
    );
    assert_eq!(convert::playlist_tracks(&[raw]), Err(ApiError::NotFound));
}

const PLAYLIST_HEADER: &str = "/contents/twoColumnBrowseResultsRenderer/tabs/0/tabRenderer/content/sectionListRenderer/contents/0/musicResponsiveHeaderRenderer";

#[test]
fn liked_music_header_has_no_author() {
    let raw = fixture("liked_songs");
    let s = convert::playlist_summary(&raw, &PlaylistId("VLLM".into())).expect("parse");
    assert_eq!(s.id, PlaylistId("LM".into()));
    assert_eq!(s.title, "Liked Music");
    assert_eq!(s.author, None);
    // The fixture shows "41 songs • 2 hours, 11 minutes".
    assert_eq!(s.count, Some(41));
    assert!(
        s.thumbnail
            .as_deref()
            .is_some_and(|u| u.ends_with("-1200.png")),
        "largest cover expected, got {:?}",
        s.thumbnail
    );
    let tracks = convert::playlist_tracks(&[raw]).expect("tracks");
    assert_eq!(tracks.len(), 15);
}

#[test]
fn header_without_author_matches_typed_parse() {
    let raw = fixture("playlist");
    let id = PlaylistId("PLx".into());
    let typed = convert::playlist_summary(&raw, &id).expect("parse");
    assert_eq!(typed.author.as_deref(), Some("Carlos Antonio"));

    let mut v: Value = serde_json::from_str(&raw).unwrap();
    let header = v.pointer_mut(PLAYLIST_HEADER).unwrap();
    let header = header.as_object_mut().unwrap();
    header.remove("facepile");
    header.remove("straplineTextOne");
    let fallback = convert::playlist_summary(&v.to_string(), &id).expect("parse without author");
    assert_eq!(fallback.author, None);
    assert_eq!(fallback.title, typed.title);
    assert_eq!(fallback.count, typed.count);
    assert_eq!(fallback.thumbnail, typed.thumbnail);
}

#[test]
fn fixtures_contain_no_redacted_leaks() {
    let account = std::env::var("FEATHERPLAY_ACCOUNT_NAME")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let mut checked = 0;
    for entry in std::fs::read_dir(fixtures_dir()).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(
            !text.contains("@gmail.com"),
            "{name} contains an email address"
        );
        if let Some(a) = &account {
            // The message names the file only, never the account.
            assert!(
                !text.contains(a.as_str()),
                "{name} contains the account name"
            );
        }
        serde_json::from_str::<Value>(&text).unwrap_or_else(|_| panic!("{name} is not valid JSON"));
        checked += 1;
    }
    assert!(
        checked >= 9,
        "expected the recorded fixtures, found {checked}"
    );
}

#[test]
fn account_info_reads_the_menu_header() {
    let info = convert::account_info(&fixture("account_menu")).expect("parse");
    // The recording is scrubbed: placeholders only.
    assert_eq!(info.name, "Test Listener");
    // The recorded account has no channel, so no handle.
    assert_eq!(info.handle, None);
    assert_eq!(
        info.photo.as_deref(),
        Some("https://yt3.ggpht.com/placeholder-account-photo=s108-c-k-c0x00ffffff-no-rj")
    );
}

#[test]
fn account_info_reads_handle_and_widest_photo() {
    let mut v: Value = serde_json::from_str(&fixture("account_menu")).unwrap();
    let header = v
        .pointer_mut(
            "/actions/0/openPopupAction/popup/multiPageMenuRenderer/header/activeAccountHeaderRenderer",
        )
        .unwrap();
    header["channelHandle"] = json!({"runs": [{"text": "@testlistener"}]});
    header["accountPhoto"]["thumbnails"] = json!([
        {"url": "https://yt3.ggpht.com/p=s48", "width": 48, "height": 48},
        {"url": "https://yt3.ggpht.com/p=s176", "width": 176, "height": 176},
        {"url": "https://yt3.ggpht.com/p=s88", "width": 88, "height": 88}
    ]);
    let info = convert::account_info(&v.to_string()).expect("parse");
    assert_eq!(info.handle.as_deref(), Some("@testlistener"));
    assert_eq!(info.photo.as_deref(), Some("https://yt3.ggpht.com/p=s176"));
}

#[test]
fn account_info_signed_out_is_auth_expired() {
    let signed_out = json!({
        "responseContext": {"serviceTrackingParams": [
            {"service": "GFEEDBACK", "params": [{"key": "logged_in", "value": "0"}]}
        ]},
        "actions": []
    });
    assert_eq!(
        convert::account_info(&signed_out.to_string()),
        Err(featherplay_ytm::ApiError::AuthExpired)
    );
    let unknown = json!({"actions": []});
    assert!(matches!(
        convert::account_info(&unknown.to_string()),
        Err(featherplay_ytm::ApiError::Parse(_))
    ));
}

#[test]
fn accounts_lists_every_identity_once_in_switcher_order() {
    use featherplay_ytm::Identity;
    // The endpoint answers with the anti-hijacking prefix; the fixture is stored without it.
    let raw = format!(")]}}'\n{}", fixture("account_switcher"));
    let list = convert::accounts(&raw, &Identity::default()).expect("parse");
    let keys: Vec<String> = list.iter().map(|a| a.identity.key()).collect();
    // The active account's own section first, then each Google account's identities.
    assert_eq!(keys, ["2", "0", "0/100000000000000000002", "1"]);
    let names: Vec<&str> = list.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Test Listener",
            "Test Listener",
            "Test Channel",
            "Second Listener"
        ]
    );
    // Only the active identity carries its handle; the others have a byline.
    assert_eq!(list[0].handle.as_deref(), Some("@testlistener"));
    assert!(list[1..].iter().all(|a| a.handle.is_none()));
    assert_eq!(list[1].byline.as_deref(), Some("No channel"));
    assert!(!list[1].has_channel);
    assert_eq!(list[2].byline.as_deref(), Some("12 subscribers"));
    assert!(list[2].has_channel);
    assert_eq!(
        list[2].photo.as_deref(),
        Some("https://yt3.ggpht.com/placeholder-account-c=s36-c-k-c0x00ffffff-no-rj")
    );
    // Sent without an identity: the server's selection is the active one.
    let active: Vec<bool> = list.iter().map(|a| a.active).collect();
    assert_eq!(active, [true, false, false, false]);
    // Debug never shows the names.
    assert!(!format!("{list:?}").contains("Listener"));
}

#[test]
fn accounts_marks_the_identity_sent_as_active() {
    use featherplay_ytm::Identity;
    let brand = Identity::new(0, Some("100000000000000000002")).unwrap();
    let list = convert::accounts(&fixture("account_switcher"), &brand).expect("parse");
    let active: Vec<String> = list
        .iter()
        .filter(|a| a.active)
        .map(|a| a.identity.key())
        .collect();
    assert_eq!(active, ["0/100000000000000000002"]);
    // An identity the switcher doesn't list falls back to the server's selection.
    let gone = Identity::new(7, None).unwrap();
    let list = convert::accounts(&fixture("account_switcher"), &gone).expect("parse");
    assert!(list[0].active && list.iter().filter(|a| a.active).count() == 1);
}

#[test]
fn accounts_skips_disabled_items_and_reads_signed_out() {
    use featherplay_ytm::{ApiError, Identity};
    let mut v: Value = serde_json::from_str(&fixture("account_switcher")).unwrap();
    let item = v
        .pointer_mut("/data/actions/0/getMultiPageMenuAction/menu/multiPageMenuRenderer/sections/2/accountSectionListRenderer/contents/0/accountItemSectionRenderer/contents/0/accountItem")
        .unwrap();
    item["isDisabled"] = json!(true);
    let list = convert::accounts(&v.to_string(), &Identity::default()).expect("parse");
    assert_eq!(list.len(), 3);
    assert!(list.iter().all(|a| a.identity.key() != "1"));

    let mut out: Value = serde_json::from_str(&fixture("account_switcher")).unwrap();
    out["data"]["responseContext"]["mainAppWebResponseContext"]["loggedOut"] = json!(true);
    assert_eq!(
        convert::accounts(&out.to_string(), &Identity::default()),
        Err(ApiError::AuthExpired)
    );
    assert!(matches!(
        convert::accounts(r#"{"code":"SUCCESS","data":{}}"#, &Identity::default()),
        Err(ApiError::Parse(_))
    ));
}

#[test]
fn identity_keys_round_trip_and_reject_edits() {
    use featherplay_ytm::Identity;
    let brand = Identity::new(0, Some("100000000000000000002")).unwrap();
    assert_eq!(Identity::from_key(&brand.key()), Some(brand.clone()));
    assert_eq!(brand.auth_user(), Some(0));
    assert_eq!(brand.page_id(), Some("100000000000000000002"));
    let second = Identity::new(2, None).unwrap();
    assert_eq!(second.key(), "2");
    assert_eq!(Identity::from_key("2"), Some(second));
    assert_eq!(Identity::from_key(""), Some(Identity::default()));
    assert!(Identity::default().is_default());
    for bad in [
        "x",
        "-1",
        "0/",
        "0/12a",
        "1000",
        "0/1/2",
        "/5",
        "0/1\r\nX: y",
    ] {
        assert_eq!(Identity::from_key(bad), None, "{bad:?}");
    }
    // Debug shows the shape only, never the page id.
    assert!(!format!("{brand:?}").contains("1000"));
}

#[test]
fn plain_lyrics_come_from_the_watch_next_lyrics_tab_and_its_browse_page() {
    use featherplay_ytm::PlainLyrics;
    // The recorded watch-next response has a Lyrics tab among Up next, Comments and Related.
    let next = fixture("radio");
    assert_eq!(
        convert::lyrics_browse_id(&next).expect("parse"),
        Some("MPLYt_dMHztPsrDgN-7".to_string())
    );
    // A track without lyrics: the tab is there, unselectable, with no endpoint.
    let mut v: Value = serde_json::from_str(&next).unwrap();
    let tab = v
        .pointer_mut("/contents/singleColumnMusicWatchNextResultsRenderer/tabbedRenderer/watchNextTabbedResultsRenderer/tabs/1/tabRenderer")
        .unwrap();
    tab.as_object_mut().unwrap().remove("endpoint");
    tab["unselectable"] = json!(true);
    assert_eq!(
        convert::lyrics_browse_id(&v.to_string()).expect("parse"),
        None
    );

    // The lyrics browse page, in the shape ytmusicapi reads.
    let page = json!({"contents": {"sectionListRenderer": {"contents": [
        {"musicDescriptionShelfRenderer": {
            "description": {"runs": [{"text": "First line\nSecond line"}]},
            "footer": {"runs": [{"text": "Source: LyricFind"}]}
        }}
    ]}}});
    assert_eq!(
        convert::lyrics(&page.to_string()).expect("parse"),
        Some(PlainLyrics {
            text: "First line\nSecond line".into(),
            source: Some("Source: LyricFind".into()),
        })
    );
    let none = json!({"contents": {"messageRenderer": {"text": {"runs": [{"text": "Lyrics not available"}]}}}});
    assert_eq!(convert::lyrics(&none.to_string()).expect("parse"), None);
}

#[test]
fn timed_lyrics_come_from_the_mobile_lyrics_page_in_order_of_their_start() {
    // Recorded from the mobile client's Lyrics tab page, the words replaced by placeholders.
    let timed = convert::timed_lyrics(&fixture("timed_lyrics"))
        .expect("parse")
        .expect("timed lines");
    assert_eq!(timed.lines.len(), 51);
    assert_eq!(timed.lines[0].start, Duration::from_millis(250));
    assert_eq!(timed.lines[0].text, "Placeholder line 1");
    assert_eq!(timed.lines[50].start, Duration::from_millis(269_750));
    assert!(timed.lines.windows(2).all(|w| w[0].start <= w[1].start));
    assert_eq!(timed.source.as_deref(), Some("Source: Musixmatch"));
}

#[test]
fn untimed_mobile_lyrics_are_not_timed_lyrics() {
    // Most songs: the same lines with no cueRange, laid out static.
    let page = json!({"contents": {"elementRenderer": {"newElement": {"type": {"componentType": {
        "model": {"timedLyricsModel": {"lyricsData": {
            "timedLyricsData": [{"lyricLine": "First line"}, {"lyricLine": "Second line"}],
            "sourceMessage": "Source: LyricFind",
            "staticLayout": true
        }}}
    }}}}}});
    assert_eq!(
        convert::timed_lyrics(&page.to_string()).expect("parse"),
        None
    );
    let none = json!({"contents": {"messageRenderer": {"text": {"runs": [{"text": "Lyrics not available"}]}}}});
    assert_eq!(
        convert::timed_lyrics(&none.to_string()).expect("parse"),
        None
    );
}

const ALBUM_ROWS: &str = "/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents/0/musicShelfRenderer/contents";

/// An album row without a duration column (or a plays column) is kept without a length;
/// the album's other rows are unchanged.
#[test]
fn an_album_row_without_a_duration_is_kept_without_one() {
    let id = AlbumId("MPREb_dMHztPsrDgN".into());
    let raw = fixture("album");
    let plain = convert::album(&raw, &id).expect("parse");
    let mut v: Value = serde_json::from_str(&raw).unwrap();
    let rows = v.pointer_mut(ALBUM_ROWS).unwrap().as_array_mut().unwrap();
    let r = rows[3]["musicResponsiveListItemRenderer"]
        .as_object_mut()
        .unwrap();
    r.remove("fixedColumns");
    r["flexColumns"].as_array_mut().unwrap().truncate(1);
    let album = convert::album(&v.to_string(), &id).expect("parse");
    assert_eq!(album.summary, plain.summary);
    assert_eq!(album.tracks.len(), plain.tracks.len());
    assert_eq!(album.tracks[3].id, plain.tracks[3].id);
    assert_eq!(album.tracks[3].duration, None);
    assert_eq!(album.tracks[4], plain.tracks[4]);
}

const PLAYLIST_ROWS: &str = "/contents/twoColumnBrowseResultsRenderer/secondaryContents/sectionListRenderer/contents/0/musicPlaylistShelfRenderer/contents";

/// A collaborative playlist (recorded 2026-10-30, scrubbed): its rows carry votes and
/// contributor avatars, and the duration is the third flex column. There is no fixed
/// column at all, which failed the whole playlist.
#[test]
fn a_collaborative_playlist_reads_rows_without_a_fixed_column() {
    let raw = fixture("playlist_collaborative");
    let tracks = convert::playlist_tracks(&[raw]).expect("parse");
    let ids: Vec<&str> = tracks.iter().map(|t| t.id.0.as_str()).collect();
    assert_eq!(ids, ["xJtqXz521mM", "I1OzfxybATE", "5khS_KwMC_g"]);
    let secs: Vec<Option<u64>> = tracks
        .iter()
        .map(|t| t.duration.map(|d| d.as_secs()))
        .collect();
    assert_eq!(secs, [Some(221), Some(268), Some(231)]);
    let first = &tracks[0];
    assert_eq!(first.title, "오늘부터 우리는 (Me Gustas tu)");
    assert_eq!(first.video_type, VideoType::Song);
    assert_eq!(first.artists.len(), 1);
    assert_eq!(first.artists[0].name, "GFRIEND");
    assert_eq!(
        first.artists[0].id,
        Some(ArtistId("UCKrKo2mifcYmM86K_suNz2Q".into()))
    );
    assert_eq!(
        first.album.as_ref().map(|a| a.id.clone()),
        Some(AlbumId("MPREb_JqteeWRhoVl".into()))
    );
    assert!(first.thumbnail.is_some());
}

#[test]
fn a_collaborative_playlist_header_reads() {
    let raw = fixture("playlist_collaborative");
    let s = convert::playlist_summary(&raw, &PlaylistId("VLPLzzTestColab".into())).expect("parse");
    assert_eq!(s.id, PlaylistId("PLzzTestColab".into()));
    assert_eq!(s.title, "Test Playlist");
    assert_eq!(s.count, Some(3));
}

/// An album's own playlist (`OLAK5uy_…`, recorded 2026-10-31 for pasted links, scrubbed of
/// its response context): no header at all, only the rows. The summary comes from them.
#[test]
fn an_album_playlist_without_a_header_reads_from_its_rows() {
    let raw = fixture("playlist_album");
    let id = PlaylistId("OLAK5uy_kfP7kUsS6hmTXdnO5JlTVRSn-LLt1bqtM".into());
    let tracks = convert::playlist_tracks(std::slice::from_ref(&raw)).expect("rows");
    assert_eq!(tracks.len(), 13);
    let s = convert::playlist_summary(&raw, &id).expect("summary");
    assert_eq!(s.id, id);
    assert_eq!(s.title, "OutRun");
    assert_eq!(s.author.as_deref(), Some("Kavinsky"));
    assert_eq!(s.count, Some(13));
    assert!(s.thumbnail.is_some());
}

/// A playlist page with neither a header nor rows is still a parse error.
#[test]
fn a_page_without_header_or_rows_is_a_parse_error() {
    let mut v: Value = serde_json::from_str(&fixture("playlist_album")).unwrap();
    v.pointer_mut(PLAYLIST_ROWS)
        .unwrap()
        .as_array_mut()
        .unwrap()
        .clear();
    let id = PlaylistId("OLAK5uy_x".into());
    assert!(matches!(
        convert::playlist_summary(&v.to_string(), &id),
        Err(ApiError::Parse(_))
    ));
}

/// Removes every duration from row `i`: the fixed column, and a flex column that reads
/// as a duration.
fn strip_duration(rows: &mut [Value], i: usize) {
    let r = rows[i]["musicResponsiveListItemRenderer"]
        .as_object_mut()
        .unwrap();
    r.remove("fixedColumns");
    let flex = r["flexColumns"].as_array_mut().unwrap();
    flex.retain(|c| {
        let t = c
            .pointer("/musicResponsiveListItemFlexColumnRenderer/text/runs/0/text")
            .and_then(Value::as_str)
            .unwrap_or_default();
        convert::parse_duration(t).is_none()
    });
}

/// A row with no duration anywhere is still a track, shown without a length.
#[test]
fn a_row_without_any_duration_is_kept_without_one() {
    for name in ["playlist", "playlist_collaborative"] {
        let raw = fixture(name);
        let plain = convert::playlist_tracks(std::slice::from_ref(&raw)).expect("parse");
        let mut v: Value = serde_json::from_str(&raw).unwrap();
        strip_duration(
            v.pointer_mut(PLAYLIST_ROWS)
                .unwrap()
                .as_array_mut()
                .unwrap(),
            1,
        );
        let tracks = convert::playlist_tracks(&[v.to_string()]).expect(name);
        assert_eq!(tracks.len(), plain.len(), "{name}");
        assert_eq!(tracks[1].id, plain[1].id, "{name}");
        assert_eq!(tracks[1].duration, None, "{name}");
        assert_eq!(tracks[1].album, plain[1].album, "{name}");
        assert_eq!(tracks[0], plain[0], "{name}");
    }
}

/// A greyed-out row is skipped, as before; the others stay.
#[test]
fn a_greyed_out_row_is_skipped() {
    let raw = fixture("playlist");
    let plain = convert::playlist_tracks(std::slice::from_ref(&raw)).expect("parse");
    let mut v: Value = serde_json::from_str(&raw).unwrap();
    let rows = v
        .pointer_mut(PLAYLIST_ROWS)
        .unwrap()
        .as_array_mut()
        .unwrap();
    rows[2]["musicResponsiveListItemRenderer"]["musicItemRendererDisplayPolicy"] =
        json!("MUSIC_ITEM_RENDERER_DISPLAY_POLICY_GREY_OUT");
    let tracks = convert::playlist_tracks(&[v.to_string()]).expect("parse");
    assert_eq!(tracks.len(), plain.len() - 1);
    assert!(tracks.iter().all(|t| t.id != plain[2].id));
}

/// The first page ends with a continuation item; the next page comes as an
/// `appendContinuationItemsAction`, here with a row that has no duration. Both pages'
/// rows are read, in order, and the token is found on each page.
#[test]
fn a_continuation_with_a_row_without_duration_reads() {
    let mut first: Value = serde_json::from_str(&fixture("playlist")).unwrap();
    let first_rows = first
        .pointer_mut(PLAYLIST_ROWS)
        .unwrap()
        .as_array_mut()
        .unwrap();
    let first_count = first_rows.len();
    first_rows.push(json!({ "continuationItemRenderer": { "continuationEndpoint": {
        "continuationCommand": { "token": "NEXT_PAGE_TOKEN", "request": "CONTINUATION_REQUEST_TYPE_BROWSE" }
    } } }));

    let liked: Value = serde_json::from_str(&fixture("liked_songs")).unwrap();
    let mut rows = liked
        .pointer(PLAYLIST_ROWS)
        .unwrap()
        .as_array()
        .unwrap()
        .clone();
    strip_duration(&mut rows, 0);
    let more = rows.len();
    let second = json!({ "onResponseReceivedActions": [ { "appendContinuationItemsAction": {
        "continuationItems": rows
    } } ] });

    let (first, second) = (first.to_string(), second.to_string());
    assert_eq!(
        convert::playlist_continuation(&first).as_deref(),
        Some("NEXT_PAGE_TOKEN")
    );
    assert_eq!(convert::playlist_continuation(&second), None);

    let tracks = convert::playlist_tracks(&[first, second]).expect("parse");
    assert_eq!(tracks.len(), first_count + more);
    let liked_first = convert::playlist_tracks(&[liked.to_string()]).expect("liked")[0].clone();
    let joined = &tracks[first_count];
    assert_eq!(joined.id, liked_first.id);
    assert_eq!(joined.duration, None);
}

/// A library grid item, as the library's playlists page lists it, with `menu` as its
/// menu's items.
fn library_item(id: &str, title: &str, menu: Value) -> Value {
    json!({ "musicTwoRowItemRenderer": {
        "title": { "runs": [ { "text": title, "navigationEndpoint": {
            "browseEndpoint": { "browseId": format!("VL{id}") } } } ] },
        "subtitle": { "runs": [ { "text": "12 tracks" } ] },
        "menu": { "menuRenderer": { "items": menu } }
    } })
}

/// The library's playlists page with these grid items.
fn library_page(items: Vec<Value>) -> String {
    json!({ "contents": { "singleColumnBrowseResultsRenderer": { "tabs": [ { "tabRenderer": {
        "content": { "sectionListRenderer": { "contents": [ { "gridRenderer": { "items": items } } ] } }
    } } ] } } })
    .to_string()
}

/// The library toggle a saved playlist's menu carries ("Remove playlist from library"),
/// shaped as search results have it (search_playlists.json).
fn library_toggle(id: &str) -> Value {
    json!({ "toggleMenuServiceItemRenderer": {
        "defaultIcon": { "iconType": "BOOKMARK_BORDER" },
        "defaultServiceEndpoint": { "likeEndpoint": { "status": "LIKE", "target": { "playlistId": id } } },
        "isToggled": true,
        "toggledIcon": { "iconType": "BOOKMARK" },
        "toggledServiceEndpoint": { "likeEndpoint": { "status": "INDIFFERENT", "target": { "playlistId": id } } }
    } })
}

#[test]
fn a_library_playlist_with_a_library_toggle_is_saved_not_owned() {
    let page = library_page(vec![library_item(
        "PLsaved",
        "Someone's mix",
        json!([library_toggle("PLsaved")]),
    )]);
    let playlists = convert::library_playlists(&[page]).expect("parse");
    assert!(!playlists[0].owned);
}

#[test]
fn a_library_playlist_without_a_library_toggle_is_the_accounts_own() {
    let delete = json!({ "menuNavigationItemRenderer": {
        "icon": { "iconType": "DELETE" },
        "navigationEndpoint": { "confirmDialogEndpoint": {} }
    } });
    let page = library_page(vec![library_item("PLmine", "Mine", json!([delete]))]);
    let playlists = convert::library_playlists(&[page]).expect("parse");
    assert!(playlists[0].owned);
}

#[test]
fn a_playlists_radio_is_its_bare_id_after_rdampl() {
    assert_eq!(convert::playlist_radio_id("PLabc"), "RDAMPLPLabc");
    assert_eq!(convert::playlist_radio_id("VLPLabc"), "RDAMPLPLabc");
}

#[test]
fn search_results_are_never_owned() {
    let found = convert::search_playlists(&fixture("search_playlists")).expect("parse");
    assert!(found.iter().all(|p| !p.owned));
}

/// "No results for …" (the Artists tab for "in love with myself", recorded signed out
/// 2026-11-06): an empty tab, not an unexpected answer.
#[test]
fn a_search_with_no_results_is_empty() {
    let raw = fixture("search_no_results");
    assert_eq!(convert::search_artists(&raw), Ok(Vec::new()));
    assert_eq!(convert::search_songs(&raw), Ok(Vec::new()));
}

// ---------------------------------------------------------------------------
// Explore and the history. New releases, charts, moods and a mood's playlists were recorded
// signed out (2026-10-30, so the charts are the IP's country); the history is ytmapi-rs's
// public `get_history_20240713.json` (ytmapi-rs, MIT, (c) nick42d), trimmed to six rows a
// day; it holds no account data.

#[test]
fn new_releases_are_albums_and_singles_then_music_videos() {
    let shelves = convert::new_releases(&fixture("explore_new_releases")).expect("parse");
    let titles: Vec<&str> = shelves.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(titles, ["Albums & singles", "Music videos"]);
    assert!(
        shelves[0]
            .items
            .iter()
            .all(|i| matches!(i, HomeItem::Album(_)))
    );
    let HomeItem::Album(first) = &shelves[0].items[0] else {
        unreachable!()
    };
    assert_eq!(first.title, "Te Amo");
    assert_eq!(first.artists[0].name, "DJ TYNA");
    assert!(
        shelves[1]
            .items
            .iter()
            .all(|i| matches!(i, HomeItem::Track(t) if t.video_type == VideoType::MusicVideo))
    );
}

#[test]
fn charts_name_the_country_shown_and_offer_the_others_by_code() {
    let charts = convert::charts(&fixture("explore_charts")).expect("parse");
    assert_eq!(charts.country.as_deref(), Some("IT"));
    let codes: Vec<&str> = charts.countries.iter().map(|c| c.code.as_str()).collect();
    assert_eq!(&codes[..3], ["IT", "ZZ", "AR"]);
    assert_eq!(charts.countries[1].name, "Global");
    assert_eq!(charts.countries.last().map(|c| c.code.as_str()), Some("ZW"));
}

#[test]
fn charts_have_the_video_charts_as_playlists() {
    let charts = convert::charts(&fixture("explore_charts")).expect("parse");
    assert_eq!(charts.shelves.len(), 1);
    let shelf = &charts.shelves[0];
    assert_eq!(shelf.title, "Video charts");
    let ids: Vec<&str> = shelf
        .items
        .iter()
        .map(|i| match i {
            HomeItem::Playlist(p) => p.id.0.as_str(),
            _ => "?",
        })
        .collect();
    assert_eq!(ids.len(), 3);
    assert!(ids.iter().all(|id| !id.starts_with("VL")), "{ids:?}");
}

#[test]
fn chart_artists_come_ranked_with_their_trend() {
    let charts = convert::charts(&fixture("explore_charts")).expect("parse");
    assert_eq!(charts.artists.len(), 20);
    let first = &charts.artists[0];
    assert_eq!(first.rank, 1);
    assert_eq!(first.artist.name, "Geolier");
    assert_eq!(first.artist.id, ArtistId("UCcVStX6y07F8-euPi0A7hLA".into()));
    assert_eq!(first.subscribers.as_deref(), Some("1.35M subscribers"));
    assert_eq!(first.trend, Trend::Same);
    assert!(first.artist.thumbnail.is_some());
    let ranks: Vec<u32> = charts.artists.iter().map(|a| a.rank).collect();
    assert_eq!(ranks, (1..=20).collect::<Vec<_>>());
    assert!(charts.artists.iter().any(|a| a.trend == Trend::Up));
    assert!(charts.artists.iter().any(|a| a.trend == Trend::Down));
}

#[test]
fn moods_come_in_their_sections_with_the_params_that_open_them() {
    let sections = convert::moods(&fixture("explore_moods")).expect("parse");
    let titles: Vec<&str> = sections.iter().map(|s| s.title.as_str()).collect();
    assert_eq!(titles, ["Moods & moments", "Genres"]);
    assert_eq!(sections[0].moods.len(), 12);
    assert_eq!(sections[1].moods.len(), 25);
    assert_eq!(sections[0].moods[0].title, "Chill");
    assert_eq!(sections[0].moods[0].params, "ggMPOg1uX1JOQWZFeDByc2Jm");
}

#[test]
fn a_moods_page_has_its_title_and_shelves_of_playlists() {
    let page = convert::mood_playlists(&fixture("explore_mood_playlists")).expect("parse");
    assert_eq!(page.title, "Chill");
    assert_eq!(page.shelves.len(), 4);
    assert_eq!(page.shelves[0].title, "Coffee shop blends");
    let HomeItem::Playlist(p) = &page.shelves[0].items[0] else {
        panic!("not a playlist");
    };
    assert_eq!(p.title, "Coffee Shop Blend");
    assert_eq!(
        p.id,
        PlaylistId("RDCLAK5uy_nBE4bLuBHUXWZrF59ZrkPEToKt8M_I3Vc".into())
    );
}

#[test]
fn the_history_is_grouped_by_day_newest_first() {
    let days = convert::history(&fixture("history")).expect("parse");
    let titles: Vec<&str> = days.iter().map(|d| d.title.as_str()).collect();
    assert_eq!(
        titles,
        ["Today", "Yesterday", "This week", "Last week", "June 2024"]
    );
    // Six rows a day, but yesterday's first is an upload, which can't play here.
    let counts: Vec<usize> = days.iter().map(|d| d.tracks.len()).collect();
    assert_eq!(counts, [6, 5, 6, 6, 6]);
    let first = &days[0].tracks[0];
    assert_eq!(first.id, TrackId("UofrSj07rnI".into()));
    assert_eq!(first.title, "Backwards Marathon");
    assert_eq!(first.artists[0].name, "Between The Buried And Me");
    assert_eq!(
        first.album.as_ref().map(|a| a.title.as_str()),
        Some("Alaska")
    );
    assert_eq!(first.duration, Some(Duration::from_secs(8 * 60 + 28)));
}

/// The history page signed out: a message asking to sign in, and no shelves.
fn signed_out_history() -> String {
    json!({
        "contents": { "singleColumnBrowseResultsRenderer": { "tabs": [{ "tabRenderer": {
            "content": { "sectionListRenderer": { "contents": [{ "itemSectionRenderer": {
                "contents": [{ "messageRenderer": {
                    "text": { "runs": [{ "text": "Sign in to view your history" }] }
                } }]
            } }] } }
        } }] } }
    })
    .to_string()
}

#[test]
fn a_signed_out_history_is_auth_expired() {
    assert_eq!(
        convert::history(&signed_out_history()),
        Err(ApiError::AuthExpired)
    );
}

#[test]
fn an_empty_history_has_no_days() {
    let page = json!({
        "contents": { "singleColumnBrowseResultsRenderer": { "tabs": [{ "tabRenderer": {
            "content": { "sectionListRenderer": { "contents": [] } }
        } }] } }
    });
    assert_eq!(convert::history(&page.to_string()), Ok(Vec::new()));
}
