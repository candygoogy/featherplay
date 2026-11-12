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

mod guest;
mod http;
mod query;
#[cfg(test)]
mod tests;

use std::fmt;
use std::future::Future;
use std::ops::ControlFlow;
use std::pin::Pin;
use std::sync::Arc;

use featherplay_core::model::{AlbumId, ArtistId, LikeStatus, PlaylistId, Track, TrackId};
use ytmapi_rs::YtMusicBuilder;
use ytmapi_rs::auth::{AuthToken, BrowserToken};
use ytmapi_rs::common::{
    AlbumID, ApiOutcome, ArtistChannelID, LikeStatus as YtLike, LyricsID, PlaylistID, VideoID,
    YoutubeID,
};
use ytmapi_rs::query::playlist::{DuplicateHandlingMode, PrivacyStatus};
use ytmapi_rs::query::search::{
    AlbumsFilter, ArtistsFilter, FilteredSearch, PlaylistsFilter, SongsFilter,
};
use ytmapi_rs::query::{
    AddPlaylistItemsQuery, CreatePlaylistQuery, DeletePlaylistQuery, EditPlaylistQuery,
    GetAlbumQuery, GetArtistQuery, GetLibraryAlbumsQuery, GetLibraryArtistsQuery,
    GetLibraryPlaylistsQuery, GetLyricsIDQuery, GetLyricsQuery, GetPlaylistTracksQuery,
    GetSearchSuggestionsQuery, GetWatchPlaylistQuery, PostQuery, Query, RatePlaylistQuery,
    RateSongQuery, SearchQuery,
};

use self::guest::{GuestToken, plausible_version};
use self::http::{account_client, empty, guest_error, guest_reply, http_client, refused};
use self::query::{
    AccountMenuQuery, AccountSwitcherQuery, Browse, HomeQuery, MOBILE_BROWSE_URL, Post,
    mobile_browse,
};
use crate::convert::{self, PlaylistRest, map_error};
use crate::{
    AccountIdentity, AccountInfo, Album, AlbumSummary, ApiError, Artist, ArtistSummary, Charts,
    HistoryDay, HomeFeed, HomeShelf, Identity, MoodPage, MoodSection, Playlist, PlaylistPart,
    PlaylistSummary, SearchFilter, SearchResults, SearchSuggestions, SongLyrics,
};

/// Upper bound on continuation pages for one list (about 100 rows per page).
const MAX_PAGES: usize = 50;

/// The Google account index header (`authuser`).
const AUTH_USER: &str = "x-goog-authuser";
/// The brand account header (the delegated session's page id).
const PAGE_ID: &str = "x-goog-pageid";

/// A request's future, boxed in this crate (`raw_post`).
type Boxed<'a, T> = Pin<Box<dyn Future<Output = Result<T, ApiError>> + Send + 'a>>;

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
    /// again. Boxed (`raw_post`).
    #[inline(never)]
    pub fn refresh(&self) -> Boxed<'_, Option<String>> {
        Box::pin(async move {
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
        })
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
        let pages = self.playlist_pages(Post::of(&query), true).await?;
        convert::playlist_tracks(&pages)
    }

    /// Parsed from raw JSON with our own paging: ytmapi-rs 0.3.3 fails on
    /// library playlists whose subtitle has no track count.
    pub async fn library_playlists(&self) -> Result<Vec<PlaylistSummary>, ApiError> {
        convert::library_playlists(&self.grid_pages(Post::of(&GetLibraryPlaylistsQuery)).await?)
    }

    /// Parsed from raw JSON with our own paging: ytmapi-rs 0.3.3 fails on an
    /// empty album library.
    pub async fn library_albums(&self) -> Result<Vec<AlbumSummary>, ApiError> {
        convert::library_albums(
            &self
                .grid_pages(Post::of(&GetLibraryAlbumsQuery::default()))
                .await?,
        )
    }

    /// Paged from raw JSON (`shelf_pages`); ytmapi-rs 0.3.3 parses each page.
    pub async fn library_artists(&self) -> Result<Vec<ArtistSummary>, ApiError> {
        let pages = self
            .shelf_pages(Post::of(&GetLibraryArtistsQuery::default()))
            .await?;
        convert::library_artists(&pages)
    }

    /// Accepts the id with or without the `VL` browse prefix. Liked songs
    /// (`LM`) is the account's own, so a signed-out page is `AuthExpired`,
    /// as in `liked_songs`, and a guest's is `SignInRequired`.
    pub async fn playlist(&self, id: &PlaylistId) -> Result<Playlist, ApiError> {
        let mut whole: Option<Playlist> = None;
        let pages = self
            .playlist_by_page(id, |part| {
                match (part, &mut whole) {
                    (PlaylistPart::First(p), _) => whole = Some(p),
                    (PlaylistPart::More(tracks), Some(p)) => p.tracks.extend(tracks),
                    (PlaylistPart::More(_), None) => {}
                }
                ControlFlow::Continue(())
            })
            .await?;
        let mut playlist = whole.ok_or_else(empty)?;
        playlist.pages = pages;
        Ok(playlist)
    }

    /// `playlist` a page at a time: each is handed to `each` as soon as it's read, before
    /// the next is asked for, so a long list (a mix of 500 songs is 19 pages, ~10 s) shows
    /// its first songs after one request. Pages are fetched one after another, at most
    /// `MAX_PAGES`, to the end of the list (`PlaylistPaging`), or until `each` breaks.
    /// Returns how many pages were read. A page that fails after the first ends it with
    /// that error; the pages handed on stay handed on.
    pub async fn playlist_by_page(
        &self,
        id: &PlaylistId,
        mut each: impl FnMut(PlaylistPart) -> ControlFlow<()> + Send,
    ) -> Result<usize, ApiError> {
        let (first, rest) = self.playlist_first(id).await?;
        let query = playlist_query(id);
        let base = Post::of(&query);
        let base = &base;
        let fetch = move |token: String| self.raw_post(base.next(token));
        follow_playlist(first, rest, fetch, &mut each).await
    }

    /// A playlist's first page, one request: its header and songs, and where the list goes
    /// on (`playlist_rest`), if it does. Liked songs as in `playlist`.
    pub async fn playlist_first(
        &self,
        id: &PlaylistId,
    ) -> Result<(Playlist, Option<PlaylistRest>), ApiError> {
        let signed_in = convert::bare_playlist_id(&id.0) == "LM";
        if signed_in {
            self.account()?;
        }
        let first = self.raw(&playlist_query(id)).await?;
        if signed_in && convert::logged_in(&first) == Some(false) {
            return Err(ApiError::AuthExpired);
        }
        convert::playlist_first(&first, id)
    }

    /// The rest of a playlist from where `playlist_first` stopped, a page at a time as
    /// `playlist_by_page` reads them. Returns how many pages were read, the first included.
    pub async fn playlist_rest(
        &self,
        rest: PlaylistRest,
        mut each: impl FnMut(PlaylistPart) -> ControlFlow<()> + Send,
    ) -> Result<usize, ApiError> {
        let query = playlist_query(rest.id());
        let base = Post::of(&query);
        let base = &base;
        let fetch = move |token: String| self.raw_post(base.next(token));
        follow_rest(rest, fetch, &mut each).await
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
        if let Some(token) = token
            && let Ok(page) = self.raw_post(Post::of(&HomeQuery).next(token)).await
        {
            pages.push(page);
        }
        convert::home_feed(&pages)
    }

    /// The next page of the Home feed, from `HomeFeed::more`: one request. Home asks for it
    /// as the user scrolls near the end.
    pub async fn home_more(&self, token: &str) -> Result<HomeFeed, ApiError> {
        let query = Post::of(&HomeQuery).next(token.to_string());
        convert::home_more(&self.raw_post(query).await?)
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
    /// `active`. The request YouTube Music's own "Switch account" sends. Read-only. Boxed
    /// (`raw_post`).
    #[inline(never)]
    pub fn accounts(&self) -> Boxed<'_, Vec<AccountIdentity>> {
        Box::pin(async move {
            convert::accounts(
                &self.raw_account(&AccountSwitcherQuery).await?,
                &self.identity,
            )
        })
    }

    /// The account menu's raw JSON, for recording the fixture. It holds the
    /// account's personal data: scrub it before it's stored anywhere.
    pub async fn account_menu_raw(&self) -> Result<String, ApiError> {
        self.account_post(&Post::of(&AccountMenuQuery)).await
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

    /// A catalogue request, signed in or not: `query` as a `Post`, sent by `raw_post`.
    /// The bounds keep account-only queries out.
    fn raw<'a, Q>(
        &'a self,
        query: &'a Q,
    ) -> impl Future<Output = Result<String, ApiError>> + Send + 'a
    where
        Q: Query<BrowserToken> + Query<GuestToken> + PostQuery,
    {
        self.raw_post(Post::of(query))
    }

    /// A catalogue request, signed in or not. A guest is never told its session ended: a
    /// 401/403 to a visitor asks for an account.
    ///
    /// Boxed here, as `account_post`, `refresh` and `accounts` are: a future is compiled
    /// in each crate that polls it, and the app and the controller crate both reach these.
    #[inline(never)]
    fn raw_post<'a>(&'a self, query: Post<'a>) -> Boxed<'a, String> {
        Box::pin(async move {
            match &self.session {
                Session::Account { .. } => self.account_post(&query).await,
                Session::Guest(inner) => inner
                    .raw_json_query::<Post>(&query)
                    .await
                    .map_err(|e| guest_error(map_error(e)))
                    .and_then(guest_reply),
            }
        })
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

    /// `raw_account` for a `Post`, which most account requests are. Boxed (`raw_post`).
    #[inline(never)]
    fn account_post<'a>(&'a self, query: &'a Post<'_>) -> Boxed<'a, String> {
        Box::pin(self.raw_account(query))
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
    async fn grid_pages(&self, base: Post<'_>) -> Result<Vec<String>, ApiError> {
        let first = self.account_post(&base).await?;
        if convert::logged_in(&first) == Some(false) {
            return Err(ApiError::AuthExpired);
        }
        let mut token = convert::grid_continuation(&first);
        let mut pages = vec![first];
        while let Some(t) = token.take()
            && pages.len() < MAX_PAGES
        {
            let page = self.account_post(&base.next(t)).await?;
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
    async fn playlist_pages(
        &self,
        base: Post<'_>,
        signed_in: bool,
    ) -> Result<Vec<String>, ApiError> {
        let first = self.raw_post(base.clone()).await?;
        if signed_in && convert::logged_in(&first) == Some(false) {
            return Err(ApiError::AuthExpired);
        }
        let mut paging = convert::PlaylistPaging::default();
        let mut token = paging.next_token(&first);
        let mut pages = vec![first];
        while let Some(t) = token.take()
            && pages.len() < MAX_PAGES
        {
            let page = self.raw_post(base.next(t)).await?;
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
    async fn shelf_pages(&self, base: Post<'_>) -> Result<Vec<String>, ApiError> {
        let first = self.account_post(&base).await?;
        if convert::logged_in(&first) == Some(false) {
            return Err(ApiError::AuthExpired);
        }
        let mut token = convert::shelf_continuation(&first);
        let mut pages = vec![first];
        while let Some(t) = token.take()
            && pages.len() < MAX_PAGES
        {
            let page = self.account_post(&base.next(t)).await?;
            token = convert::shelf_continuation(&page);
            pages.push(page);
        }
        Ok(pages)
    }
}

/// The request for a playlist's first page; its continuations are asked with the same.
fn playlist_query(id: &PlaylistId) -> GetPlaylistTracksQuery<'static> {
    GetPlaylistTracksQuery::new(PlaylistID::from_raw(convert::browse_playlist_id(&id.0)))
}

/// Follows a playlist from its `first` page and where the list goes on after it (`rest`):
/// hands `first` to `each`, then the pages after as `follow_rest` does. Returns how many
/// pages were read.
async fn follow_playlist<F, Fut>(
    first: Playlist,
    rest: Option<PlaylistRest>,
    fetch: F,
    each: &mut impl FnMut(PlaylistPart) -> ControlFlow<()>,
) -> Result<usize, ApiError>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Result<String, ApiError>>,
{
    if each(PlaylistPart::First(first)).is_break() {
        return Ok(1);
    }
    match rest {
        Some(rest) => follow_rest(rest, fetch, each).await,
        None => Ok(1),
    }
}

/// Asks `fetch` for each page after `rest` by its token and hands its songs to `each`, to
/// the end of the list, `MAX_PAGES`, or until `each` breaks. Returns how many pages were
/// read, the first included.
async fn follow_rest<F, Fut>(
    mut rest: PlaylistRest,
    mut fetch: F,
    each: &mut impl FnMut(PlaylistPart) -> ControlFlow<()>,
) -> Result<usize, ApiError>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Result<String, ApiError>>,
{
    while rest.pages() < MAX_PAGES {
        let page = fetch(rest.token().to_string()).await?;
        let pages = rest.pages() + 1;
        let (tracks, next) = rest.read(&page)?;
        if each(PlaylistPart::More(tracks)).is_break() {
            return Ok(pages);
        }
        match next {
            Some(next) => rest = next,
            None => return Ok(pages),
        }
    }
    Ok(rest.pages())
}
