//! The demo fixture set (`FEATHERPLAY_FIXTURES=demo`): six free-licence songs (Josh Woodward,
//! DoKashiteru feat. Snowflake, Scott Buckley; CC BY), the same ones the website plays, with
//! our own cover art and the three Josh Woodward songs' timed lyrics, from
//! `crates/controller/fixtures/demo`. Nothing in it was recorded from an account: the account
//! is a plain "Listener" with no photo, and the playlists and shelves are made up from the six
//! songs. What gets published (the README's screenshots) is rendered from this set, never from
//! the recordings, whose covers are commercial releases.
//!
//! `demo.json` names the songs by their slug, which is also their cover's and lyrics' file name.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use featherplay_core::model::{
    AlbumId, AlbumRef, ArtistId, ArtistRef, LikeStatus, PlaylistId, Track, TrackId, VideoType,
};
use featherplay_lyrics::lrc::Line;
use featherplay_ytm::{
    AccountIdentity, AccountInfo, Album, AlbumSummary, ApiError, Artist, ArtistSummary, HistoryDay,
    HomeFeed, HomeItem, HomeShelf, Identity, Playlist, PlaylistSummary, SearchFilter,
    SearchResults,
};
use serde::Deserialize;

/// The scheme of the demo's cover URLs: `demo://<slug>`.
const COVER_SCHEME: &str = "demo://";
const TRACK_PREFIX: &str = "demo-";
const SINGLE_PREFIX: &str = "demo-single-";
const ARTIST_PREFIX: &str = "demo-artist-";

#[derive(Debug, Deserialize)]
struct Data {
    account: String,
    tracks: Vec<DemoTrack>,
    playlists: Vec<DemoPlaylist>,
    home: Vec<DemoShelf>,
    liked: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct DemoTrack {
    slug: String,
    title: String,
    artist: String,
    duration_ms: u64,
    /// Set for the songs whose lyrics ship (`lyrics/<slug>.lyrics.json`): the line under them.
    #[serde(default)]
    lyrics_credit: Option<String>,
}

#[derive(Debug, Deserialize)]
struct DemoPlaylist {
    id: String,
    title: String,
    #[serde(default)]
    author: Option<String>,
    #[serde(default)]
    owned: bool,
    cover: String,
    tracks: Vec<String>,
}

/// A Home shelf: one kind of item each, by slug (songs, singles) or id (playlists).
#[derive(Debug, Deserialize)]
struct DemoShelf {
    title: String,
    #[serde(default)]
    tracks: Vec<String>,
    #[serde(default)]
    singles: Vec<String>,
    #[serde(default)]
    playlists: Vec<String>,
}

/// One timed line as the website's lyrics files have it.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DemoLine {
    at_ms: u64,
    line: String,
}

#[derive(Debug)]
pub struct Demo {
    dir: PathBuf,
    data: Data,
}

fn parse_error(what: &str, e: impl std::fmt::Display) -> ApiError {
    ApiError::Parse(format!("demo fixture {what}: {e}"))
}

impl Demo {
    /// `dir` holds `demo.json`, `covers/` and `lyrics/`.
    pub fn load(dir: &Path) -> Result<Demo, String> {
        let path = dir.join("demo.json");
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("demo fixtures {}: {e}", path.display()))?;
        let data: Data = serde_json::from_str(&text)
            .map_err(|e| format!("demo fixtures {}: {e}", path.display()))?;
        let demo = Demo {
            dir: dir.to_path_buf(),
            data,
        };
        demo.check()?;
        Ok(demo)
    }

    /// Every slug and playlist id the file names exists, so the methods below can't miss.
    fn check(&self) -> Result<(), String> {
        let slugs: HashSet<&str> = self.data.tracks.iter().map(|t| t.slug.as_str()).collect();
        let ids: HashSet<&str> = self.data.playlists.iter().map(|p| p.id.as_str()).collect();
        let named_slugs = self
            .data
            .playlists
            .iter()
            .flat_map(|p| p.tracks.iter().chain(std::iter::once(&p.cover)))
            .chain(
                self.data
                    .home
                    .iter()
                    .flat_map(|s| s.tracks.iter().chain(&s.singles)),
            )
            .chain(&self.data.liked);
        for slug in named_slugs {
            if !slugs.contains(slug.as_str()) {
                return Err(format!("demo fixtures: no track {slug:?}"));
            }
        }
        for id in self.data.home.iter().flat_map(|s| &s.playlists) {
            if !ids.contains(id.as_str()) {
                return Err(format!("demo fixtures: no playlist {id:?}"));
            }
        }
        if self.data.tracks.is_empty() {
            return Err("demo fixtures: no tracks".into());
        }
        Ok(())
    }

    /// The cover file of a `demo://<slug>` URL.
    pub fn cover_file(&self, url: &str) -> Option<PathBuf> {
        let slug = url.strip_prefix(COVER_SCHEME)?;
        let file = self.dir.join("covers").join(format!("{slug}.webp"));
        file.is_file().then_some(file)
    }

    fn find(&self, slug: &str) -> Option<&DemoTrack> {
        self.data.tracks.iter().find(|t| t.slug == slug)
    }

    fn tracks(&self, slugs: &[String]) -> Vec<Track> {
        slugs
            .iter()
            .filter_map(|s| self.find(s))
            .map(|t| self.track_of(t))
            .collect()
    }

    fn artist_ref(name: &str) -> ArtistRef {
        ArtistRef {
            id: Some(ArtistId(format!("{ARTIST_PREFIX}{}", slugify(name)))),
            name: name.to_owned(),
        }
    }

    fn track_of(&self, t: &DemoTrack) -> Track {
        let liked = self.data.liked.contains(&t.slug);
        Track {
            id: TrackId(format!("{TRACK_PREFIX}{}", t.slug)),
            title: t.title.clone(),
            artists: vec![Self::artist_ref(&t.artist)],
            album: Some(AlbumRef {
                id: AlbumId(format!("{SINGLE_PREFIX}{}", t.slug)),
                title: t.title.clone(),
            }),
            duration: Some(Duration::from_millis(t.duration_ms)),
            video_type: VideoType::Song,
            thumbnail: Some(cover_url(&t.slug)),
            like: if liked {
                LikeStatus::Like
            } else {
                LikeStatus::Indifferent
            },
            counterpart: None,
        }
    }

    /// Each song is its own single.
    fn single_of(t: &DemoTrack) -> AlbumSummary {
        AlbumSummary {
            id: AlbumId(format!("{SINGLE_PREFIX}{}", t.slug)),
            title: t.title.clone(),
            artists: vec![Self::artist_ref(&t.artist)],
            year: None,
            thumbnail: Some(cover_url(&t.slug)),
        }
    }

    fn singles(&self, slugs: &[String]) -> Vec<AlbumSummary> {
        slugs
            .iter()
            .filter_map(|s| self.find(s))
            .map(Self::single_of)
            .collect()
    }

    fn summary_of(&self, p: &DemoPlaylist) -> PlaylistSummary {
        PlaylistSummary {
            id: PlaylistId(p.id.clone()),
            title: p.title.clone(),
            author: p
                .author
                .clone()
                .or_else(|| p.owned.then(|| self.data.account.clone())),
            count: u32::try_from(p.tracks.len()).ok(),
            views: None,
            avatar: None,
            thumbnail: Some(cover_url(&p.cover)),
            owned: p.owned,
        }
    }

    fn find_playlist(&self, id: &str) -> Option<&DemoPlaylist> {
        self.data.playlists.iter().find(|p| p.id == id)
    }

    pub fn home(&self) -> HomeFeed {
        let shelves = self
            .data
            .home
            .iter()
            .map(|s| {
                let mut items: Vec<HomeItem> = Vec::new();
                items.extend(self.tracks(&s.tracks).into_iter().map(HomeItem::Track));
                items.extend(self.singles(&s.singles).into_iter().map(HomeItem::Album));
                items.extend(
                    s.playlists
                        .iter()
                        .filter_map(|id| self.find_playlist(id))
                        .map(|p| HomeItem::Playlist(self.summary_of(p))),
                );
                HomeShelf {
                    title: s.title.clone(),
                    strapline: None,
                    items,
                }
            })
            .collect();
        HomeFeed {
            shelves,
            more: None,
        }
    }

    /// The account's own playlists, then the saved ones; not the mixes made for it.
    pub fn library_playlists(&self) -> Vec<PlaylistSummary> {
        let (own, saved): (Vec<_>, Vec<_>) = self
            .data
            .playlists
            .iter()
            .filter(|p| !is_mix(&p.id))
            .partition(|p| p.owned);
        own.into_iter()
            .chain(saved)
            .map(|p| self.summary_of(p))
            .collect()
    }

    pub fn liked_songs(&self) -> Vec<Track> {
        self.tracks(&self.data.liked)
    }

    pub fn library_albums(&self) -> Vec<AlbumSummary> {
        self.data.tracks.iter().map(Self::single_of).collect()
    }

    /// The artists, in the order their first song comes.
    pub fn library_artists(&self) -> Vec<ArtistSummary> {
        let mut seen = HashSet::new();
        self.data
            .tracks
            .iter()
            .filter(|t| seen.insert(t.artist.as_str()))
            .map(|t| ArtistSummary {
                id: ArtistId(format!("{ARTIST_PREFIX}{}", slugify(&t.artist))),
                name: t.artist.clone(),
                thumbnail: None,
            })
            .collect()
    }

    /// "LM" is Liked songs; an unknown id opens the first playlist.
    pub fn playlist(&self, id: &PlaylistId) -> Playlist {
        if id.0 == "LM" || id.0 == "VLLM" {
            let tracks = self.liked_songs();
            return Playlist {
                summary: PlaylistSummary {
                    id: id.clone(),
                    title: "Liked songs".into(),
                    author: Some(self.data.account.clone()),
                    count: u32::try_from(tracks.len()).ok(),
                    views: None,
                    avatar: None,
                    thumbnail: None,
                    owned: false,
                },
                tracks,
                pages: 1,
            };
        }
        let bare = id.0.strip_prefix("VL").unwrap_or(&id.0);
        let p = self.find_playlist(bare).unwrap_or(&self.data.playlists[0]);
        Playlist {
            summary: self.summary_of(p),
            tracks: self.tracks(&p.tracks),
            pages: 1,
        }
    }

    /// A song's single; an unknown id opens the first song's.
    pub fn album(&self, id: &AlbumId) -> Album {
        let t =
            id.0.strip_prefix(SINGLE_PREFIX)
                .and_then(|slug| self.find(slug))
                .unwrap_or(&self.data.tracks[0]);
        Album {
            summary: Self::single_of(t),
            tracks: vec![self.track_of(t)],
        }
    }

    /// The first song's artist, with their songs as top songs and singles.
    pub fn artist(&self) -> Artist {
        let name = &self.data.tracks[0].artist;
        let theirs: Vec<&DemoTrack> = self
            .data
            .tracks
            .iter()
            .filter(|t| &t.artist == name)
            .collect();
        Artist {
            summary: ArtistSummary {
                id: ArtistId(format!("{ARTIST_PREFIX}{}", slugify(name))),
                name: name.clone(),
                thumbnail: None,
            },
            top_songs: theirs.iter().map(|t| self.track_of(t)).collect(),
            albums: Vec::new(),
            singles: theirs.iter().map(|t| Self::single_of(t)).collect(),
        }
    }

    pub fn search(&self, filter: SearchFilter) -> SearchResults {
        let mut out = SearchResults::default();
        match filter {
            SearchFilter::Songs => out.songs = self.radio(),
            SearchFilter::Albums => out.albums = self.library_albums(),
            SearchFilter::Artists => out.artists = self.library_artists(),
            SearchFilter::Playlists => out.playlists = self.library_playlists(),
        }
        out
    }

    /// Every song, in the file's order: the queue the snapshots play.
    pub fn radio(&self) -> Vec<Track> {
        self.data.tracks.iter().map(|t| self.track_of(t)).collect()
    }

    pub fn track(&self, id: &TrackId) -> Option<Track> {
        let slug = id.0.strip_prefix(TRACK_PREFIX)?;
        self.find(slug).map(|t| self.track_of(t))
    }

    pub fn history(&self) -> Vec<HistoryDay> {
        vec![HistoryDay {
            title: "Today".into(),
            tracks: self.radio(),
        }]
    }

    /// The singles, as Explore's new releases.
    pub fn new_releases(&self) -> Vec<HomeShelf> {
        vec![HomeShelf {
            title: "Singles".into(),
            strapline: None,
            items: self
                .library_albums()
                .into_iter()
                .map(HomeItem::Album)
                .collect(),
        }]
    }

    /// The plain listener: no handle, no photo.
    pub fn account_info(&self) -> AccountInfo {
        AccountInfo {
            name: self.data.account.clone(),
            handle: None,
            photo: None,
        }
    }

    pub fn accounts(&self) -> Vec<AccountIdentity> {
        vec![AccountIdentity {
            name: self.data.account.clone(),
            handle: None,
            byline: None,
            has_channel: false,
            photo: None,
            account: None,
            active: true,
            identity: Identity::default(),
        }]
    }

    /// A song's timed lyrics and the line crediting them, when they ship with the set.
    pub fn lyrics(&self, id: &TrackId) -> Result<Option<(Vec<Line>, String)>, ApiError> {
        let Some(t) = id.0.strip_prefix(TRACK_PREFIX).and_then(|s| self.find(s)) else {
            return Ok(None);
        };
        let Some(credit) = &t.lyrics_credit else {
            return Ok(None);
        };
        let path = self
            .dir
            .join("lyrics")
            .join(format!("{}.lyrics.json", t.slug));
        let text = std::fs::read_to_string(&path).map_err(|e| parse_error(&t.slug, e))?;
        let lines: Vec<DemoLine> =
            serde_json::from_str(&text).map_err(|e| parse_error(&t.slug, e))?;
        let lines = lines
            .into_iter()
            .map(|l| Line {
                start: Duration::from_millis(l.at_ms),
                text: l.line,
            })
            .collect();
        Ok(Some((lines, credit.clone())))
    }
}

fn cover_url(slug: &str) -> String {
    format!("{COVER_SCHEME}{slug}")
}

/// Mixes made for the account, as Home tells them from playlists (`pages::home::is_mix`).
fn is_mix(id: &str) -> bool {
    id.starts_with("RDTMAK")
}

/// "Josh Woodward" → "josh-woodward".
fn slugify(name: &str) -> String {
    name.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join("-")
}

#[cfg(test)]
mod tests {
    use super::super::{FixtureSet, Fixtures};
    use super::*;

    const TITLES: [&str; 6] = [
        "California Lullabye",
        "Invisible Light",
        "She Dreams in Blue",
        "Watching Seabirds Soar (Harmony)",
        "Origami",
        "In This Moment",
    ];

    fn demo() -> Fixtures {
        Fixtures::load_set(FixtureSet::Demo).expect("demo fixtures")
    }

    fn id(slug: &str) -> TrackId {
        TrackId(format!("{TRACK_PREFIX}{slug}"))
    }

    /// The cover URL of a Home feed item, as the app's Home cards show it
    /// (`ui_bridge::home_thumbnail`).
    fn home_thumbnail(item: &HomeItem) -> Option<String> {
        match item {
            HomeItem::Album(a) => a.thumbnail.clone(),
            HomeItem::Playlist(p) => p.thumbnail.clone(),
            HomeItem::Artist(a) => a.thumbnail.clone(),
            HomeItem::Track(t) => t.thumbnail.clone(),
        }
    }

    #[test]
    fn featherplay_fixtures_demo_should_pick_the_demo_set() {
        assert_eq!(FixtureSet::parse(Some("demo")), Ok(FixtureSet::Demo));
        assert_eq!(FixtureSet::parse(None), Ok(FixtureSet::Recorded));
        assert!(FixtureSet::parse(Some("other")).is_err());
    }

    #[test]
    fn the_demo_radio_should_be_the_six_free_licence_songs() {
        let titles: Vec<String> = demo()
            .radio()
            .expect("radio")
            .into_iter()
            .map(|t| t.title)
            .collect();
        assert_eq!(titles, TITLES);
    }

    #[test]
    fn every_song_on_demo_home_should_be_one_of_the_six() {
        let home = demo().home().expect("home");
        assert!(home.shelves.len() >= 3, "Home looks full");
        for shelf in &home.shelves {
            for item in &shelf.items {
                let title = match item {
                    HomeItem::Track(t) => &t.title,
                    HomeItem::Album(a) => &a.title,
                    HomeItem::Playlist(_) | HomeItem::Artist(_) => continue,
                };
                assert!(TITLES.contains(&title.as_str()), "{title:?} on Home");
            }
        }
    }

    #[test]
    fn every_demo_cover_should_be_one_of_our_own_cover_files() {
        let f = demo();
        let home = f.home().expect("home");
        let urls = home
            .shelves
            .iter()
            .flat_map(|s| &s.items)
            .filter_map(home_thumbnail)
            .chain(
                f.radio()
                    .expect("radio")
                    .into_iter()
                    .filter_map(|t| t.thumbnail),
            )
            .chain(
                f.library_playlists()
                    .expect("playlists")
                    .into_iter()
                    .filter_map(|p| p.thumbnail),
            );
        let mut n = 0;
        for url in urls {
            assert!(f.cover_file(&url).is_some(), "no cover for {url}");
            n += 1;
        }
        assert!(n > 10);
    }

    #[test]
    fn the_demo_account_should_be_a_plain_listener_without_a_photo() {
        let info = demo().account_info().expect("account");
        assert_eq!(info.name, "Listener");
        assert_eq!(info.handle, None);
        assert_eq!(info.photo, None);
    }

    #[test]
    fn the_demo_playlist_page_should_list_its_songs() {
        let f = demo();
        let first = f.library_playlists().expect("playlists").remove(0);
        assert_eq!(first.title, "Quiet hours");
        assert!(first.owned);
        let page = f.playlist(&first.id).expect("playlist");
        assert_eq!(page.tracks.len(), 6);
    }

    #[test]
    fn josh_woodward_songs_should_have_timed_demo_lyrics() {
        let (lines, credit) = demo()
            .demo_lyrics(&id("california-lullabye"))
            .expect("read")
            .expect("lyrics");
        assert_eq!(lines[0].text, "But distance lost its fight");
        assert_eq!(lines[0].start, Duration::from_millis(700));
        assert!(credit.contains("Josh Woodward"));
    }

    #[test]
    fn watching_seabirds_soar_should_have_no_demo_lyrics() {
        let found = demo()
            .demo_lyrics(&id("watching-seabirds-soar"))
            .expect("read");
        assert_eq!(found, None);
    }
}
