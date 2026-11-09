//! `YtMusic`: the requests. Parsing lives in `convert`.
//!
//! Every method sends its request once and never retries. A 401/403, or a
//! signed-out response on a library call, becomes `ApiError::AuthExpired`.
//!
//! Signed out (`YtMusic::guest`), the client reads the catalogue as a visitor: search,
//! albums, artists, public playlists, Home, Explore, radio and lyrics. What belongs to an
//! account (the library, Liked songs, the history, ratings, playlist edits, the account
//! menu) is `ApiError::SignInRequired`, without a request.
//!
//! Identities: one cookie session can act as any Google account signed in to its cookie
//! jar, and as any of their brand (channel) accounts. The identity goes in two headers,
//! as yt-dlp sends them: `X-Goog-AuthUser` (the account's index in the jar) and
//! `X-Goog-PageId` (the brand account's page id). ytmusicapi puts the brand id in the
//! body instead (`context.user.onBehalfOfUser`), which ytmapi-rs 0.3.3's typed queries
//! can't carry: their context is fixed (and nests `user` under `client`). The headers are
//! default headers of the reqwest client handed to ytmapi-rs, so every request carries
//! them, typed and raw alike. The SAPISIDHASH is the same for every identity (checked
//! live: each identity's library reads differently). Switching builds a new client
//! around the same `BrowserToken`, with no request.

use std::borrow::Cow;
use std::fmt;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use featherplay_core::model::{AlbumId, ArtistId, LikeStatus, PlaylistId, Track, TrackId};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use ytmapi_rs::YtMusicBuilder;
use ytmapi_rs::auth::{AuthToken, BrowserToken, RawResult};
use ytmapi_rs::common::{
    AlbumID, ApiOutcome, ArtistChannelID, LikeStatus as YtLike, LyricsID, PlaylistID, VideoID,
    YoutubeID,
};
use ytmapi_rs::parse::{ParseFrom, ProcessedResult};
use ytmapi_rs::query::playlist::{DuplicateHandlingMode, PrivacyStatus};
use ytmapi_rs::query::search::{
    AlbumsFilter, ArtistsFilter, FilteredSearch, PlaylistsFilter, SongsFilter,
};
use ytmapi_rs::query::{
    AddPlaylistItemsQuery, CreatePlaylistQuery, DeletePlaylistQuery, EditPlaylistQuery,
    GetAlbumQuery, GetArtistQuery, GetLibraryAlbumsQuery, GetLibraryArtistsQuery,
    GetLibraryPlaylistsQuery, GetLyricsIDQuery, GetLyricsQuery, GetMethod, GetPlaylistTracksQuery,
    GetQuery, GetSearchSuggestionsQuery, GetWatchPlaylistQuery, PostMethod, PostQuery, Query,
    RatePlaylistQuery, RateSongQuery, SearchQuery,
};

use crate::convert::{self, map_error};
use crate::{
    AccountIdentity, AccountInfo, Album, AlbumSummary, ApiError, Artist, ArtistSummary, Charts,
    HistoryDay, HomeFeed, HomeShelf, Identity, MoodPage, MoodSection, Playlist, PlaylistSummary,
    SearchFilter, SearchResults, SearchSuggestions, SongLyrics,
};

/// Upper bound on continuation pages for one list (about 100 rows per page).
const MAX_PAGES: usize = 50;

/// The Google account index header (`authuser`).
const AUTH_USER: &str = "x-goog-authuser";
/// The brand account header (the delegated session's page id).
const PAGE_ID: &str = "x-goog-pageid";

/// A client for YouTube Music: signed in and acting as one identity, or signed out
/// (`guest`). Cheap to clone.
#[derive(Clone)]
pub struct YtMusic {
    session: Session,
    identity: Identity,
    /// `session_number`.
    number: u64,
    /// Without the session or the identity headers: the mobile lyrics request.
    public: reqwest::Client,
}

#[derive(Clone)]
enum Session {
    Account {
        inner: ytmapi_rs::YtMusic<BrowserToken>,
        /// Kept to build the client for another identity without signing in again.
        token: BrowserToken,
        /// Resumed with a kept client version (`resume`): its own sign-in, done once.
        refresh: Option<Arc<Refresh>>,
    },
    /// No cookies.
    Guest(ytmapi_rs::YtMusic<GuestToken>),
}

/// A resumed session's sign-in, as `connect` does it: in the background (`refresh`), or
/// when a request is refused before that's done. Shared by one identity's clones; once it's
/// in, their requests go out with its version.
struct Refresh {
    cookies: String,
    fresh: tokio::sync::OnceCell<Fresh>,
}

struct Fresh {
    inner: ytmapi_rs::YtMusic<BrowserToken>,
    token: BrowserToken,
}

impl Refresh {
    fn new(cookies: String) -> Arc<Refresh> {
        Arc::new(Refresh {
            cookies,
            fresh: tokio::sync::OnceCell::new(),
        })
    }
}

/// `YtMusic::session_number` for `token`: a hash of its `SAPISID` cookie, which the cookie
/// jar's sign-in keeps (a reconnect with the same cookies has the same number). Never 0.
/// Not kept anywhere: it only tells two clients of this run apart.
fn session_of(token: &BrowserToken) -> u64 {
    use std::hash::{Hash, Hasher};
    let sapisid = serde_json::to_value(token)
        .ok()
        .and_then(|v| v.get("sapisid")?.as_str().map(str::to_string))
        .unwrap_or_default();
    let mut h = std::hash::DefaultHasher::new();
    sapisid.hash(&mut h);
    h.finish().max(1)
}

impl fmt::Debug for YtMusic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("YtMusic { .. }")
    }
}

impl YtMusic {
    /// `cookies` is the browser's `cookie:` header for music.youtube.com. Acts as the
    /// identity last chosen in that browser.
    pub async fn from_cookie_header(cookies: &str) -> Result<YtMusic, ApiError> {
        YtMusic::connect(cookies, &Identity::default()).await
    }

    /// Signs in with `cookies` (one request, for the client version) and acts as
    /// `identity`.
    pub async fn connect(cookies: &str, identity: &Identity) -> Result<YtMusic, ApiError> {
        let client = ytmapi_rs::Client::new_from_reqwest_client(http_client(identity)?);
        let token = BrowserToken::from_str(cookies.trim(), &client)
            .await
            .map_err(map_error)?;
        YtMusic::with_token(token, identity.clone())
    }

    /// Signs in with `cookies` and the client version an earlier `connect` read
    /// (`client_version`), and acts as `identity`. Sends nothing: the request `connect`
    /// waits for is left out, so the first page's request can go at once. The cookies are
    /// checked as `connect` checks them (a `SAPISID`, else `AuthExpired`); an odd version is
    /// a `Parse` error, and the caller connects instead.
    pub fn resume(
        cookies: &str,
        identity: &Identity,
        client_version: &str,
    ) -> Result<YtMusic, ApiError> {
        if !plausible_version(client_version) {
            return Err(ApiError::Parse("the kept client version isn't one".into()));
        }
        let cookies = cookies.trim();
        // As ytmapi-rs's `BrowserToken::from_str` reads it.
        let sapisid = cookies
            .split_once("SAPISID=")
            .and_then(|(_, rest)| rest.split_once(';'))
            .map(|(value, _)| value)
            .ok_or(ApiError::AuthExpired)?;
        // ytmapi-rs 0.3.3 builds a `BrowserToken` only from the page's request; its
        // fields are private, but the type is `Deserialize`.
        let token: BrowserToken = serde_json::from_value(serde_json::json!({
            "sapisid": sapisid,
            "client_version": client_version,
            "cookies": cookies,
        }))
        .map_err(|_| ApiError::Parse("building the session token".into()))?;
        YtMusic::resumed(token, cookies.to_string(), identity.clone())
    }

    /// The client version a signed-in client sends (to keep for `resume`): a resumed one's
    /// fresh version once `refresh` has it. `None` for a visitor's, which is worked out
    /// from the date.
    pub fn client_version(&self) -> Option<String> {
        self.account()
            .ok()
            .map(|(_, token)| token.client_version().into_owned())
    }

    /// A resumed session (`resume`) signs in as `connect` does, once (one request for the
    /// page), and its requests use the fresh version from then on. Returns that version;
    /// `None` without a request when the client wasn't resumed. A failure is `connect`'s
    /// (`AuthExpired` for refused cookies, `Network` otherwise), and a later call tries
    /// again.
    pub async fn refresh(&self) -> Result<Option<String>, ApiError> {
        let Session::Account {
            refresh: Some(refresh),
            ..
        } = &self.session
        else {
            return Ok(None);
        };
        let fresh = refresh
            .fresh
            .get_or_try_init(|| async {
                let client =
                    ytmapi_rs::Client::new_from_reqwest_client(http_client(&self.identity)?);
                let token = BrowserToken::from_str(&refresh.cookies, &client)
                    .await
                    .map_err(map_error)?;
                let inner = account_client(&token, &self.identity)?;
                Ok::<_, ApiError>(Fresh { inner, token })
            })
            .await?;
        Ok(Some(fresh.token.client_version().into_owned()))
    }

    /// Signed out: the catalogue as a visitor, without cookies. Sends nothing.
    pub fn guest() -> Result<YtMusic, ApiError> {
        let client = ytmapi_rs::Client::new_from_reqwest_client(http_client(&Identity::default())?);
        let inner = YtMusicBuilder::new_with_client(client)
            .with_auth_token(GuestToken)
            .build()
            .map_err(map_error)?;
        Ok(YtMusic {
            session: Session::Guest(inner),
            identity: Identity::default(),
            number: 0,
            public: http_client(&Identity::default())?,
        })
    }

    /// Whether this client has an account (`connect`), rather than being a visitor
    /// (`guest`).
    pub fn signed_in(&self) -> bool {
        matches!(self.session, Session::Account { .. })
    }

    /// The same session acting as `identity`. Sends nothing. A resumed session not
    /// refreshed yet stays resumed (with a `refresh` of its own: this identity's).
    pub fn as_identity(&self, identity: &Identity) -> Result<YtMusic, ApiError> {
        let Session::Account { token, refresh, .. } = &self.session else {
            return Err(ApiError::SignInRequired);
        };
        let (token, refresh) = match refresh {
            Some(r) => match r.fresh.get() {
                Some(fresh) => (fresh.token.clone(), None),
                None => (token.clone(), Some(Refresh::new(r.cookies.clone()))),
            },
            None => (token.clone(), None),
        };
        YtMusic::account_session(token, refresh, identity.clone(), self.number)
    }

    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    /// Which sign-in this client belongs to: the same for every client of one cookie
    /// session (its identities, a reconnect), another for another sign-in; 0 for a visitor.
    /// Two sign-ins' own identities (`Identity::default`) can be two Google accounts.
    pub fn session_number(&self) -> u64 {
        self.number
    }

    fn with_token(token: BrowserToken, identity: Identity) -> Result<YtMusic, ApiError> {
        let number = session_of(&token);
        YtMusic::account_session(token, None, identity, number)
    }

    fn resumed(
        token: BrowserToken,
        cookies: String,
        identity: Identity,
    ) -> Result<YtMusic, ApiError> {
        let number = session_of(&token);
        YtMusic::account_session(token, Some(Refresh::new(cookies)), identity, number)
    }

    fn account_session(
        token: BrowserToken,
        refresh: Option<Arc<Refresh>>,
        identity: Identity,
        number: u64,
    ) -> Result<YtMusic, ApiError> {
        Ok(YtMusic {
            session: Session::Account {
                inner: account_client(&token, &identity)?,
                token,
                refresh,
            },
            identity,
            number,
            public: http_client(&Identity::default())?,
        })
    }

    /// Uses the filtered search: ytmapi-rs 0.3.3 can't parse the current
    /// unfiltered search layout.
    pub async fn search(&self, q: &str, f: SearchFilter) -> Result<SearchResults, ApiError> {
        let mut out = SearchResults::default();
        match f {
            SearchFilter::Songs => {
                let query: SearchQuery<FilteredSearch<SongsFilter>> = q.into();
                out.songs = convert::search_songs(&self.raw(&query).await?)?;
            }
            SearchFilter::Albums => {
                let query: SearchQuery<FilteredSearch<AlbumsFilter>> = q.into();
                out.albums = convert::search_albums(&self.raw(&query).await?)?;
            }
            SearchFilter::Artists => {
                let query: SearchQuery<FilteredSearch<ArtistsFilter>> = q.into();
                out.artists = convert::search_artists(&self.raw(&query).await?)?;
            }
            SearchFilter::Playlists => {
                let query: SearchQuery<FilteredSearch<PlaylistsFilter>> = q.into();
                out.playlists = convert::search_playlists(&self.raw(&query).await?)?;
            }
        }
        Ok(out)
    }

    /// The filtered search's raw JSON, as `search` gets it. For recording fixtures: scrub
    /// it before it's stored anywhere. One request, read-only.
    pub async fn search_raw(&self, q: &str, f: SearchFilter) -> Result<String, ApiError> {
        match f {
            SearchFilter::Songs => {
                let query: SearchQuery<FilteredSearch<SongsFilter>> = q.into();
                self.raw(&query).await
            }
            SearchFilter::Albums => {
                let query: SearchQuery<FilteredSearch<AlbumsFilter>> = q.into();
                self.raw(&query).await
            }
            SearchFilter::Artists => {
                let query: SearchQuery<FilteredSearch<ArtistsFilter>> = q.into();
                self.raw(&query).await
            }
            SearchFilter::Playlists => {
                let query: SearchQuery<FilteredSearch<PlaylistsFilter>> = q.into();
                self.raw(&query).await
            }
        }
    }

    /// What YouTube Music suggests while `q` is typed: completions, then a few direct
    /// results. One request, read-only.
    pub async fn search_suggestions(&self, q: &str) -> Result<SearchSuggestions, ApiError> {
        convert::search_suggestions(&self.search_suggestions_raw(q).await?)
    }

    /// What YouTube Music suggests while `q` is typed (`music/get_search_suggestions`), raw:
    /// query completions, then a few direct results. ytmapi-rs 0.3.3 has the query but
    /// parses only the completions. One request, read-only; for recording fixtures, scrub
    /// it first (a signed-in reply lists the account's past searches).
    pub async fn search_suggestions_raw(&self, q: &str) -> Result<String, ApiError> {
        self.raw(&GetSearchSuggestionsQuery::new(q)).await
    }

    pub async fn liked_songs(&self) -> Result<Vec<Track>, ApiError> {
        self.account()?;
        let query = GetPlaylistTracksQuery::new(PlaylistID::from_raw("VLLM"));
        let pages = self.playlist_pages(&query, true).await?;
        convert::playlist_tracks(&pages)
    }

    /// Parsed from raw JSON with our own paging: ytmapi-rs 0.3.3 fails on
    /// library playlists whose subtitle has no track count.
    pub async fn library_playlists(&self) -> Result<Vec<PlaylistSummary>, ApiError> {
        convert::library_playlists(&self.grid_pages(&GetLibraryPlaylistsQuery).await?)
    }

    /// Parsed from raw JSON with our own paging: ytmapi-rs 0.3.3 fails on an
    /// empty album library.
    pub async fn library_albums(&self) -> Result<Vec<AlbumSummary>, ApiError> {
        convert::library_albums(&self.grid_pages(&GetLibraryAlbumsQuery::default()).await?)
    }

    /// Paged from raw JSON (`shelf_pages`); ytmapi-rs 0.3.3 parses each page.
    pub async fn library_artists(&self) -> Result<Vec<ArtistSummary>, ApiError> {
        let pages = self.shelf_pages(&GetLibraryArtistsQuery::default()).await?;
        convert::library_artists(&pages)
    }

    /// Accepts the id with or without the `VL` browse prefix. Liked songs
    /// (`LM`) is the account's own, so a signed-out page is `AuthExpired`,
    /// as in `liked_songs`, and a guest's is `SignInRequired`.
    pub async fn playlist(&self, id: &PlaylistId) -> Result<Playlist, ApiError> {
        let query =
            GetPlaylistTracksQuery::new(PlaylistID::from_raw(convert::browse_playlist_id(&id.0)));
        let signed_in = convert::bare_playlist_id(&id.0) == "LM";
        if signed_in {
            self.account()?;
        }
        let pages = self.playlist_pages(&query, signed_in).await?;
        let first = pages.first().ok_or_else(empty)?;
        Ok(Playlist {
            summary: convert::playlist_summary(first, id)?,
            tracks: convert::playlist_tracks(&pages)?,
            pages: pages.len(),
        })
    }

    pub async fn album(&self, id: &AlbumId) -> Result<Album, ApiError> {
        let query = GetAlbumQuery::new(AlbumID::from_raw(id.0.as_str()));
        convert::album(&self.raw(&query).await?, id)
    }

    pub async fn artist(&self, id: &ArtistId) -> Result<Artist, ApiError> {
        let query = GetArtistQuery::new(ArtistChannelID::from_raw(id.0.as_str()));
        convert::artist(&self.raw(&query).await?)
    }

    /// The first batch of the watch playlist ("radio") for `seed`.
    pub async fn radio(&self, seed: &TrackId) -> Result<Vec<Track>, ApiError> {
        let query = GetWatchPlaylistQuery::new_from_video_id(VideoID::from_raw(seed.0.as_str()));
        convert::radio(&self.raw(&query).await?)
    }

    /// One track by its id, for a pasted link: the same watch-playlist request as `radio`,
    /// once, keeping only the seed. `None` when the id doesn't play. Read-only.
    pub async fn track(&self, id: &TrackId) -> Result<Option<Track>, ApiError> {
        let query = GetWatchPlaylistQuery::new_from_video_id(VideoID::from_raw(id.0.as_str()));
        convert::seed_track(&self.raw(&query).await?, id)
    }

    /// The track's own lyrics from its Lyrics tab: the watch-next request for the tab's
    /// browse id, then the tab's page as the mobile app gets it (`mobile_browse`), for timed
    /// lines. Most songs have none; then, or if that request fails, the page as the web
    /// gets it, for the plain text with its verse breaks (the mobile page's untimed lines
    /// have none). One request when the track has no lyrics, three at most. Read-only.
    pub async fn lyrics(&self, id: &TrackId) -> Result<Option<SongLyrics>, ApiError> {
        let Some(browse_id) = self.lyrics_browse_id(id).await? else {
            return Ok(None);
        };
        let timed = mobile_browse(&self.public, MOBILE_BROWSE_URL, &browse_id)
            .await
            .and_then(|raw| convert::timed_lyrics(&raw));
        if let Ok(Some(timed)) = timed {
            return Ok(Some(SongLyrics::Timed(timed)));
        }
        let page = GetLyricsQuery::new(LyricsID::from_raw(browse_id));
        Ok(convert::lyrics(&self.raw(&page).await?)?.map(SongLyrics::Plain))
    }

    /// The Lyrics tab's page as the mobile app gets it, raw (`mobile_browse`), after the
    /// watch-next request for its browse id. `None` when the track has no lyrics. For
    /// recording the fixture. Read-only.
    pub async fn timed_lyrics_raw(&self, id: &TrackId) -> Result<Option<String>, ApiError> {
        let Some(browse_id) = self.lyrics_browse_id(id).await? else {
            return Ok(None);
        };
        mobile_browse(&self.public, MOBILE_BROWSE_URL, &browse_id)
            .await
            .map(Some)
    }

    /// The browse id of the track's Lyrics tab, from the watch-next request; `None` when the
    /// track has no lyrics.
    async fn lyrics_browse_id(&self, id: &TrackId) -> Result<Option<String>, ApiError> {
        let next = GetLyricsIDQuery::new(VideoID::from_raw(id.0.as_str()));
        convert::lyrics_browse_id(&self.raw(&next).await?)
    }

    pub async fn rate(&self, id: &TrackId, s: LikeStatus) -> Result<(), ApiError> {
        let rating = match s {
            LikeStatus::Like => YtLike::Liked,
            LikeStatus::Dislike => YtLike::Disliked,
            LikeStatus::Indifferent => YtLike::Indifferent,
        };
        let query = RateSongQuery::new(VideoID::from_raw(id.0.as_str()), rating);
        self.account()?.0.query(query).await.map_err(map_error)
    }

    /// Fails if a track is already in the playlist (the API's default).
    pub async fn add_to_playlist(&self, p: &PlaylistId, ids: &[TrackId]) -> Result<(), ApiError> {
        if ids.is_empty() {
            return Ok(());
        }
        let query = AddPlaylistItemsQuery::new_from_videos(
            PlaylistID::from_raw(convert::bare_playlist_id(&p.0)),
            ids.iter().map(|t| VideoID::from_raw(t.0.as_str())),
            DuplicateHandlingMode::default(),
        );
        self.account()?
            .0
            .query(query)
            .await
            .map(|_| ())
            .map_err(map_error)
    }

    /// Creates an empty private playlist and returns its id. One write. The app doesn't
    /// offer it yet; the live test makes its throwaway playlist with it.
    pub async fn create_playlist(&self, title: &str) -> Result<PlaylistId, ApiError> {
        let query = CreatePlaylistQuery::new(title, None, PrivacyStatus::Private);
        let id = self.account()?.0.query(query).await.map_err(map_error)?;
        Ok(PlaylistId(id.get_raw().to_string()))
    }

    /// Renames one of the account's own playlists. One write.
    pub async fn rename_playlist(&self, p: &PlaylistId, title: &str) -> Result<(), ApiError> {
        let query = EditPlaylistQuery::new_title(
            PlaylistID::from_raw(convert::bare_playlist_id(&p.0)),
            title,
        );
        match self.account()?.0.query(query).await.map_err(map_error)? {
            ApiOutcome::Success => Ok(()),
            ApiOutcome::Failure => Err(ApiError::Parse("the rename was refused".into())),
        }
    }

    /// Deletes one of the account's own playlists. One write; it can't be undone.
    pub async fn delete_playlist(&self, p: &PlaylistId) -> Result<(), ApiError> {
        let query = DeletePlaylistQuery::new(PlaylistID::from_raw(convert::bare_playlist_id(&p.0)));
        self.account()?.0.query(query).await.map_err(map_error)
    }

    /// Removes someone else's playlist from the library (YouTube Music's "Remove playlist
    /// from library": the playlist's rating back to indifferent). One write.
    pub async fn remove_playlist_from_library(&self, p: &PlaylistId) -> Result<(), ApiError> {
        let query = RatePlaylistQuery::new(
            PlaylistID::from_raw(convert::bare_playlist_id(&p.0)),
            YtLike::Indifferent,
        );
        self.account()?.0.query(query).await.map_err(map_error)
    }

    /// The first batch of a playlist's radio: songs like the playlist's.
    pub async fn playlist_radio(&self, p: &PlaylistId) -> Result<Vec<Track>, ApiError> {
        let query = GetWatchPlaylistQuery::new_from_playlist_id(PlaylistID::from_raw(
            convert::playlist_radio_id(&p.0),
        ));
        convert::radio(&self.raw(&query).await?)
    }

    /// The Home feed: the first page and at most one continuation page, and the token of
    /// the page after them (`home_more`). Parsed from raw JSON: ytmapi-rs 0.3.3 has no Home
    /// query. A continuation that fails (network or shape) leaves the first page's shelves.
    pub async fn home(&self) -> Result<HomeFeed, ApiError> {
        let first = self.raw(&HomeQuery).await?;
        if self.signed_in() && convert::logged_in(&first) == Some(false) {
            return Err(ApiError::AuthExpired);
        }
        let token = convert::home_continuation(&first);
        let mut pages = vec![first];
        if let Some(token) = token {
            let base = &HomeQuery;
            if let Ok(page) = self.raw(&Continuation { base, token }).await {
                pages.push(page);
            }
        }
        convert::home_feed(&pages)
    }

    /// The next page of the Home feed, from `HomeFeed::more`: one request. Home asks for it
    /// as the user scrolls near the end.
    pub async fn home_more(&self, token: &str) -> Result<HomeFeed, ApiError> {
        let query = Continuation {
            base: &HomeQuery,
            token: token.to_string(),
        };
        convert::home_more(&self.raw(&query).await?)
    }

    /// Explore's new releases: albums and singles, then music videos. One request.
    pub async fn new_releases(&self) -> Result<Vec<HomeShelf>, ApiError> {
        convert::new_releases(&self.browse_raw("FEmusic_new_releases", None, None).await?)
    }

    /// Explore's charts for `country` (a code from `Charts::countries`), or for the
    /// account's country. One request.
    pub async fn charts(&self, country: Option<&str>) -> Result<Charts, ApiError> {
        convert::charts(&self.browse_raw("FEmusic_charts", None, country).await?)
    }

    /// Explore's moods and genres. One request.
    pub async fn moods(&self) -> Result<Vec<MoodSection>, ApiError> {
        convert::moods(
            &self
                .browse_raw("FEmusic_moods_and_genres", None, None)
                .await?,
        )
    }

    /// A mood or genre's curated playlists, from its `Mood::params`. One request.
    pub async fn mood_playlists(&self, params: &str) -> Result<MoodPage, ApiError> {
        let raw = self
            .browse_raw("FEmusic_moods_and_genres_category", Some(params), None)
            .await?;
        convert::mood_playlists(&raw)
    }

    /// The listening history, newest day first: the first page (about 200 plays across
    /// devices). One request; signed out, `AuthExpired`; a guest's, `SignInRequired`.
    pub async fn history(&self) -> Result<Vec<HistoryDay>, ApiError> {
        self.account()?;
        convert::history(&self.browse_raw("FEmusic_history", None, None).await?)
    }

    /// The signed-in account's name, handle and photo, from the account menu.
    /// ytmapi-rs 0.3.3 has no account query; this is the request ytmusicapi's
    /// `get_account_info` sends. Read-only.
    pub async fn account_info(&self) -> Result<AccountInfo, ApiError> {
        convert::account_info(&self.account_menu_raw().await?)
    }

    /// Every identity the account switcher lists (the Google accounts signed in to the
    /// cookie jar and their brand accounts), with the one this client acts as marked
    /// `active`. The request YouTube Music's own "Switch account" sends. Read-only.
    pub async fn accounts(&self) -> Result<Vec<AccountIdentity>, ApiError> {
        convert::accounts(
            &self.raw_account(&AccountSwitcherQuery).await?,
            &self.identity,
        )
    }

    /// The account menu's raw JSON, for recording the fixture. It holds the
    /// account's personal data: scrub it before it's stored anywhere.
    pub async fn account_menu_raw(&self) -> Result<String, ApiError> {
        self.raw_account(&AccountMenuQuery).await
    }

    /// One `browse` page as raw JSON: `id` (`FEmusic_charts`, …), with the page's `params`
    /// (a mood's playlists) and the charts' `country`, when given. For recording fixtures:
    /// the history is personal data, so scrub it before it's stored anywhere.
    pub async fn browse_raw(
        &self,
        id: &str,
        params: Option<&str>,
        country: Option<&str>,
    ) -> Result<String, ApiError> {
        self.raw(&Browse {
            id,
            params,
            country,
        })
        .await
    }

    /// A catalogue request, signed in or not. A guest is never told its session ended: a
    /// 401/403 to a visitor asks for an account.
    async fn raw<Q>(&self, query: &Q) -> Result<String, ApiError>
    where
        Q: Query<BrowserToken> + Query<GuestToken>,
    {
        match &self.session {
            Session::Account { .. } => self.raw_account(query).await,
            Session::Guest(inner) => inner
                .raw_json_query::<Q>(query)
                .await
                .map_err(|e| guest_error(map_error(e)))
                .and_then(guest_reply),
        }
    }

    /// A request about the account: `SignInRequired` for a guest, sending nothing. A
    /// resumed session's refused request (any error but 401/403: maybe an old client
    /// version) refreshes, once per session, and goes once more if it went with another
    /// version than the fresh one.
    async fn raw_account<Q: Query<BrowserToken>>(&self, query: &Q) -> Result<String, ApiError> {
        let (client, token) = self.account()?;
        let raw = client.raw_json_query::<Q>(query).await.map_err(map_error)?;
        if !self.is_resumed() || !refused(&raw) {
            return Ok(raw);
        }
        // The version this request went with: a refresh may have landed meanwhile.
        let sent = token.client_version().into_owned();
        match self.refresh().await {
            Ok(Some(fresh)) if fresh != sent => {
                log::info!("the kept client version was refused; trying with the fresh one");
                self.account()?
                    .0
                    .raw_json_query::<Q>(query)
                    .await
                    .map_err(map_error)
            }
            _ => Ok(raw),
        }
    }

    /// A resumed session (`resume`), refreshed or not.
    fn is_resumed(&self) -> bool {
        matches!(
            &self.session,
            Session::Account {
                refresh: Some(_),
                ..
            }
        )
    }

    /// The signed-in client and its token (a resumed session's fresh ones once they're in),
    /// or `SignInRequired`.
    fn account(&self) -> Result<(&ytmapi_rs::YtMusic<BrowserToken>, &BrowserToken), ApiError> {
        match &self.session {
            Session::Account {
                refresh,
                inner,
                token,
            } => Ok(match refresh.as_ref().and_then(|r| r.fresh.get()) {
                Some(fresh) => (&fresh.inner, &fresh.token),
                None => (inner, token),
            }),
            Session::Guest(_) => Err(ApiError::SignInRequired),
        }
    }

    /// Raw pages of a library grid: the first page, then grid continuations
    /// one after another, at most `MAX_PAGES`. A signed-out first page is
    /// `AuthExpired` and nothing more is fetched.
    async fn grid_pages<Q>(&self, base: &Q) -> Result<Vec<String>, ApiError>
    where
        Q: Query<BrowserToken> + PostQuery,
    {
        let first = self.raw_account(base).await?;
        if convert::logged_in(&first) == Some(false) {
            return Err(ApiError::AuthExpired);
        }
        let mut token = convert::grid_continuation(&first);
        let mut pages = vec![first];
        while let Some(t) = token.take()
            && pages.len() < MAX_PAGES
        {
            let page = self.raw_account(&Continuation { base, token: t }).await?;
            token = convert::grid_continuation(&page);
            pages.push(page);
        }
        Ok(pages)
    }

    /// Raw pages of a playlist: the first page, then its continuations one after another,
    /// at most `MAX_PAGES`. Our own paging: ytmapi-rs's stream finds the next token by
    /// parsing the page with its typed parser, so a page it can't read silently ends the
    /// list there. A mix's pages never run out of tokens; `PlaylistPaging` tells where its
    /// list ends. With `signed_in`, a signed-out first page is `AuthExpired` and nothing
    /// more is fetched.
    async fn playlist_pages<Q>(&self, base: &Q, signed_in: bool) -> Result<Vec<String>, ApiError>
    where
        Q: Query<BrowserToken> + Query<GuestToken> + PostQuery,
    {
        let first = self.raw(base).await?;
        if signed_in && convert::logged_in(&first) == Some(false) {
            return Err(ApiError::AuthExpired);
        }
        let mut paging = convert::PlaylistPaging::default();
        let mut token = paging.next_token(&first);
        let mut pages = vec![first];
        while let Some(t) = token.take()
            && pages.len() < MAX_PAGES
        {
            let page = self.raw(&Continuation { base, token: t }).await?;
            token = paging.next_token(&page);
            pages.push(page);
        }
        Ok(pages)
    }

    /// Raw pages of a library shelf (the artists): the first page, then shelf continuations
    /// one after another, at most `MAX_PAGES`. Our own paging, as `playlist_pages`: ytmapi-rs's
    /// stream finds the next token with its typed parser, so a row it can't read silently
    /// ended the list there. A signed-out first page is `AuthExpired` and nothing more is
    /// fetched.
    async fn shelf_pages<Q>(&self, base: &Q) -> Result<Vec<String>, ApiError>
    where
        Q: Query<BrowserToken> + PostQuery,
    {
        let first = self.raw_account(base).await?;
        if convert::logged_in(&first) == Some(false) {
            return Err(ApiError::AuthExpired);
        }
        let mut token = convert::shelf_continuation(&first);
        let mut pages = vec![first];
        while let Some(t) = token.take()
            && pages.len() < MAX_PAGES
        {
            let page = self.raw_account(&Continuation { base, token: t }).await?;
            token = convert::shelf_continuation(&page);
            pages.push(page);
        }
        Ok(pages)
    }
}

/// A visitor's requests: no cookies and no visitor id, and the web client's version from
/// today's date, as ytmapi-rs's visitor token and ytmusicapi send it. ytmapi-rs 0.3.3's own
/// `NoAuthToken` first reads the YouTube Music page for its `VISITOR_DATA`, which the
/// signed-out page no longer has in the config it reads (2026-11-06: only
/// `EOM_VISITOR_DATA`), so it never gets built; the API answers without one.
#[derive(Clone, Debug)]
struct GuestToken;

impl AuthToken for GuestToken {
    fn headers(&self) -> ytmapi_rs::Result<impl IntoIterator<Item = (&str, Cow<'_, str>)>> {
        Ok([
            ("User-Agent", BROWSER_USER_AGENT.into()),
            ("X-Origin", YTM_ORIGIN.into()),
            ("Content-Type", "application/json".into()),
        ])
    }
    /// Worked out for each request, so a long run never sends an old one.
    fn client_version(&self) -> Cow<'_, str> {
        web_client_version(SystemTime::now()).into()
    }
    fn deserialize_response<Q>(raw: RawResult<Q, Self>) -> ytmapi_rs::Result<ProcessedResult<Q>> {
        ProcessedResult::try_from(raw)
    }
}

/// Whether `v` can be a web client version (`1.20261106.01.00`): digits and dots, short.
fn plausible_version(v: &str) -> bool {
    (1..=32).contains(&v.len())
        && v.starts_with(|c: char| c.is_ascii_digit())
        && v.chars().all(|c| c.is_ascii_digit() || c == '.')
}

/// The web client's version for `now`: `1.YYYYMMDD.01.00` (UTC).
fn web_client_version(now: SystemTime) -> String {
    let days = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0);
    let (y, m, d) = civil_date(days);
    format!("1.{y:04}{m:02}{d:02}.01.00")
}

/// The calendar date `days` after 1970-01-01 (Howard Hinnant's `civil_from_days`).
fn civil_date(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + u64::from(m <= 2);
    (y, m, d)
}

/// The Home feed's first page (`FEmusic_home`), fetched as raw JSON.
struct HomeQuery;

impl<A: AuthToken> Query<A> for HomeQuery {
    type Output = Unparsed;
    type Method = PostMethod;
}

impl PostQuery for HomeQuery {
    fn header(&self) -> serde_json::Map<String, serde_json::Value> {
        serde_json::Map::from_iter([("browseId".to_string(), "FEmusic_home".into())])
    }
    fn params(&self) -> Vec<(&str, std::borrow::Cow<'_, str>)> {
        vec![]
    }
    fn path(&self) -> &str {
        "browse"
    }
}

impl ParseFrom<HomeQuery> for Unparsed {
    fn parse_from(_: ProcessedResult<HomeQuery>) -> ytmapi_rs::Result<Self> {
        Ok(Unparsed)
    }
}

/// A `browse` page by id, fetched as raw JSON: Explore's pages (new releases, charts,
/// moods and genres, a mood's playlists) and the history. ytmapi-rs 0.3.3 has no charts
/// or new-releases query, and its history and moods parsers drop what the pages show.
struct Browse<'a> {
    id: &'a str,
    /// A mood or genre's playlists (`FEmusic_moods_and_genres_category`).
    params: Option<&'a str>,
    /// The charts' country (`ZZ` is Global), as ytmusicapi's `get_charts` sends it.
    country: Option<&'a str>,
}

impl<A: AuthToken> Query<A> for Browse<'_> {
    type Output = Unparsed;
    type Method = PostMethod;
}

impl PostQuery for Browse<'_> {
    fn header(&self) -> serde_json::Map<String, serde_json::Value> {
        let mut body = serde_json::Map::from_iter([("browseId".to_string(), self.id.into())]);
        if let Some(p) = self.params {
            body.insert("params".into(), p.into());
        }
        if let Some(c) = self.country {
            body.insert(
                "formData".into(),
                serde_json::json!({ "selectedValues": [c] }),
            );
        }
        body
    }
    fn params(&self) -> Vec<(&str, std::borrow::Cow<'_, str>)> {
        vec![]
    }
    fn path(&self) -> &str {
        "browse"
    }
}

impl ParseFrom<Browse<'_>> for Unparsed {
    fn parse_from(_: ProcessedResult<Browse<'_>>) -> ytmapi_rs::Result<Self> {
        Ok(Unparsed)
    }
}

/// The mobile app's client, as ytmusicapi's `as_mobile` sends it: YouTube Music gives
/// timed lyrics (`timedLyricsData`) to it and not to the web client.
const MOBILE_CLIENT: &str = "ANDROID_MUSIC";
const MOBILE_CLIENT_VERSION: &str = "7.21.50";

/// The `browse` endpoint, for the mobile client's request.
const MOBILE_BROWSE_URL: &str =
    "https://music.youtube.com/youtubei/v1/browse?alt=json&prettyPrint=false";
const YTM_ORIGIN: &str = "https://music.youtube.com";
/// A desktop browser's, as ytmusicapi sends with the mobile client.
const BROWSER_USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/130.0.0.0 Safari/537.36";

/// The body of a `browse` request for page `id` as the mobile app.
fn mobile_browse_body(id: &str) -> serde_json::Value {
    serde_json::json!({
        "context": {
            "client": {
                "clientName": MOBILE_CLIENT,
                "clientVersion": MOBILE_CLIENT_VERSION,
                "hl": "en",
            },
            "user": {},
        },
        "browseId": id,
    })
}

/// A `browse` page by id as the mobile app asks for it, raw, from `url`: the Lyrics tab's page
/// with timed lines. Sent over `public`, without the session: with the web session's cookies
/// the mobile client is answered 400 (checked live 2026-10-31, as in ytmusicapi issue 828),
/// and the page is public. One request, read-only.
async fn mobile_browse(public: &reqwest::Client, url: &str, id: &str) -> Result<String, ApiError> {
    let network = |e: reqwest::Error| ApiError::Network(e.to_string());
    let response = public
        .post(url)
        .header(reqwest::header::ORIGIN, YTM_ORIGIN)
        .header(reqwest::header::USER_AGENT, BROWSER_USER_AGENT)
        .json(&mobile_browse_body(id))
        .send()
        .await
        .map_err(network)?;
    let status = response.status();
    if !status.is_success() {
        return Err(ApiError::Network(format!(
            "mobile lyrics page: status {status}"
        )));
    }
    response.text().await.map_err(network)
}

/// The account menu (`account/account_menu`), fetched as raw JSON.
struct AccountMenuQuery;

impl Query<BrowserToken> for AccountMenuQuery {
    type Output = Unparsed;
    type Method = PostMethod;
}

impl PostQuery for AccountMenuQuery {
    fn header(&self) -> serde_json::Map<String, serde_json::Value> {
        serde_json::Map::new()
    }
    fn params(&self) -> Vec<(&str, std::borrow::Cow<'_, str>)> {
        vec![]
    }
    fn path(&self) -> &str {
        "account/account_menu"
    }
}

impl ParseFrom<AccountMenuQuery> for Unparsed {
    fn parse_from(_: ProcessedResult<AccountMenuQuery>) -> ytmapi_rs::Result<Self> {
        Ok(Unparsed)
    }
}

/// The account switcher, fetched as raw JSON: the GET behind the account menu's
/// "Switch account" (`getAccountSwitcherEndpoint`). Innertube's `account/accounts_list`
/// answers the music client with no accounts.
struct AccountSwitcherQuery;

impl Query<BrowserToken> for AccountSwitcherQuery {
    type Output = Unparsed;
    type Method = GetMethod;
}

impl GetQuery for AccountSwitcherQuery {
    fn url(&self) -> &str {
        "https://music.youtube.com/getAccountSwitcherEndpoint"
    }
    fn params(&self) -> Vec<(&str, std::borrow::Cow<'_, str>)> {
        vec![]
    }
}

impl ParseFrom<AccountSwitcherQuery> for Unparsed {
    fn parse_from(_: ProcessedResult<AccountSwitcherQuery>) -> ytmapi_rs::Result<Self> {
        Ok(Unparsed)
    }
}

/// The next page of `base`, fetched as raw JSON. Same request shape as
/// ytmapi-rs's own continuation query, for lists its typed parser can't read.
struct Continuation<'a, Q> {
    base: &'a Q,
    token: String,
}

#[derive(Debug)]
struct Unparsed;

impl<Q: PostQuery, A: AuthToken> Query<A> for Continuation<'_, Q> {
    type Output = Unparsed;
    type Method = PostMethod;
}

impl<Q: PostQuery> PostQuery for Continuation<'_, Q> {
    fn header(&self) -> serde_json::Map<String, serde_json::Value> {
        self.base.header()
    }
    fn params(&self) -> Vec<(&str, std::borrow::Cow<'_, str>)> {
        vec![
            ("ctoken", self.token.as_str().into()),
            ("continuation", self.token.as_str().into()),
        ]
    }
    fn path(&self) -> &str {
        self.base.path()
    }
}

impl<Q> ParseFrom<Continuation<'_, Q>> for Unparsed {
    fn parse_from(_: ProcessedResult<Continuation<'_, Q>>) -> ytmapi_rs::Result<Self> {
        Ok(Unparsed)
    }
}

/// The HTTP client for `identity`: its headers go on every request. Nothing for the
/// default identity. Every request carries the cookies, so a redirect is followed only
/// to a `cookie_host`. HTTP/1.1 only: since 2026-10-30, music.youtube.com answers a
/// browser user agent (ytmapi-rs always sends one) over HTTP/2 with a response h2
/// rejects as a protocol error, so every sign-in failed.
fn http_client(identity: &Identity) -> Result<reqwest::Client, ApiError> {
    http_client_with(identity, TIMEOUT)
}

/// A connection to YouTube opens in well under a second; after 5 s it isn't coming.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
/// The biggest response (an artist page, ~450 KB of JSON before gzip) takes a couple of
/// seconds on a slow link; library continuations are one request per page, so a long
/// library doesn't add up against it. 20 s also ends a half-open connection.
const TIMEOUT: Duration = Duration::from_secs(20);

/// `http_client` with another total timeout per request (tests use a short one).
fn http_client_with(identity: &Identity, timeout: Duration) -> Result<reqwest::Client, ApiError> {
    featherplay_proxy::client()
        .connect_timeout(CONNECT_TIMEOUT)
        .timeout(timeout)
        .http1_only()
        .default_headers(identity_headers(identity))
        .redirect(redirect_policy())
        .build()
        .map_err(|e| ApiError::Network(format!("building the HTTP client: {e}")))
}

/// The most redirects one request follows.
const MAX_REDIRECTS: usize = 5;

/// Follows a redirect only over HTTPS to YouTube or Google. reqwest already drops the
/// `Cookie` and `Authorization` headers on a redirect to another host; this also keeps
/// the request, and any body a 307 would send again, from going anywhere else.
fn redirect_policy() -> reqwest::redirect::Policy {
    reqwest::redirect::Policy::custom(|attempt| {
        if attempt.previous().len() >= MAX_REDIRECTS {
            attempt.error("too many redirects")
        } else if !cookie_host(attempt.url()) {
            attempt.error("redirect away from YouTube refused")
        } else {
            attempt.follow()
        }
    })
}

/// HTTPS on `youtube.com`, `google.com` or one of their subdomains: where the session
/// cookies may go.
fn cookie_host(url: &reqwest::Url) -> bool {
    const DOMAINS: [&str; 2] = ["youtube.com", "google.com"];
    url.scheme() == "https"
        && url.host_str().is_some_and(|host| {
            DOMAINS.iter().any(|d| {
                host == *d
                    || host
                        .strip_suffix(d)
                        .is_some_and(|sub| sub.len() > 1 && sub.ends_with('.'))
            })
        })
}

fn identity_headers(identity: &Identity) -> HeaderMap {
    let mut headers = HeaderMap::new();
    let mut put = |name: &'static str, value: &str| {
        // Digits only (see `Identity::new`), so the value is always a valid header.
        if let Ok(v) = HeaderValue::from_str(value) {
            headers.insert(HeaderName::from_static(name), v);
        }
    };
    if let Some(user) = identity.auth_user() {
        put(AUTH_USER, &user.to_string());
    }
    if let Some(page) = identity.page_id() {
        put(PAGE_ID, page);
    }
    headers
}

/// What a guest is told: a refusal (401/403) means the request needs an account, as no
/// session can have ended.
fn guest_error(e: ApiError) -> ApiError {
    match e {
        ApiError::AuthExpired => ApiError::SignInRequired,
        other => other,
    }
}

/// A guest's answer as the parsers get it, unless it's a refusal: ytmapi-rs hands back the
/// body whatever the HTTP status, and the parsers would read its `error.code` 401/403 as an
/// ended session.
fn guest_reply(raw: String) -> Result<String, ApiError> {
    match error_code(&raw) {
        Some(401 | 403) => Err(ApiError::SignInRequired),
        _ => Ok(raw),
    }
}

/// The `error.code` of an answer, if it's an error.
fn error_code(raw: &str) -> Option<u64> {
    #[derive(serde::Deserialize)]
    struct Reply {
        error: Option<Code>,
    }
    #[derive(serde::Deserialize)]
    struct Code {
        code: Option<u64>,
    }
    serde_json::from_str::<Reply>(raw)
        .ok()
        .and_then(|r| r.error)
        .and_then(|e| e.code)
}

/// An error answer to a signed-in request that a fresh client version might fix: any but a
/// refused session (401/403). Error answers are small; a page isn't parsed for it.
fn refused(raw: &str) -> bool {
    const MOST: usize = 4096;
    raw.len() <= MOST && error_code(raw).is_some_and(|c| c != 401 && c != 403)
}

/// The ytmapi-rs client for a signed-in `identity` with `token`. Sends nothing.
fn account_client(
    token: &BrowserToken,
    identity: &Identity,
) -> Result<ytmapi_rs::YtMusic<BrowserToken>, ApiError> {
    let client = ytmapi_rs::Client::new_from_reqwest_client(http_client(identity)?);
    YtMusicBuilder::new_with_client(client)
        .with_auth_token(token.clone())
        .build()
        .map_err(map_error)
}

fn empty() -> ApiError {
    ApiError::Parse("empty response".into())
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Instant;

    use super::*;

    /// A local port that takes connections and never answers.
    fn silent_server() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let url = format!("http://{}/browse", listener.local_addr().unwrap());
        std::thread::spawn(move || {
            let mut held = Vec::new();
            for conn in listener.incoming() {
                held.push(conn);
            }
        });
        url
    }

    #[tokio::test]
    async fn a_server_that_never_answers_times_out() {
        let url = silent_server();
        let client = http_client_with(&Identity::default(), Duration::from_millis(300)).unwrap();
        let t = Instant::now();
        let sent = tokio::time::timeout(Duration::from_secs(5), client.get(&url).send()).await;
        let err = sent
            .expect("neither an answer nor an error within 5 s")
            .expect_err("an answer from a silent server");
        assert!(err.is_timeout(), "{err}");
        assert!(t.elapsed() < Duration::from_secs(2), "{:?}", t.elapsed());
    }

    /// Serves one request on a local port and hands back its head, lower-cased.
    fn one_request() -> (String, std::thread::JoinHandle<String>) {
        serve_once("HTTP/1.1 200 OK\r\ncontent-length: 2\r\nconnection: close\r\n\r\n{}".into())
    }

    /// Serves one request on a local port with `reply`, and hands back its head,
    /// lower-cased.
    fn serve_once(reply: String) -> (String, std::thread::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let url = format!(
            "http://{}/getAccountSwitcherEndpoint",
            listener.local_addr().unwrap()
        );
        let served = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().expect("accept");
            let mut head = Vec::new();
            let mut buf = [0u8; 1024];
            while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = stream.read(&mut buf).expect("read");
                if n == 0 {
                    break;
                }
                head.extend_from_slice(&buf[..n]);
            }
            stream.write_all(reply.as_bytes()).expect("write");
            String::from_utf8_lossy(&head).to_lowercase()
        });
        (url, served)
    }

    /// Sends one GET the way ytmapi-rs does (its `Client`, with per-request headers like
    /// the browser token's) over the client built for `identity`.
    async fn head_sent_as(identity: &Identity) -> String {
        let (url, served) = one_request();
        let client = ytmapi_rs::Client::new_from_reqwest_client(http_client(identity).unwrap());
        let headers = [
            ("Authorization", std::borrow::Cow::from("SAPISIDHASH 1_x")),
            ("Cookie", "SAPISID=x".into()),
        ];
        client.get_query(&url, headers, &()).await.expect("request");
        served.join().expect("server")
    }

    #[tokio::test]
    async fn a_brand_account_sends_its_index_and_page_id() {
        let brand = Identity::new(0, Some("100000000000000000002")).unwrap();
        let head = head_sent_as(&brand).await;
        assert!(
            head.contains("\r\nx-goog-authuser: 0\r\n"),
            "authuser missing"
        );
        assert!(head.contains("\r\nx-goog-pageid: 100000000000000000002\r\n"));
        // ytmapi-rs's own headers still go out alongside.
        assert!(head.contains("\r\nauthorization: sapisidhash 1_x\r\n"));
        assert!(head.contains("\r\ncookie: sapisid=x\r\n"));
    }

    #[tokio::test]
    async fn a_google_account_sends_its_index_only() {
        let second = Identity::new(2, None).unwrap();
        let head = head_sent_as(&second).await;
        assert!(head.contains("\r\nx-goog-authuser: 2\r\n"));
        assert!(!head.contains("x-goog-pageid"));
    }

    #[tokio::test]
    async fn the_default_identity_sends_neither() {
        let head = head_sent_as(&Identity::default()).await;
        assert!(!head.contains("x-goog-authuser"));
        assert!(!head.contains("x-goog-pageid"));
    }

    #[tokio::test]
    async fn the_mobile_lyrics_request_goes_without_the_session() {
        // The mobile client is refused (400) with the web session's cookies; lyrics are public.
        let (url, served) = one_request();
        let public = http_client(&Identity::default()).unwrap();
        mobile_browse(&public, &url, "MPLYt_x")
            .await
            .expect("request");
        let head = served.join().expect("server");
        assert!(head.starts_with("post "), "{head}");
        assert!(!head.contains("\r\ncookie:"), "cookies sent");
        assert!(!head.contains("\r\nauthorization:"), "authorization sent");
        assert!(head.contains("\r\norigin: https://music.youtube.com\r\n"));
    }

    #[test]
    fn the_mobile_lyrics_request_asks_as_the_android_app() {
        let body = mobile_browse_body("MPLYt_x");
        assert_eq!(body["browseId"], "MPLYt_x");
        assert_eq!(body["context"]["client"]["clientName"], "ANDROID_MUSIC");
        assert_eq!(body["context"]["client"]["clientVersion"], "7.21.50");
    }

    #[test]
    fn a_visitor_sends_the_web_client_version_for_the_day() {
        let at = |days: u64| SystemTime::UNIX_EPOCH + Duration::from_secs(days * 86_400 + 3_600);
        assert_eq!(web_client_version(at(20_763)), "1.20261106.01.00");
        assert_eq!(web_client_version(at(11_016)), "1.20000229.01.00");
    }

    #[test]
    fn a_refusal_to_a_visitor_asks_to_sign_in_never_ends_a_session() {
        // ytmapi-rs hands back the body whatever the status: the refusal is in the JSON.
        let refused = r#"{"error":{"code":403,"message":"The caller does not have permission"}}"#;
        assert_eq!(guest_reply(refused.into()), Err(ApiError::SignInRequired));
        let fine = r#"{"responseContext":{},"contents":{}}"#;
        assert_eq!(guest_reply(fine.into()).as_deref(), Ok(fine));
        let other = r#"{"error":{"code":500}}"#;
        assert_eq!(guest_reply(other.into()).as_deref(), Ok(other));
    }

    #[test]
    fn a_resumed_session_sends_the_kept_client_version() {
        // Not async: building it sends nothing.
        let api = YtMusic::resume(
            "A=1; SAPISID=abc/def; B=2",
            &Identity::default(),
            "1.20261101.01.00",
        )
        .expect("resume");
        assert!(api.signed_in());
        assert_eq!(api.client_version().as_deref(), Some("1.20261101.01.00"));
        assert_eq!(YtMusic::guest().unwrap().client_version(), None);
    }

    #[test]
    fn a_session_resumes_only_with_its_sapisid_and_a_version_that_looks_like_one() {
        let id = Identity::default();
        assert_eq!(
            YtMusic::resume("A=1; B=2", &id, "1.20261101.01.00").map(drop),
            Err(ApiError::AuthExpired)
        );
        for odd in ["", "1.2026\"x", "<html>", &"1".repeat(40)] {
            assert!(YtMusic::resume("SAPISID=abc;", &id, odd).is_err(), "{odd}");
        }
    }

    #[test]
    fn another_identity_of_a_resumed_session_is_resumed_too() {
        let api = YtMusic::resume("SAPISID=abc;", &Identity::default(), "1.20261101.01.00")
            .expect("resume");
        let other = api.as_identity(&Identity::new(2, None).unwrap()).unwrap();
        assert!(other.is_resumed());
        assert_eq!(other.client_version().as_deref(), Some("1.20261101.01.00"));
        // The same sign-in, and so is the same cookie session again (a reconnect); another
        // sign-in is another, whatever its identity.
        assert_eq!(other.session_number(), api.session_number());
        let again =
            YtMusic::resume("A=1; SAPISID=abc; B=2", &Identity::default(), "1.2").expect("resume");
        assert_eq!(again.session_number(), api.session_number());
        let another = YtMusic::resume("SAPISID=xyz;", &Identity::default(), "1.20261101.01.00")
            .expect("resume");
        assert_ne!(another.session_number(), api.session_number());
        assert_ne!(api.session_number(), 0);
        assert_eq!(YtMusic::guest().unwrap().session_number(), 0);
    }

    #[tokio::test]
    async fn only_a_resumed_session_refreshes() {
        // Nothing listens anywhere here: a request would fail as a network error instead.
        let token: BrowserToken = serde_json::from_value(serde_json::json!({
            "sapisid": "abc", "client_version": "1.20261101.01.00", "cookies": "SAPISID=abc;"
        }))
        .unwrap();
        let connected = YtMusic::with_token(token, Identity::default()).unwrap();
        assert!(!connected.is_resumed());
        assert_eq!(connected.refresh().await, Ok(None));
        assert_eq!(YtMusic::guest().unwrap().refresh().await, Ok(None));
    }

    #[test]
    fn an_error_answer_but_a_refused_session_may_be_the_client_version() {
        assert!(refused(
            r#"{"error":{"code":400,"message":"Precondition check failed."}}"#
        ));
        assert!(refused(r#"{"error":{"code":404}}"#));
        assert!(!refused(r#"{"error":{"code":401}}"#), "the session ended");
        assert!(!refused(r#"{"error":{"code":403}}"#), "the session ended");
        assert!(!refused(r#"{"responseContext":{},"contents":{}}"#));
        let page = format!(
            r#"{{"contents":"{}","error":{{"code":400}}}}"#,
            "x".repeat(5000)
        );
        assert!(!refused(&page), "a page isn't parsed for it");
    }

    #[tokio::test]
    async fn a_visitor_is_asked_to_sign_in_for_the_account_without_a_request() {
        // Nothing listens anywhere here: a request would fail as a network error instead.
        let guest = YtMusic::guest().expect("guest");
        assert!(!guest.signed_in());
        let id = TrackId("x".into());
        assert_eq!(
            guest.library_playlists().await,
            Err(ApiError::SignInRequired)
        );
        assert_eq!(guest.history().await, Err(ApiError::SignInRequired));
        assert_eq!(
            guest.playlist(&PlaylistId("LM".into())).await.map(drop),
            Err(ApiError::SignInRequired)
        );
        assert_eq!(
            guest.rate(&id, LikeStatus::Like).await,
            Err(ApiError::SignInRequired)
        );
        assert_eq!(
            guest.as_identity(&Identity::default()).map(drop),
            Err(ApiError::SignInRequired)
        );
    }

    #[test]
    fn cookies_may_only_go_to_youtube_and_google_over_https() {
        let ok = |u: &str| cookie_host(&reqwest::Url::parse(u).unwrap());
        assert!(ok("https://music.youtube.com/youtubei/v1/browse"));
        assert!(ok("https://youtube.com/"));
        assert!(ok("https://consent.youtube.com/m"));
        assert!(ok("https://accounts.google.com/ServiceLogin"));
        assert!(!ok("http://music.youtube.com/"), "plain HTTP");
        assert!(!ok("https://music.youtube.com.example.net/"));
        assert!(!ok("https://notyoutube.com/"));
        assert!(!ok("https://youtube.com.example/"));
        assert!(!ok("https://example.net/?u=music.youtube.com"));
    }

    #[tokio::test]
    async fn a_redirect_away_from_youtube_is_not_followed() {
        // Stands for any other host: it must never see the request.
        let (target, elsewhere) = one_request();
        let (url, served) = serve_once(format!(
            "HTTP/1.1 307 Temporary Redirect\r\nlocation: {target}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
        ));
        let client =
            ytmapi_rs::Client::new_from_reqwest_client(http_client(&Identity::default()).unwrap());
        let headers = [("Cookie", std::borrow::Cow::from("SAPISID=x"))];
        let sent = client.get_query(&url, headers, &()).await;
        served.join().expect("server");
        assert!(sent.is_err(), "the redirect was followed");
        assert!(
            !elsewhere.is_finished(),
            "the redirect target was contacted"
        );
    }
}
