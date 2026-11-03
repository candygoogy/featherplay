//! `YtMusic`: the requests. Parsing lives in `convert`.
//!
//! Every method sends its request once and never retries. A 401/403, or a
//! signed-out response on a library call, becomes `ApiError::AuthExpired`.
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

use std::fmt;
use std::time::Duration;

use featherplay_core::model::{AlbumId, ArtistId, LikeStatus, PlaylistId, Track, TrackId};
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use ytmapi_rs::YtMusicBuilder;
use ytmapi_rs::auth::BrowserToken;
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

/// A signed-in client acting as one identity. Cheap to clone.
#[derive(Clone)]
pub struct YtMusic {
    inner: ytmapi_rs::YtMusic<BrowserToken>,
    /// Kept to build the client for another identity without signing in again.
    token: BrowserToken,
    identity: Identity,
    /// Without the session or the identity headers: the mobile lyrics request.
    public: reqwest::Client,
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

    /// The same session acting as `identity`. Sends nothing.
    pub fn as_identity(&self, identity: &Identity) -> Result<YtMusic, ApiError> {
        YtMusic::with_token(self.token.clone(), identity.clone())
    }

    pub fn identity(&self) -> &Identity {
        &self.identity
    }

    fn with_token(token: BrowserToken, identity: Identity) -> Result<YtMusic, ApiError> {
        let client = ytmapi_rs::Client::new_from_reqwest_client(http_client(&identity)?);
        let inner = YtMusicBuilder::new_with_client(client)
            .with_auth_token(token.clone())
            .build()
            .map_err(map_error)?;
        Ok(YtMusic {
            inner,
            token,
            identity,
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
    /// as in `liked_songs`.
    pub async fn playlist(&self, id: &PlaylistId) -> Result<Playlist, ApiError> {
        let query =
            GetPlaylistTracksQuery::new(PlaylistID::from_raw(convert::browse_playlist_id(&id.0)));
        let signed_in = convert::bare_playlist_id(&id.0) == "LM";
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
        self.inner.query(query).await.map_err(map_error)
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
        self.inner.query(query).await.map(|_| ()).map_err(map_error)
    }

    /// Creates an empty private playlist and returns its id. One write. The app doesn't
    /// offer it yet; the live test makes its throwaway playlist with it.
    pub async fn create_playlist(&self, title: &str) -> Result<PlaylistId, ApiError> {
        let query = CreatePlaylistQuery::new(title, None, PrivacyStatus::Private);
        let id = self.inner.query(query).await.map_err(map_error)?;
        Ok(PlaylistId(id.get_raw().to_string()))
    }

    /// Renames one of the account's own playlists. One write.
    pub async fn rename_playlist(&self, p: &PlaylistId, title: &str) -> Result<(), ApiError> {
        let query = EditPlaylistQuery::new_title(
            PlaylistID::from_raw(convert::bare_playlist_id(&p.0)),
            title,
        );
        match self.inner.query(query).await.map_err(map_error)? {
            ApiOutcome::Success => Ok(()),
            ApiOutcome::Failure => Err(ApiError::Parse("the rename was refused".into())),
        }
    }

    /// Deletes one of the account's own playlists. One write; it can't be undone.
    pub async fn delete_playlist(&self, p: &PlaylistId) -> Result<(), ApiError> {
        let query = DeletePlaylistQuery::new(PlaylistID::from_raw(convert::bare_playlist_id(&p.0)));
        self.inner.query(query).await.map_err(map_error)
    }

    /// Removes someone else's playlist from the library (YouTube Music's "Remove playlist
    /// from library": the playlist's rating back to indifferent). One write.
    pub async fn remove_playlist_from_library(&self, p: &PlaylistId) -> Result<(), ApiError> {
        let query = RatePlaylistQuery::new(
            PlaylistID::from_raw(convert::bare_playlist_id(&p.0)),
            YtLike::Indifferent,
        );
        self.inner.query(query).await.map_err(map_error)
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
        if convert::logged_in(&first) == Some(false) {
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
    /// devices). One request; signed out, `AuthExpired`.
    pub async fn history(&self) -> Result<Vec<HistoryDay>, ApiError> {
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
        convert::accounts(&self.raw(&AccountSwitcherQuery).await?, &self.identity)
    }

    /// The account menu's raw JSON, for recording the fixture. It holds the
    /// account's personal data: scrub it before it's stored anywhere.
    pub async fn account_menu_raw(&self) -> Result<String, ApiError> {
        self.raw(&AccountMenuQuery).await
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

    async fn raw<Q: Query<BrowserToken>>(&self, query: &Q) -> Result<String, ApiError> {
        self.inner
            .raw_json_query::<Q>(query)
            .await
            .map_err(map_error)
    }

    /// Raw pages of a library grid: the first page, then grid continuations
    /// one after another, at most `MAX_PAGES`. A signed-out first page is
    /// `AuthExpired` and nothing more is fetched.
    async fn grid_pages<Q>(&self, base: &Q) -> Result<Vec<String>, ApiError>
    where
        Q: Query<BrowserToken> + PostQuery,
    {
        let first = self.raw(base).await?;
        if convert::logged_in(&first) == Some(false) {
            return Err(ApiError::AuthExpired);
        }
        let mut token = convert::grid_continuation(&first);
        let mut pages = vec![first];
        while let Some(t) = token.take()
            && pages.len() < MAX_PAGES
        {
            let page = self.raw(&Continuation { base, token: t }).await?;
            token = convert::grid_continuation(&page);
            pages.push(page);
        }
        Ok(pages)
    }

    /// Raw pages of a playlist: the first page, then its continuations one after another,
    /// at most `MAX_PAGES`. Our own paging: ytmapi-rs's stream finds the next token by
    /// parsing the page with its typed parser, so a page it can't read silently ends the
    /// list there. With `signed_in`, a signed-out first page is `AuthExpired` and nothing
    /// more is fetched.
    async fn playlist_pages<Q>(&self, base: &Q, signed_in: bool) -> Result<Vec<String>, ApiError>
    where
        Q: Query<BrowserToken> + PostQuery,
    {
        let first = self.raw(base).await?;
        if signed_in && convert::logged_in(&first) == Some(false) {
            return Err(ApiError::AuthExpired);
        }
        let mut token = convert::playlist_continuation(&first);
        let mut pages = vec![first];
        while let Some(t) = token.take()
            && pages.len() < MAX_PAGES
        {
            let page = self.raw(&Continuation { base, token: t }).await?;
            token = convert::playlist_continuation(&page);
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
        let first = self.raw(base).await?;
        if convert::logged_in(&first) == Some(false) {
            return Err(ApiError::AuthExpired);
        }
        let mut token = convert::shelf_continuation(&first);
        let mut pages = vec![first];
        while let Some(t) = token.take()
            && pages.len() < MAX_PAGES
        {
            let page = self.raw(&Continuation { base, token: t }).await?;
            token = convert::shelf_continuation(&page);
            pages.push(page);
        }
        Ok(pages)
    }
}

/// The Home feed's first page (`FEmusic_home`), fetched as raw JSON.
struct HomeQuery;

impl Query<BrowserToken> for HomeQuery {
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

impl Query<BrowserToken> for Browse<'_> {
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

impl<Q: PostQuery> Query<BrowserToken> for Continuation<'_, Q> {
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
    reqwest::Client::builder()
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
