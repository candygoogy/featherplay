//! The music catalogue and library, over ytmapi-rs with cookie (browser) auth.
//!
//! `api` makes the requests; `convert` turns raw responses into
//! `featherplay_core::model` types and is tested on recorded fixtures.

use std::fmt;
use std::time::Duration;

use featherplay_core::model::{AlbumId, ArtistId, ArtistRef, PlaylistId, Track};
use serde::{Deserialize, Serialize};

pub mod api;
pub mod convert;

pub use api::YtMusic;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SearchFilter {
    Songs,
    Albums,
    Artists,
    Playlists,
}

/// Only the list matching the requested `SearchFilter` is filled.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SearchResults {
    pub songs: Vec<Track>,
    pub albums: Vec<AlbumSummary>,
    pub artists: Vec<ArtistSummary>,
    pub playlists: Vec<PlaylistSummary>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct AlbumSummary {
    pub id: AlbumId,
    pub title: String,
    pub artists: Vec<ArtistRef>,
    pub year: Option<String>,
    pub thumbnail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ArtistSummary {
    pub id: ArtistId,
    pub name: String,
    pub thumbnail: Option<String>,
}

/// `id` is the bare playlist id, without the `VL` browse prefix.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PlaylistSummary {
    pub id: PlaylistId,
    pub title: String,
    pub author: Option<String>,
    pub count: Option<u32>,
    /// "529K views", as a Home card's subtitle has it (a community playlist).
    #[serde(default)]
    pub views: Option<String>,
    /// The creator's round photo, which a community playlist on Home wears over its collage.
    #[serde(default)]
    pub avatar: Option<String>,
    pub thumbnail: Option<String>,
    /// The account's own playlist (it can rename and delete it), as the library lists it.
    /// `false` for a saved one of someone else's, and wherever else a playlist appears.
    #[serde(default)]
    pub owned: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Album {
    pub summary: AlbumSummary,
    pub tracks: Vec<Track>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Playlist {
    pub summary: PlaylistSummary,
    pub tracks: Vec<Track>,
    /// How many responses it was read from: the first page and its continuations.
    #[serde(default)]
    pub pages: usize,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Artist {
    pub summary: ArtistSummary,
    pub top_songs: Vec<Track>,
    pub albums: Vec<AlbumSummary>,
    pub singles: Vec<AlbumSummary>,
}

/// One shelf of the Home feed, such as "Quick picks" or "Listen again".
/// `strapline` is the small line above the title, when there is one.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HomeShelf {
    pub title: String,
    pub strapline: Option<String>,
    pub items: Vec<HomeItem>,
}

/// Home feed shelves, and the token of the page after them (`YtMusic::home_more`), if the
/// feed goes on.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HomeFeed {
    pub shelves: Vec<HomeShelf>,
    pub more: Option<String>,
}

/// Explore's charts for one country: the video charts (playlists) and the top artists.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Charts {
    /// The code of the country shown (`"IT"`, `"ZZ"` for Global): the account's, or the
    /// one asked for.
    pub country: Option<String>,
    /// Every country the charts offer, in the menu's order (the shown one and Global
    /// first).
    pub countries: Vec<Country>,
    pub shelves: Vec<HomeShelf>,
    /// In rank order.
    pub artists: Vec<ChartArtist>,
}

/// A country the charts can show: its code, as the charts are asked for it, and its name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Country {
    pub code: String,
    pub name: String,
}

/// One of the charts' top artists.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ChartArtist {
    pub rank: u32,
    pub artist: ArtistSummary,
    /// "1.35M subscribers", as the chart has it.
    pub subscribers: Option<String>,
    pub trend: Trend,
}

/// Where a chart entry moved since the last chart.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Trend {
    Up,
    Down,
    #[default]
    Same,
}

/// A section of Explore's moods and genres ("Moods & moments", "Genres").
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MoodSection {
    pub title: String,
    pub moods: Vec<Mood>,
}

/// A mood or genre: its name, and the `params` its playlists are browsed with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mood {
    pub title: String,
    pub params: String,
}

/// A mood or genre's page: its name and its shelves of curated playlists.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MoodPage {
    pub title: String,
    pub shelves: Vec<HomeShelf>,
}

/// One day of the listening history ("Today", "Yesterday", "June 2024"), newest first.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct HistoryDay {
    pub title: String,
    pub tracks: Vec<Track>,
}

/// A track's plain lyrics from its Lyrics tab, and the line naming where they come from
/// ("Source: …"), when there is one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlainLyrics {
    pub text: String,
    pub source: Option<String>,
}

/// One line of a track's timed lyrics, from when it starts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimedLine {
    pub start: Duration,
    pub text: String,
}

/// A track's timed lyrics from its Lyrics tab, as the mobile app gets them, sorted by start;
/// and the line naming where they come from ("Source: …"), when there is one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimedLyrics {
    pub lines: Vec<TimedLine>,
    pub source: Option<String>,
}

/// A track's own lyrics: timed lines when YouTube Music has them, else plain text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SongLyrics {
    Timed(TimedLyrics),
    Plain(PlainLyrics),
}

/// The signed-in account, from the account menu. Personal data: never log it.
/// `handle` is the channel handle (`@name`), when the account has one; `photo` is the
/// widest avatar URL (the image CDN resizes it, see the app's cover cache).
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct AccountInfo {
    pub name: String,
    pub handle: Option<String>,
    pub photo: Option<String>,
}

impl fmt::Debug for AccountInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("AccountInfo { .. }")
    }
}

/// Who the requests act as, within one cookie session. `auth_user` is a Google account's
/// index among the accounts signed in to the cookie jar (sent as `X-Goog-AuthUser`);
/// `page_id` picks one of that account's brand (channel) accounts (sent as
/// `X-Goog-PageId`). The default sends neither, so the server uses the identity last
/// chosen in the browser the cookies came from.
///
/// Not secret, but it identifies the account: `Debug` shows only its shape. `key()` is
/// the form stored in the settings and passed through the UI.
#[derive(Clone, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Identity {
    auth_user: Option<u32>,
    page_id: Option<String>,
}

impl Identity {
    /// The highest `X-Goog-AuthUser` index accepted (Google allows about ten logins).
    const MAX_AUTH_USER: u32 = 99;

    /// `page_id` must be all digits, as Google's are.
    pub fn new(auth_user: u32, page_id: Option<&str>) -> Option<Identity> {
        if auth_user > Self::MAX_AUTH_USER {
            return None;
        }
        let page_id = match page_id {
            None => None,
            Some(p) if !p.is_empty() && p.len() <= 32 && p.bytes().all(|b| b.is_ascii_digit()) => {
                Some(p.to_string())
            }
            Some(_) => return None,
        };
        Some(Identity {
            auth_user: Some(auth_user),
            page_id,
        })
    }

    pub fn is_default(&self) -> bool {
        self.auth_user.is_none()
    }

    pub fn auth_user(&self) -> Option<u32> {
        self.auth_user
    }

    pub fn page_id(&self) -> Option<&str> {
        self.page_id.as_deref()
    }

    /// `"2"` for a Google account, `"0/123…"` for one of its brand accounts, `""` for the
    /// default.
    pub fn key(&self) -> String {
        match (self.auth_user, &self.page_id) {
            (None, _) => String::new(),
            (Some(u), None) => u.to_string(),
            (Some(u), Some(p)) => format!("{u}/{p}"),
        }
    }

    /// The inverse of `key`. Anything else (an edited settings file) is `None`.
    pub fn from_key(key: &str) -> Option<Identity> {
        let key = key.trim();
        if key.is_empty() {
            return Some(Identity::default());
        }
        let (user, page) = match key.split_once('/') {
            Some((u, p)) => (u, Some(p)),
            None => (key, None),
        };
        if user.is_empty() || !user.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        Identity::new(user.parse().ok()?, page)
    }
}

impl fmt::Debug for Identity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.auth_user {
            None => f.write_str("Identity(default)"),
            Some(u) if self.page_id.is_some() => write!(f, "Identity(authuser {u}, brand)"),
            Some(u) => write!(f, "Identity(authuser {u})"),
        }
    }
}

/// One identity in the account switcher: a Google account itself, or one of its brand
/// (channel) accounts. Personal data: never log or store it (only `identity.key()`).
/// `byline` is the switcher's second line ("No channel", or the channel's subscribers);
/// `account` is the Google account's email, the switcher's group title.
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct AccountIdentity {
    pub name: String,
    pub handle: Option<String>,
    pub byline: Option<String>,
    pub has_channel: bool,
    pub photo: Option<String>,
    pub account: Option<String>,
    /// The identity the requests act as now.
    pub active: bool,
    pub identity: Identity,
}

impl fmt::Debug for AccountIdentity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "AccountIdentity {{ {:?}, active: {} }}",
            self.identity, self.active
        )
    }
}

/// Videos (songs, music videos, user videos) are `Track`s.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum HomeItem {
    Track(Track),
    Album(AlbumSummary),
    Playlist(PlaylistSummary),
    Artist(ArtistSummary),
}

/// Error details never include response bodies or cookies.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApiError {
    /// The cookie session was rejected (401/403) or the response is signed out.
    AuthExpired,
    Network(String),
    Parse(String),
    NotFound,
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApiError::AuthExpired => f.write_str("session expired, sign in again"),
            ApiError::Network(m) => write!(f, "network error: {m}"),
            ApiError::Parse(m) => write!(f, "unexpected response: {m}"),
            ApiError::NotFound => f.write_str("not found"),
        }
    }
}

impl std::error::Error for ApiError {}
