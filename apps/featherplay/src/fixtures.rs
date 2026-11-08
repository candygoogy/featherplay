//! `--fixtures`: data from the recorded, scrubbed API responses in
//! `crates/ytm/tests/fixtures`, parsed by the same `featherplay_ytm::convert` code as live
//! data, and covers from `$FEATHERPLAY_HOME/design-covers` (downloaded once for the design
//! mockups, named by the SHA-1 of their URL). No network, no sign-in.
//!
//! There's one recording per kind, so every album id opens the same album, and so on.
//! The sidebar's playlists are the searched ones (there's no library recording).
//!
//! Only a few fixture thumbnails have a design cover (none on Home). For measuring with art
//! everywhere, `FEATHERPLAY_FIXTURE_COVERS` names a folder of images: every URL without a
//! design cover then gets one of them, picked by the URL's hash (`tests/smoothness.rs`).

use std::collections::HashMap;
use std::fmt;
use std::path::{Path, PathBuf};

use featherplay_core::model::{AlbumId, PlaylistId, Track, TrackId};
use featherplay_ytm::{
    AccountIdentity, AccountInfo, Album, AlbumSummary, ApiError, Artist, ArtistSummary, Charts,
    HistoryDay, HomeFeed, HomeShelf, Identity, MoodPage, MoodSection, Playlist, PlaylistSummary,
    SearchFilter, SearchResults, SearchSuggestions, convert,
};
use serde_json::Value;
use sha1::{Digest, Sha1};

mod demo;

use demo::Demo;

/// The id the recorded album was fetched with.
const ALBUM_ID: &str = "MPREb_dMHztPsrDgN";

/// Which data `--fixtures` shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FixtureSet {
    /// The recorded, scrubbed API responses.
    #[default]
    Recorded,
    /// The free-licence demo songs (`apps/featherplay/fixtures/demo`).
    Demo,
}

impl FixtureSet {
    /// `FEATHERPLAY_FIXTURES`: `demo`, or unset (or `recorded`) for the recordings.
    pub fn from_env() -> Result<FixtureSet, String> {
        Self::parse(std::env::var("FEATHERPLAY_FIXTURES").ok().as_deref())
    }

    pub fn parse(value: Option<&str>) -> Result<FixtureSet, String> {
        match value {
            None | Some("" | "recorded") => Ok(FixtureSet::Recorded),
            Some("demo") => Ok(FixtureSet::Demo),
            Some(other) => Err(format!(
                "FEATHERPLAY_FIXTURES={other:?}: use demo, or leave it unset"
            )),
        }
    }
}

pub struct Fixtures {
    dir: PathBuf,
    /// The demo set, which answers instead of the recordings (but for Explore's charts and
    /// moods).
    demo: Option<Demo>,
    /// Thumbnail URL to a file in the design-covers folder.
    covers: HashMap<String, PathBuf>,
    /// `FEATHERPLAY_FIXTURE_COVERS`: images for every other URL.
    fallback: Vec<PathBuf>,
}

impl fmt::Debug for Fixtures {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Fixtures({}, {} covers, {} fallback, demo: {})",
            self.dir.display(),
            self.covers.len(),
            self.fallback.len(),
            self.demo.is_some()
        )
    }
}

impl Fixtures {
    /// The set `FEATHERPLAY_FIXTURES` names (`FixtureSet::from_env`).
    pub fn load() -> Result<Fixtures, String> {
        Self::load_set(FixtureSet::from_env()?)
    }

    /// `FEATHERPLAY_FIXTURES_DIR` overrides the recordings' folder (the default is the source
    /// tree's).
    pub fn load_set(set: FixtureSet) -> Result<Fixtures, String> {
        let dir = std::env::var_os("FEATHERPLAY_FIXTURES_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                Path::new(env!("CARGO_MANIFEST_DIR")).join("../../crates/ytm/tests/fixtures")
            });
        if !dir.join("album.json").is_file() {
            return Err(format!("no fixtures in {}", dir.display()));
        }
        let covers_dir =
            std::env::var_os("FEATHERPLAY_HOME").map(|h| PathBuf::from(h).join("design-covers"));
        let covers = match covers_dir {
            Some(d) if d.is_dir() => index_covers(&dir, &d),
            _ => {
                log!("fixtures: no $FEATHERPLAY_HOME/design-covers, covers stay empty");
                HashMap::new()
            }
        };
        let fallback = std::env::var_os("FEATHERPLAY_FIXTURE_COVERS")
            .map(|d| list_files(Path::new(&d)))
            .unwrap_or_default();
        let demo = match set {
            FixtureSet::Recorded => None,
            FixtureSet::Demo => Some(Demo::load(
                &Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/demo"),
            )?),
        };
        Ok(Fixtures {
            dir,
            demo,
            covers,
            fallback,
        })
    }

    /// The cached design cover for a thumbnail URL from the fixtures, or else one of the
    /// `FEATHERPLAY_FIXTURE_COVERS` images; in the demo set, its own cover.
    pub fn cover_file(&self, url: &str) -> Option<PathBuf> {
        if let Some(demo) = &self.demo {
            return demo.cover_file(url);
        }
        if let Some(file) = self.covers.get(url) {
            return Some(file.clone());
        }
        if self.fallback.is_empty() {
            return None;
        }
        let digest = Sha1::digest(url.as_bytes());
        let n = u64::from_le_bytes(digest[..8].try_into().unwrap_or_default());
        let i = (n % self.fallback.len() as u64) as usize;
        self.fallback.get(i).cloned()
    }

    fn raw(&self, name: &str) -> Result<String, ApiError> {
        std::fs::read_to_string(self.dir.join(format!("{name}.json")))
            .map_err(|e| ApiError::Parse(format!("fixture {name}: {e}")))
    }

    /// The recording is an array of pages: the first one and its continuation, as the feed
    /// starts, then the page after them (a community shelf), for `home_more`.
    pub fn home(&self) -> Result<HomeFeed, ApiError> {
        if let Some(d) = &self.demo {
            return Ok(d.home());
        }
        let pages = self.home_pages()?;
        convert::home_feed(pages.get(..2).unwrap_or(&pages))
    }

    /// Any token gets the recording's third page, which ends the feed. The demo feed has
    /// no more pages.
    pub fn home_more(&self, _token: &str) -> Result<HomeFeed, ApiError> {
        if self.demo.is_some() {
            return Ok(HomeFeed::default());
        }
        let pages = self.home_pages()?;
        let page = pages
            .get(2)
            .ok_or_else(|| ApiError::Parse("fixture home: no third page".into()))?;
        convert::home_more(page)
    }

    fn home_pages(&self) -> Result<Vec<String>, ApiError> {
        let pages: Vec<Value> = serde_json::from_str(&self.raw("home")?)
            .map_err(|e| ApiError::Parse(format!("fixture home: {e}")))?;
        Ok(pages.iter().map(Value::to_string).collect())
    }

    /// The recorded playlist, standing for one of the account's own, then the searched ones,
    /// saved from others.
    pub fn library_playlists(&self) -> Result<Vec<PlaylistSummary>, ApiError> {
        if let Some(d) = &self.demo {
            return Ok(d.library_playlists());
        }
        let mut own = self.playlist(&PlaylistId("fixture".into()))?.summary;
        own.owned = true;
        let mut out = vec![own];
        for p in convert::search_playlists(&self.raw("search_playlists")?)? {
            if !out.iter().any(|o| o.title == p.title) {
                out.push(p);
            }
        }
        Ok(out)
    }

    pub fn liked_songs(&self) -> Result<Vec<Track>, ApiError> {
        if let Some(d) = &self.demo {
            return Ok(d.liked_songs());
        }
        #[allow(unused_mut, reason = "lengthened by the performance harness only")]
        let mut tracks = convert::playlist_tracks(&[self.raw("liked_songs")?])?;
        #[cfg(feature = "perf-harness")]
        crate::perf::lengthen(&mut tracks);
        Ok(tracks)
    }

    /// The recorded album, then the artist's albums and singles.
    pub fn library_albums(&self) -> Result<Vec<AlbumSummary>, ApiError> {
        if let Some(d) = &self.demo {
            return Ok(d.library_albums());
        }
        let mut out = vec![self.album(&AlbumId(ALBUM_ID.into()))?.summary];
        let artist = self.artist()?;
        out.extend(artist.albums);
        out.extend(artist.singles);
        out.dedup_by(|a, b| a.id == b.id);
        Ok(out)
    }

    pub fn library_artists(&self) -> Result<Vec<ArtistSummary>, ApiError> {
        if let Some(d) = &self.demo {
            return Ok(d.library_artists());
        }
        Ok(vec![self.artist()?.summary])
    }

    /// "LM" opens the liked songs recording, anything else the playlist recording.
    pub fn playlist(&self, id: &PlaylistId) -> Result<Playlist, ApiError> {
        if let Some(d) = &self.demo {
            return Ok(d.playlist(id));
        }
        let name = if id.0 == "LM" || id.0 == "VLLM" {
            "liked_songs"
        } else {
            "playlist"
        };
        let raw = self.raw(name)?;
        #[allow(unused_mut, reason = "lengthened by the performance harness only")]
        let mut tracks = convert::playlist_tracks(std::slice::from_ref(&raw))?;
        #[cfg(feature = "perf-harness")]
        crate::perf::lengthen(&mut tracks);
        Ok(Playlist {
            summary: convert::playlist_summary(&raw, id)?,
            tracks,
            pages: 1,
        })
    }

    pub fn album(&self, id: &AlbumId) -> Result<Album, ApiError> {
        if let Some(d) = &self.demo {
            return Ok(d.album(id));
        }
        let _ = id;
        convert::album(&self.raw("album")?, &AlbumId(ALBUM_ID.into()))
    }

    pub fn artist(&self) -> Result<Artist, ApiError> {
        if let Some(d) = &self.demo {
            return Ok(d.artist());
        }
        convert::artist(&self.raw("artist")?)
    }

    /// Songs and playlists are recorded; albums and artists come from the album and
    /// artist recordings.
    pub fn search(&self, filter: SearchFilter) -> Result<SearchResults, ApiError> {
        if let Some(d) = &self.demo {
            return Ok(d.search(filter));
        }
        let mut out = SearchResults::default();
        match filter {
            SearchFilter::Songs => out.songs = convert::search_songs(&self.raw("search_songs")?)?,
            SearchFilter::Playlists => {
                out.playlists = convert::search_playlists(&self.raw("search_playlists")?)?
            }
            SearchFilter::Albums => out.albums = self.library_albums()?,
            SearchFilter::Artists => out.artists = self.library_artists()?,
        }
        Ok(out)
    }

    /// The recorded suggestions for "daft pu"; the demo set has none.
    pub fn search_suggestions(&self) -> Result<SearchSuggestions, ApiError> {
        if self.demo.is_some() {
            return Ok(SearchSuggestions::default());
        }
        convert::search_suggestions(&self.raw("search_suggestions")?)
    }

    /// The scrubbed account recording (a placeholder name). Its photo URL is a
    /// placeholder too, so the avatar is the artist's picture, or the album cover when
    /// that one wasn't downloaded.
    pub fn account_info(&self) -> Result<AccountInfo, ApiError> {
        if let Some(d) = &self.demo {
            return Ok(d.account_info());
        }
        let mut info = convert::account_info(&self.raw("account_menu")?)?;
        let artist = self.artist()?.summary.thumbnail;
        let album = self.album(&AlbumId(ALBUM_ID.into()))?.summary.thumbnail;
        info.photo = [artist, album]
            .into_iter()
            .flatten()
            .find(|url| self.cover_file(url).is_some());
        Ok(info)
    }

    /// The scrubbed account switcher (placeholder identities). Its photo URLs are
    /// placeholders, so the photos are the fixtures' covers that were downloaded, or none.
    pub fn accounts(&self) -> Result<Vec<AccountIdentity>, ApiError> {
        if let Some(d) = &self.demo {
            return Ok(d.accounts());
        }
        let mut list = convert::accounts(&self.raw("account_switcher")?, &Identity::default())?;
        let mut covers = [
            self.artist()?.summary.thumbnail,
            self.album(&AlbumId(ALBUM_ID.into()))?.summary.thumbnail,
        ]
        .into_iter()
        .flatten()
        .filter(|url| self.cover_file(url).is_some());
        for a in &mut list {
            a.photo = covers.next();
        }
        Ok(list)
    }

    pub fn new_releases(&self) -> Result<Vec<HomeShelf>, ApiError> {
        if let Some(d) = &self.demo {
            return Ok(d.new_releases());
        }
        convert::new_releases(&self.raw("explore_new_releases")?)
    }

    pub fn charts(&self) -> Result<Charts, ApiError> {
        convert::charts(&self.raw("explore_charts")?)
    }

    pub fn moods(&self) -> Result<Vec<MoodSection>, ApiError> {
        convert::moods(&self.raw("explore_moods")?)
    }

    pub fn mood_playlists(&self) -> Result<MoodPage, ApiError> {
        convert::mood_playlists(&self.raw("explore_mood_playlists")?)
    }

    pub fn history(&self) -> Result<Vec<HistoryDay>, ApiError> {
        if let Some(d) = &self.demo {
            return Ok(d.history());
        }
        convert::history(&self.raw("history")?)
    }

    pub fn radio(&self) -> Result<Vec<Track>, ApiError> {
        if let Some(d) = &self.demo {
            return Ok(d.radio());
        }
        convert::radio(&self.raw("radio")?)
    }

    /// A track of the radio recordings (seeded on the song, and on its music video).
    pub fn track(&self, id: &TrackId) -> Result<Option<Track>, ApiError> {
        if let Some(d) = &self.demo {
            return Ok(d.track(id));
        }
        for name in ["radio", "radio_mv"] {
            if let Some(track) = convert::seed_track(&self.raw(name)?, id)? {
                return Ok(Some(track));
            }
        }
        Ok(None)
    }

    /// The demo set answers (`FixtureSet::Demo`).
    pub fn is_demo(&self) -> bool {
        self.demo.is_some()
    }

    /// The demo set's timed lyrics for a song and the line crediting them; `None` for a song
    /// without lyrics, and outside the demo set.
    pub fn demo_lyrics(
        &self,
        id: &TrackId,
    ) -> Result<Option<(Vec<featherplay_lyrics::Line>, String)>, ApiError> {
        match &self.demo {
            Some(d) => d.lyrics(id),
            None => Ok(None),
        }
    }
}

/// Maps every thumbnail URL in the fixtures to a design cover, when one of the URLs of
/// the same thumbnail set was downloaded (one size per set was saved).
fn index_covers(fixtures: &Path, covers: &Path) -> HashMap<String, PathBuf> {
    let mut sets = Vec::new();
    let Ok(entries) = std::fs::read_dir(fixtures) else {
        return HashMap::new();
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "json")
            && let Ok(text) = std::fs::read_to_string(&path)
            && let Ok(json) = serde_json::from_str::<Value>(&text)
        {
            thumbnail_sets(&json, &mut sets);
        }
    }
    let mut map = HashMap::new();
    for urls in sets {
        let file = urls.iter().find_map(|u| {
            let f = covers.join(cover_name(u));
            f.is_file().then_some(f)
        });
        if let Some(file) = file {
            for u in urls {
                map.insert(u, file.clone());
            }
        }
    }
    map
}

/// The files in `dir`, sorted, so a URL always gets the same one.
fn list_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        log!("fixtures: no {}, fallback covers stay empty", dir.display());
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    files
}

/// A design cover's file name: the first 12 hex digits of the URL's SHA-1, `.jpg`.
fn cover_name(url: &str) -> String {
    let digest = Sha1::digest(url.as_bytes());
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    format!("{}.jpg", &hex[..12])
}

fn thumbnail_sets(v: &Value, out: &mut Vec<Vec<String>>) {
    match v {
        Value::Object(map) => {
            for (k, child) in map {
                if k == "thumbnails"
                    && let Value::Array(items) = child
                {
                    let urls: Vec<String> = items
                        .iter()
                        .filter_map(|t| t.get("url")?.as_str().map(str::to_string))
                        .collect();
                    if !urls.is_empty() {
                        out.push(urls);
                    }
                }
                thumbnail_sets(child, out);
            }
        }
        Value::Array(items) => {
            for child in items {
                thumbnail_sets(child, out);
            }
        }
        _ => {}
    }
}
