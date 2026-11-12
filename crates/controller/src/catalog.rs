//! Where pages get their data: the signed-in music API, or the recorded fixtures
//! (`--fixtures`). `Catalog` is shared by the controller (which signs in and sets the
//! source) and the UI thread (which reads it through `Ctx::load`).
//!
//! Every call goes out once; nothing here retries or loops.

use std::ops::ControlFlow;
use std::sync::{Arc, RwLock};

use featherplay_core::model::{AlbumId, ArtistId, LikeStatus, PlaylistId, Track, TrackId};
use featherplay_core::sync::RwLockExt;
use featherplay_ytm::{
    AccountIdentity, AccountInfo, Album, AlbumSummary, ApiError, Artist, ArtistSummary, Charts,
    HistoryDay, HomeFeed, HomeShelf, MoodPage, MoodSection, Playlist, PlaylistPart, PlaylistRest,
    PlaylistSummary, SearchFilter, SearchResults, SearchSuggestions, SongLyrics, YtMusic,
};
use tokio::runtime::Handle;

use crate::fixtures::Fixtures;

/// A snapshot of the data source. Cheap to clone.
#[derive(Clone, Debug)]
pub enum Source {
    /// No session: the catalogue as a visitor (`YtMusic::guest`); what belongs to an
    /// account fails with `ApiError::SignInRequired`.
    SignedOut(YtMusic),
    /// A stored session exists, but connecting with it failed for a network or parse
    /// reason: every call fails with `ApiError::Network`, so pages offer Try again rather
    /// than Sign in. The controller reconnects (`controller/reconnect.rs`).
    Unreachable,
    Live(YtMusic),
    Fixtures(Arc<Fixtures>),
}

impl Source {
    pub fn signed_in(&self) -> bool {
        !matches!(self, Source::SignedOut(_))
    }

    /// The signed-in account's name and photo. Fixtures give a placeholder account.
    pub async fn account_info(&self) -> Result<AccountInfo, ApiError> {
        match self {
            Source::SignedOut(_) => Err(ApiError::SignInRequired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.account_info().await,
            Source::Fixtures(f) => f.account_info(),
        }
    }

    /// Every identity the account switcher lists, the active one marked. Fixtures give
    /// placeholder identities.
    pub async fn accounts(&self) -> Result<Vec<AccountIdentity>, ApiError> {
        match self {
            Source::SignedOut(_) => Err(ApiError::SignInRequired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.accounts().await,
            Source::Fixtures(f) => f.accounts(),
        }
    }

    pub async fn home(&self) -> Result<HomeFeed, ApiError> {
        match self {
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) | Source::SignedOut(api) => api.home().await,
            Source::Fixtures(f) => f.home(),
        }
    }

    /// The Home feed's next page, from `HomeFeed::more`.
    pub async fn home_more(&self, token: &str) -> Result<HomeFeed, ApiError> {
        match self {
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) | Source::SignedOut(api) => api.home_more(token).await,
            Source::Fixtures(f) => f.home_more(token),
        }
    }

    /// Explore's new releases.
    pub async fn new_releases(&self) -> Result<Vec<HomeShelf>, ApiError> {
        match self {
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) | Source::SignedOut(api) => api.new_releases().await,
            Source::Fixtures(f) => f.new_releases(),
        }
    }

    /// Explore's charts for `country`, or the account's country. Fixtures have one country.
    pub async fn charts(&self, country: Option<&str>) -> Result<Charts, ApiError> {
        match self {
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) | Source::SignedOut(api) => api.charts(country).await,
            Source::Fixtures(f) => f.charts(),
        }
    }

    /// Explore's moods and genres.
    pub async fn moods(&self) -> Result<Vec<MoodSection>, ApiError> {
        match self {
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) | Source::SignedOut(api) => api.moods().await,
            Source::Fixtures(f) => f.moods(),
        }
    }

    /// A mood or genre's playlists. Fixtures have one mood's, for any params.
    pub async fn mood_playlists(&self, params: &str) -> Result<MoodPage, ApiError> {
        match self {
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) | Source::SignedOut(api) => api.mood_playlists(params).await,
            Source::Fixtures(f) => f.mood_playlists(),
        }
    }

    /// The listening history, newest day first.
    pub async fn history(&self) -> Result<Vec<HistoryDay>, ApiError> {
        match self {
            Source::SignedOut(_) => Err(ApiError::SignInRequired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.history().await,
            Source::Fixtures(f) => f.history(),
        }
    }

    pub async fn library_playlists(&self) -> Result<Vec<PlaylistSummary>, ApiError> {
        match self {
            Source::SignedOut(_) => Err(ApiError::SignInRequired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.library_playlists().await,
            Source::Fixtures(f) => f.library_playlists(),
        }
    }

    pub async fn liked_songs(&self) -> Result<Vec<Track>, ApiError> {
        match self {
            Source::SignedOut(_) => Err(ApiError::SignInRequired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.liked_songs().await,
            Source::Fixtures(f) => f.liked_songs(),
        }
    }

    pub async fn library_albums(&self) -> Result<Vec<AlbumSummary>, ApiError> {
        match self {
            Source::SignedOut(_) => Err(ApiError::SignInRequired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.library_albums().await,
            Source::Fixtures(f) => f.library_albums(),
        }
    }

    pub async fn library_artists(&self) -> Result<Vec<ArtistSummary>, ApiError> {
        match self {
            Source::SignedOut(_) => Err(ApiError::SignInRequired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.library_artists().await,
            Source::Fixtures(f) => f.library_artists(),
        }
    }

    pub async fn playlist(&self, id: &PlaylistId) -> Result<Playlist, ApiError> {
        match self {
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) | Source::SignedOut(api) => api.playlist(id).await,
            Source::Fixtures(f) => f.playlist(id),
        }
    }

    /// `playlist`'s first page, and where the list goes on, if it does
    /// (`YtMusic::playlist_first`). One request.
    pub async fn playlist_first(
        &self,
        id: &PlaylistId,
    ) -> Result<(Playlist, Option<PlaylistRest>), ApiError> {
        match self {
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) | Source::SignedOut(api) => api.playlist_first(id).await,
            Source::Fixtures(f) => f.playlist_first(id),
        }
    }

    /// The rest of a playlist after `playlist_first`, each page handed to `each` as it's
    /// read (`YtMusic::playlist_rest`). Returns how many pages were read, the first included.
    pub async fn playlist_rest(
        &self,
        rest: PlaylistRest,
        each: impl FnMut(PlaylistPart) -> ControlFlow<()> + Send,
    ) -> Result<usize, ApiError> {
        match self {
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) | Source::SignedOut(api) => api.playlist_rest(rest, each).await,
            Source::Fixtures(f) => f.playlist_rest(rest, each),
        }
    }

    pub async fn album(&self, id: &AlbumId) -> Result<Album, ApiError> {
        match self {
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) | Source::SignedOut(api) => api.album(id).await,
            Source::Fixtures(f) => f.album(id),
        }
    }

    pub async fn artist(&self, id: &ArtistId) -> Result<Artist, ApiError> {
        match self {
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) | Source::SignedOut(api) => api.artist(id).await,
            Source::Fixtures(f) => f.artist(),
        }
    }

    pub async fn search(&self, q: &str, filter: SearchFilter) -> Result<SearchResults, ApiError> {
        match self {
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) | Source::SignedOut(api) => api.search(q, filter).await,
            Source::Fixtures(f) => f.search(filter),
        }
    }

    /// What the top bar suggests while `q` is typed. Fixtures have the one recording (for
    /// "daft pu"), whatever is typed; the demo set has none.
    pub async fn search_suggestions(&self, q: &str) -> Result<SearchSuggestions, ApiError> {
        match self {
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) | Source::SignedOut(api) => api.search_suggestions(q).await,
            Source::Fixtures(f) => f.search_suggestions(),
        }
    }

    pub async fn radio(&self, seed: &TrackId) -> Result<Vec<Track>, ApiError> {
        match self {
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) | Source::SignedOut(api) => api.radio(seed).await,
            Source::Fixtures(f) => f.radio(),
        }
    }

    /// One track by its id (a pasted link), or `None` when it doesn't play. Fixtures know
    /// the tracks of their radio recordings.
    pub async fn track(&self, id: &TrackId) -> Result<Option<Track>, ApiError> {
        match self {
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) | Source::SignedOut(api) => api.track(id).await,
            Source::Fixtures(f) => f.track(id),
        }
    }

    /// The track's own lyrics from its Lyrics tab: timed when YouTube Music has them, else
    /// plain. Fixtures have none.
    pub async fn lyrics(&self, id: &TrackId) -> Result<Option<SongLyrics>, ApiError> {
        match self {
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) | Source::SignedOut(api) => api.lyrics(id).await,
            Source::Fixtures(_) => Ok(None),
        }
    }

    /// A live write. Fixtures accept it and change nothing.
    pub async fn rate(&self, id: &TrackId, status: LikeStatus) -> Result<(), ApiError> {
        match self {
            Source::SignedOut(_) => Err(ApiError::SignInRequired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.rate(id, status).await,
            Source::Fixtures(_) => Ok(()),
        }
    }

    /// The first batch of a playlist's radio. Fixtures give the radio recording.
    pub async fn playlist_radio(&self, id: &PlaylistId) -> Result<Vec<Track>, ApiError> {
        match self {
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) | Source::SignedOut(api) => api.playlist_radio(id).await,
            Source::Fixtures(f) => f.radio(),
        }
    }

    /// A live write. Fixtures accept it and change nothing.
    pub async fn rename_playlist(&self, id: &PlaylistId, title: &str) -> Result<(), ApiError> {
        match self {
            Source::SignedOut(_) => Err(ApiError::SignInRequired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.rename_playlist(id, title).await,
            Source::Fixtures(_) => Ok(()),
        }
    }

    /// A live write that can't be undone. Fixtures accept it and change nothing.
    pub async fn delete_playlist(&self, id: &PlaylistId) -> Result<(), ApiError> {
        match self {
            Source::SignedOut(_) => Err(ApiError::SignInRequired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.delete_playlist(id).await,
            Source::Fixtures(_) => Ok(()),
        }
    }

    /// A live write. Fixtures accept it and change nothing.
    pub async fn remove_playlist_from_library(&self, id: &PlaylistId) -> Result<(), ApiError> {
        match self {
            Source::SignedOut(_) => Err(ApiError::SignInRequired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.remove_playlist_from_library(id).await,
            Source::Fixtures(_) => Ok(()),
        }
    }
}

/// What every call says while the stored session can't connect.
fn unreachable() -> ApiError {
    ApiError::Network("the stored session couldn't connect yet".into())
}

/// The shared source plus the tokio runtime its calls run on.
#[derive(Clone)]
pub struct Catalog {
    source: Arc<RwLock<Source>>,
    /// The visitor client, kept for every sign-out.
    guest: YtMusic,
    rt: Handle,
}

impl Catalog {
    /// Signed out until the controller sets the session. An HTTP client that can't be
    /// built doesn't fail it: the app is offline then (`YtMusic::guest`).
    pub fn new(rt: Handle) -> Result<Catalog, ApiError> {
        let guest = YtMusic::guest()?;
        Ok(Catalog {
            source: Arc::new(RwLock::new(Source::SignedOut(guest.clone()))),
            guest,
            rt,
        })
    }

    /// Back to the visitor client: a sign-out, or a session that ended.
    pub fn sign_out(&self) {
        self.set(Source::SignedOut(self.guest.clone()));
    }

    /// The source is replaced whole, so a panic elsewhere never leaves it half-set: a
    /// poisoned lock still holds the last source, rather than signing the app out.
    pub fn source(&self) -> Source {
        self.source.read_or_recover().clone()
    }

    pub fn set(&self, source: Source) {
        *self.source.write_or_recover() = source;
    }

    pub fn runtime(&self) -> &Handle {
        &self.rt
    }

    pub fn fixtures(&self) -> Option<Arc<Fixtures>> {
        match self.source() {
            Source::Fixtures(f) => Some(f),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn block_on<F: std::future::Future>(f: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime")
            .block_on(f)
    }

    #[test]
    fn an_unreachable_session_fails_as_a_network_error_not_as_signed_out() {
        let source = Source::Unreachable;
        assert!(matches!(block_on(source.home()), Err(ApiError::Network(_))));
        assert!(matches!(
            block_on(source.album(&AlbumId("a".into()))),
            Err(ApiError::Network(_))
        ));
        assert!(source.signed_in());
    }

    fn fixtures() -> Source {
        Source::Fixtures(Arc::new(Fixtures::load().expect("fixtures")))
    }

    #[test]
    fn the_fixture_library_has_one_own_playlist_then_saved_ones() {
        let playlists = block_on(fixtures().library_playlists()).expect("playlists");
        assert!(
            playlists[0].owned,
            "the recorded playlist stands for an own one"
        );
        assert!(playlists.len() > 1 && playlists[1..].iter().all(|p| !p.owned));
    }

    #[test]
    fn playlist_writes_on_fixtures_succeed_and_change_nothing() {
        let source = fixtures();
        let id = PlaylistId("fixture".into());
        let before = block_on(source.library_playlists()).expect("playlists");
        block_on(source.rename_playlist(&id, "New name")).expect("rename");
        block_on(source.delete_playlist(&id)).expect("delete");
        block_on(source.remove_playlist_from_library(&id)).expect("remove");
        assert_eq!(block_on(source.library_playlists()).expect("after"), before);
    }

    #[test]
    fn the_account_signed_out_asks_to_sign_in_rather_than_ending_a_session() {
        let id = PlaylistId("PLx".into());
        let signed_out = Source::SignedOut(YtMusic::guest().expect("guest"));
        assert!(matches!(
            block_on(signed_out.delete_playlist(&id)),
            Err(ApiError::SignInRequired)
        ));
        assert!(matches!(
            block_on(signed_out.library_playlists()),
            Err(ApiError::SignInRequired)
        ));
        assert!(matches!(
            block_on(signed_out.history()),
            Err(ApiError::SignInRequired)
        ));
        assert!(matches!(
            block_on(Source::Unreachable.rename_playlist(&id, "x")),
            Err(ApiError::Network(_))
        ));
    }

    #[test]
    fn a_fixture_playlist_radio_is_the_radio_recording() {
        let tracks = block_on(fixtures().playlist_radio(&PlaylistId("PLx".into()))).expect("radio");
        assert!(!tracks.is_empty());
    }
}
