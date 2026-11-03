//! Live checks against the secondary test account. Run with
//! `FEATHERPLAY_COOKIES_FILE=<path> cargo test -p featherplay-ytm -- --ignored`.
//!
//! Each request runs once, nothing retries. The only write is one like on the
//! test track, which is always reverted to indifferent.

use featherplay_core::model::{LikeStatus, TrackId, VideoType};
use featherplay_ytm::{SearchFilter, SongLyrics, YtMusic};

const SONG: &str = "Jb6gcoR266U";
/// "Daftendirekt" on Homework: the one track the live test may rate.
const RATE_TRACK: &str = "H9gERrxL78U";

async fn connect() -> YtMusic {
    let path =
        std::env::var("FEATHERPLAY_COOKIES_FILE").expect("FEATHERPLAY_COOKIES_FILE is not set");
    let cookies = std::fs::read_to_string(&path).expect("cookie file is not readable");
    YtMusic::from_cookie_header(cookies.trim())
        .await
        .expect("sign in with the cookie header")
}

/// The app's own sign-in check, and nothing more: the page (for the client version), then
/// one library read. Two requests, both reads.
#[tokio::test]
#[ignore = "live: needs FEATHERPLAY_COOKIES_FILE"]
async fn live_sign_in_check() {
    let yt = connect().await;
    let playlists = yt.library_playlists().await.expect("library playlists");
    println!("signed in: {} library playlists", playlists.len());
}

#[tokio::test]
#[ignore = "live: needs FEATHERPLAY_COOKIES_FILE"]
async fn live_reads() {
    let yt = connect().await;

    let found = yt
        .search("Daft Punk Around the World", SearchFilter::Songs)
        .await
        .expect("search");
    let song = found
        .songs
        .iter()
        .find(|t| t.id.0 == SONG)
        .expect("Around the World in results");
    assert_eq!(song.video_type, VideoType::Song);
    println!("search: {} songs", found.songs.len());

    let liked = yt.liked_songs().await.expect("liked songs");
    assert!(!liked.is_empty(), "liked songs is empty (signed out?)");
    println!("liked songs: {}", liked.len());

    let radio = yt.radio(&TrackId(SONG.into())).await.expect("radio");
    assert!(radio.len() >= 10, "radio gave {} tracks", radio.len());
    println!("radio: {} tracks", radio.len());

    // Counts only: a strapline ("Listen again") can carry the account name.
    let home = yt.home().await.expect("home").shelves;
    assert!(home.len() >= 3, "home gave {} shelves", home.len());
    let items: usize = home.iter().map(|s| s.items.len()).sum();
    println!("home: {} shelves, {items} items", home.len());

    // Library lists have no fixtures; check they parse. Counts only.
    let playlists = yt.library_playlists().await.expect("library playlists");
    let albums = yt.library_albums().await.expect("library albums");
    let artists = yt.library_artists().await.expect("library artists");
    println!(
        "library: {} playlists, {} albums, {} artists",
        playlists.len(),
        albums.len(),
        artists.len()
    );
}

#[tokio::test]
#[ignore = "live: needs FEATHERPLAY_COOKIES_FILE; likes then reverts one track"]
async fn live_like_then_revert() {
    let yt = connect().await;
    let found = yt
        .search("Daft Punk Around the World", SearchFilter::Songs)
        .await
        .expect("search");
    let album_id = found
        .songs
        .iter()
        .find(|t| t.id.0 == SONG)
        .and_then(|t| t.album.clone())
        .expect("song with album")
        .id;
    let like_of = |album: &featherplay_ytm::Album| {
        album
            .tracks
            .iter()
            .find(|t| t.id.0 == RATE_TRACK)
            .map(|t| t.like)
            .expect("test track on the album")
    };

    let before = like_of(&yt.album(&album_id).await.expect("album"));
    assert_eq!(
        before,
        LikeStatus::Indifferent,
        "test track is not indifferent; leaving it alone"
    );

    let id = TrackId(RATE_TRACK.into());
    let liked = yt.rate(&id, LikeStatus::Like).await;
    // Revert exactly once, even if the like failed.
    let reverted = yt.rate(&id, LikeStatus::Indifferent).await;
    liked.expect("like");
    reverted.expect("revert to indifferent");

    let after = like_of(&yt.album(&album_id).await.expect("album after revert"));
    assert_eq!(
        after,
        LikeStatus::Indifferent,
        "revert did not stick: check manually"
    );
    println!("rate: like then revert ok, state after = {after:?}");
}

/// The sidebar menu's writes on a throwaway playlist, never on one of the user's: creates
/// it, renames it, then deletes it (three writes), with a library read after each step.
/// `FEATHERPLAY_IDENTITY` (`authuser/pageid`, as settings.toml has it) acts as that
/// identity; without it, the cookie's default one. Prints counts and outcomes only.
#[tokio::test]
#[ignore = "live: needs FEATHERPLAY_COOKIES_FILE; creates, renames and deletes a throwaway playlist"]
async fn live_playlist_rename_then_delete() {
    let mut yt = connect().await;
    if let Ok(value) = std::env::var("FEATHERPLAY_IDENTITY")
        && let Some((user, page)) = value.split_once('/')
    {
        let identity = featherplay_ytm::Identity::new(user.parse().expect("authuser"), Some(page))
            .expect("identity");
        yt = yt.as_identity(&identity).expect("client");
    }
    const NAME: &str = "featherplay live test (safe to delete)";
    const RENAMED: &str = "featherplay live test, renamed (safe to delete)";

    let id = yt.create_playlist(NAME).await.expect("create");
    // From here on, delete exactly once whatever happens in between.
    let renamed = yt.rename_playlist(&id, RENAMED).await;
    let after_rename = yt.library_playlists().await;
    let deleted = yt.delete_playlist(&id).await;
    let after_delete = yt.library_playlists().await;

    renamed.expect("rename");
    deleted.expect("delete: remove the test playlist by hand");
    let listed = after_rename.expect("library after the rename");
    let mine = listed.iter().find(|p| p.id == id);
    assert_eq!(
        mine.map(|p| p.title.as_str()),
        Some(RENAMED),
        "the renamed playlist isn't in the library under its new name"
    );
    assert!(
        mine.is_some_and(|p| p.owned),
        "the account's own playlist reads as saved"
    );
    let gone = !after_delete
        .expect("library after the delete")
        .iter()
        .any(|p| p.id == id);
    assert!(gone, "the deleted playlist is still listed: check by hand");
    let saved = listed.iter().filter(|p| !p.owned).count();
    println!(
        "playlist: created, renamed, deleted; library had {} playlists, {saved} saved",
        listed.len()
    );
}

/// One read of the account menu. With `FEATHERPLAY_RECORD_DIR` set, its raw JSON is
/// written there as `account_menu.raw.json`, unscrubbed: scrub it before it becomes the
/// fixture. Prints which fields are present, never their values.
#[tokio::test]
#[ignore = "live: needs FEATHERPLAY_COOKIES_FILE"]
async fn live_account_info() {
    let yt = connect().await;
    let raw = yt.account_menu_raw().await.expect("account menu");
    if let Some(dir) = std::env::var_os("FEATHERPLAY_RECORD_DIR") {
        let path = std::path::Path::new(&dir).join("account_menu.raw.json");
        std::fs::write(&path, &raw).expect("write the recording");
        println!("recorded {} bytes", raw.len());
    }
    let info = featherplay_ytm::convert::account_info(&raw).expect("account info");
    assert!(!info.name.is_empty(), "no account name");
    println!(
        "account: name {} chars, handle {}, photo {}",
        info.name.chars().count(),
        if info.handle.is_some() { "yes" } else { "no" },
        if info.photo.is_some() { "yes" } else { "no" },
    );
}

/// One read of the search suggestions for a partial query. With `FEATHERPLAY_RECORD_DIR`
/// set, its raw JSON is written there as `search_suggestions.raw.json`, unscrubbed: a
/// signed-in reply lists the account's past searches, so scrub it before it becomes the
/// fixture. Prints counts only.
#[tokio::test]
#[ignore = "live: needs FEATHERPLAY_COOKIES_FILE"]
async fn live_search_suggestions() {
    let yt = connect().await;
    let raw = yt
        .search_suggestions_raw("daft pu")
        .await
        .expect("search suggestions");
    if let Some(dir) = std::env::var_os("FEATHERPLAY_RECORD_DIR") {
        let path = std::path::Path::new(&dir).join("search_suggestions.raw.json");
        std::fs::write(&path, &raw).expect("write the recording");
    }
    println!("search suggestions: {} bytes", raw.len());
}

/// Switching identities, read-only: one account switcher list, then one library read
/// (the library playlists) as each identity it lists. Prints only each identity's shape,
/// its playlist count and which reads gave the same list (groups A, B, ...), never names
/// or ids.
#[tokio::test]
#[ignore = "live: needs FEATHERPLAY_COOKIES_FILE"]
async fn live_identities() {
    let yt = connect().await;
    let list = yt.accounts().await.expect("account switcher");
    assert!(!list.is_empty(), "the switcher lists no identities");
    let mut groups: Vec<Vec<String>> = Vec::new();
    for account in &list {
        let as_it = yt.as_identity(&account.identity).expect("client");
        let ids: Vec<String> = as_it
            .library_playlists()
            .await
            .expect("library playlists")
            .into_iter()
            .map(|p| p.id.0)
            .collect();
        let group = match groups.iter().position(|g| *g == ids) {
            Some(i) => i,
            None => {
                groups.push(ids.clone());
                groups.len() - 1
            }
        };
        println!(
            "{:?}: listed active {}, channel {}, {} playlists, list {}",
            account.identity,
            account.active,
            account.has_channel,
            ids.len(),
            (b'A' + group as u8) as char,
        );
    }
}

/// Switching back and forth, read-only: as each identity the switcher lists, then the
/// first one again, Home, the library playlists and Liked songs must load (Home may be
/// empty, never an error). Then, as each identity, the first playlist of every other one:
/// a playlist this identity can't see is `NotFound`, never a parse error. Prints shapes
/// and counts only.
#[tokio::test]
#[ignore = "live: needs FEATHERPLAY_COOKIES_FILE"]
async fn live_switching_reads() {
    let yt = connect().await;
    let list = yt.accounts().await.expect("account switcher");
    assert!(!list.is_empty(), "the switcher lists no identities");
    let mut firsts = Vec::new();
    for account in &list {
        let as_it = yt.as_identity(&account.identity).expect("client");
        let home = as_it.home().await.expect("home").shelves;
        let playlists = as_it.library_playlists().await.expect("library playlists");
        let liked = as_it.liked_songs().await.expect("liked songs");
        println!(
            "{:?}: home {} shelves, {} playlists, {} liked",
            account.identity,
            home.len(),
            playlists.len(),
            liked.len()
        );
        firsts.push(playlists.first().map(|p| p.id.clone()));
    }
    let back = yt.as_identity(&list[0].identity).expect("client");
    let home = back
        .home()
        .await
        .expect("home after switching back")
        .shelves;
    println!(
        "back to {:?}: home {} shelves",
        list[0].identity,
        home.len()
    );

    // Other identities' playlists, as the brand account (the reported switch).
    let Some(brand) = list.iter().find(|a| a.identity.page_id().is_some()) else {
        return;
    };
    let as_brand = yt.as_identity(&brand.identity).expect("client");
    for (account, first) in list.iter().zip(&firsts) {
        let Some(id) = first
            .as_ref()
            .filter(|_| account.identity != brand.identity)
        else {
            continue;
        };
        let answer = match as_brand.playlist(id).await {
            Ok(p) => format!("readable, {} tracks", p.tracks.len()),
            Err(featherplay_ytm::ApiError::NotFound) => "not available".into(),
            Err(e) => panic!("{:?}'s playlist as the brand: {e}", account.identity),
        };
        println!(
            "{:?}'s first playlist as the brand: {answer}",
            account.identity
        );
    }
}

/// Records Explore's pages and the history as raw JSON into `FEATHERPLAY_RECORD_DIR`, for
/// the fixtures (scrub them before committing: the history is personal). Read-only, one
/// request per page, eight pages. Prints sizes only.
#[tokio::test]
#[ignore = "live: needs FEATHERPLAY_COOKIES_FILE and FEATHERPLAY_RECORD_DIR"]
async fn live_record_discovery() {
    let dir = std::env::var_os("FEATHERPLAY_RECORD_DIR").expect("FEATHERPLAY_RECORD_DIR");
    let dir = std::path::Path::new(&dir);
    let yt = connect().await;
    let pages: [(&str, &str, Option<&str>); 7] = [
        ("explore", "FEmusic_explore", None),
        ("new_releases", "FEmusic_new_releases", None),
        ("new_releases_albums", "FEmusic_new_releases_albums", None),
        ("charts", "FEmusic_charts", None),
        ("charts_global", "FEmusic_charts", Some("ZZ")),
        ("moods", "FEmusic_moods_and_genres", None),
        ("history", "FEmusic_history", None),
    ];
    let mut moods = String::new();
    for (name, id, country) in pages {
        let raw = yt.browse_raw(id, None, country).await.expect(name);
        std::fs::write(dir.join(format!("{name}.raw.json")), &raw).expect("write");
        println!("{name}: {} bytes", raw.len());
        if name == "moods" {
            moods = raw;
        }
    }
    // The first mood's playlists.
    let v: serde_json::Value = serde_json::from_str(&moods).expect("moods json");
    let params = first_mood_params(&v).expect("a mood button");
    let raw = yt
        .browse_raw("FEmusic_moods_and_genres_category", Some(&params), None)
        .await
        .expect("mood playlists");
    std::fs::write(dir.join("mood_playlists.raw.json"), &raw).expect("write");
    println!("mood_playlists: {} bytes", raw.len());
}

/// A song with timed lyrics (Musixmatch's), the one `tests/fixtures/timed_lyrics.json` was
/// recorded from. Most songs' lyrics are untimed, as "Around the World" (`UNTIMED_SONG`).
const TIMED_SONG: &str = "6Zt4P1scOwE";
const UNTIMED_SONG: &str = "K0HSD_i2DvA";

/// Records `TIMED_SONG`'s Lyrics tab page as the mobile app gets it, raw, into
/// `FEATHERPLAY_RECORD_DIR`: two reads. Prints its size only. Scrub it before committing
/// (`timed_lyrics.json` keeps the timings, with placeholder words).
#[tokio::test]
#[ignore = "live: needs FEATHERPLAY_COOKIES_FILE and FEATHERPLAY_RECORD_DIR"]
async fn live_record_timed_lyrics() {
    let dir = std::env::var_os("FEATHERPLAY_RECORD_DIR").expect("FEATHERPLAY_RECORD_DIR");
    let yt = connect().await;
    let raw = yt
        .timed_lyrics_raw(&TrackId(TIMED_SONG.into()))
        .await
        .expect("mobile lyrics page")
        .expect("a lyrics tab");
    std::fs::write(
        std::path::Path::new(&dir).join("timed_lyrics.raw.json"),
        &raw,
    )
    .expect("write");
    println!("timed lyrics page: {} bytes", raw.len());
}

/// The search the v0.1.0 release failed on ("exciting" in Songs: one row it couldn't read
/// failed the whole page), or `FEATHERPLAY_SEARCH`, raw, into `FEATHERPLAY_RECORD_DIR`.
/// `FEATHERPLAY_SEARCH_FILTER` picks `albums`, `artists` or `playlists` instead of songs. With
/// `FEATHERPLAY_IDENTITY` (a `key()`), as that identity, only if the cookie's account switcher
/// lists it. One read, two with an identity. Prints sizes only. Scrub it before committing.
#[tokio::test]
#[ignore = "live: needs FEATHERPLAY_COOKIES_FILE and FEATHERPLAY_RECORD_DIR"]
async fn live_record_search() {
    let dir = std::env::var_os("FEATHERPLAY_RECORD_DIR").expect("FEATHERPLAY_RECORD_DIR");
    let query = std::env::var("FEATHERPLAY_SEARCH").unwrap_or_else(|_| "exciting".into());
    let filter = std::env::var("FEATHERPLAY_SEARCH_FILTER").unwrap_or_else(|_| "songs".into());
    let f = match filter.as_str() {
        "songs" => SearchFilter::Songs,
        "albums" => SearchFilter::Albums,
        "artists" => SearchFilter::Artists,
        "playlists" => SearchFilter::Playlists,
        other => panic!("unknown filter {other}"),
    };
    let mut yt = connect().await;
    if let Ok(key) = std::env::var("FEATHERPLAY_IDENTITY") {
        let identity = featherplay_ytm::Identity::from_key(&key).expect("identity key");
        let listed = yt
            .accounts()
            .await
            .expect("account switcher")
            .iter()
            .any(|a| a.identity == identity);
        println!("identity listed by this cookie: {listed}");
        assert!(listed, "not one of this cookie's identities");
        yt = yt.as_identity(&identity).expect("client");
    }
    let raw = yt.search_raw(&query, f).await.expect("search");
    let name = query.replace(|c: char| !c.is_ascii_alphanumeric(), "_");
    std::fs::write(
        std::path::Path::new(&dir).join(format!("search_{filter}_{name}.raw.json")),
        &raw,
    )
    .expect("write");
    println!("search: {} bytes", raw.len());
}

/// A song's own lyrics, timed and untimed: two reads for the timed one, three for the other.
#[tokio::test]
#[ignore = "live: needs FEATHERPLAY_COOKIES_FILE"]
async fn live_lyrics() {
    let yt = connect().await;
    let timed = yt
        .lyrics(&TrackId(TIMED_SONG.into()))
        .await
        .expect("lyrics");
    let Some(SongLyrics::Timed(timed)) = timed else {
        panic!("no timed lyrics: {timed:?}");
    };
    println!("timed: {} lines, {:?}", timed.lines.len(), timed.source);
    let plain = yt
        .lyrics(&TrackId(UNTIMED_SONG.into()))
        .await
        .expect("lyrics");
    let Some(SongLyrics::Plain(plain)) = plain else {
        panic!("not plain lyrics: {plain:?}");
    };
    println!(
        "plain: {} lines, {:?}",
        plain.text.lines().count(),
        plain.source
    );
}

/// Explore and the history parse live: six reads. Counts only (the history is personal).
#[tokio::test]
#[ignore = "live: needs FEATHERPLAY_COOKIES_FILE"]
async fn live_explore_and_history() {
    let yt = connect().await;
    let releases = yt.new_releases().await.expect("new releases");
    assert!(!releases.is_empty(), "no new releases");
    let charts = yt.charts(None).await.expect("charts");
    assert!(charts.country.is_some() && !charts.artists.is_empty());
    let global = yt.charts(Some("ZZ")).await.expect("global charts");
    assert_eq!(global.country.as_deref(), Some("ZZ"));
    let moods = yt.moods().await.expect("moods");
    let first = &moods.first().expect("a mood section").moods[0];
    let page = yt
        .mood_playlists(&first.params)
        .await
        .expect("mood playlists");
    assert!(!page.shelves.is_empty());
    let history = yt.history().await.expect("history");
    println!(
        "explore: {} release shelves, {} chart artists, {} countries, {} mood sections, \
         {} mood shelves; history: {} days",
        releases.len(),
        charts.artists.len(),
        charts.countries.len(),
        moods.len(),
        page.shelves.len(),
        history.len()
    );
}

/// What pasted links look up: the user's example song link (the one-track watch lookup), one
/// album playlist and one artist, all public. Three reads after the sign-in, nothing else.
#[tokio::test]
#[ignore = "live: needs FEATHERPLAY_COOKIES_FILE"]
async fn live_link_lookups() {
    use featherplay_core::model::{ArtistId, PlaylistId};
    let yt = connect().await;

    let id = TrackId("UjiL0RxrDAI".into());
    let track = yt.track(&id).await.expect("track").expect("the song plays");
    assert_eq!(track.id, id);
    assert!(!track.title.is_empty() && track.duration.is_some());
    println!(
        "track: {:?}, {} artists, {:?}",
        track.video_type,
        track.artists.len(),
        track.duration
    );

    let list = PlaylistId("OLAK5uy_kfP7kUsS6hmTXdnO5JlTVRSn-LLt1bqtM".into());
    let playlist = yt.playlist(&list).await.expect("album playlist");
    assert!(!playlist.tracks.is_empty());
    println!("album playlist: {} tracks", playlist.tracks.len());

    let artist = yt
        .artist(&ArtistId("UCRr1xG_2WIDs18a6cIiCxeA".into()))
        .await
        .expect("artist");
    assert!(!artist.summary.name.is_empty());
    println!("artist: {}", artist.summary.name);
}

fn first_mood_params(v: &serde_json::Value) -> Option<String> {
    match v {
        serde_json::Value::Object(o) => o
            .get("musicNavigationButtonRenderer")
            .and_then(|b| b.pointer("/clickCommand/browseEndpoint/params"))
            .and_then(|p| p.as_str())
            .map(str::to_string)
            .or_else(|| o.values().find_map(first_mood_params)),
        serde_json::Value::Array(a) => a.iter().find_map(first_mood_params),
        _ => None,
    }
}
