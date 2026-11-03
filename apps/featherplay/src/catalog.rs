//! Where pages get their data: the signed-in music API, or the recorded fixtures
//! (`--fixtures`). `Catalog` is shared by the controller (which signs in and sets the
//! source) and the UI thread (which reads it through `Ctx::load`).
//!
//! Every call goes out once; nothing here retries or loops.

use std::sync::{Arc, RwLock};

use featherplay_core::model::{AlbumId, ArtistId, LikeStatus, PlaylistId, Track, TrackId};
use featherplay_core::sync::RwLockExt;
use featherplay_ytm::{
    AccountIdentity, AccountInfo, Album, AlbumSummary, ApiError, Artist, ArtistSummary, Charts,
    HistoryDay, HomeFeed, HomeShelf, MoodPage, MoodSection, Playlist, PlaylistSummary,
    SearchFilter, SearchResults, SearchSuggestions, SongLyrics, YtMusic,
};
use tokio::runtime::Handle;

use crate::fixtures::Fixtures;

/// A snapshot of the data source. Cheap to clone.
#[derive(Clone, Debug)]
pub enum Source {
    /// No session: every call fails with `ApiError::AuthExpired`.
    SignedOut,
    /// A stored session exists, but connecting with it failed for a network or parse
    /// reason: every call fails with `ApiError::Network`, so pages offer Try again rather
    /// than Sign in. The controller reconnects (`controller/reconnect.rs`).
    Unreachable,
    Live(YtMusic),
    Fixtures(Arc<Fixtures>),
}

impl Source {
    pub fn signed_in(&self) -> bool {
        !matches!(self, Source::SignedOut)
    }

    /// The signed-in account's name and photo. Fixtures give a placeholder account.
    pub async fn account_info(&self) -> Result<AccountInfo, ApiError> {
        match self {
            Source::SignedOut => Err(ApiError::AuthExpired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.account_info().await,
            Source::Fixtures(f) => f.account_info(),
        }
    }

    /// Every identity the account switcher lists, the active one marked. Fixtures give
    /// placeholder identities.
    pub async fn accounts(&self) -> Result<Vec<AccountIdentity>, ApiError> {
        match self {
            Source::SignedOut => Err(ApiError::AuthExpired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.accounts().await,
            Source::Fixtures(f) => f.accounts(),
        }
    }

    pub async fn home(&self) -> Result<HomeFeed, ApiError> {
        match self {
            Source::SignedOut => Err(ApiError::AuthExpired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.home().await,
            Source::Fixtures(f) => f.home(),
        }
    }

    /// The Home feed's next page, from `HomeFeed::more`.
    pub async fn home_more(&self, token: &str) -> Result<HomeFeed, ApiError> {
        match self {
            Source::SignedOut => Err(ApiError::AuthExpired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.home_more(token).await,
            Source::Fixtures(f) => f.home_more(token),
        }
    }

    /// Explore's new releases.
    pub async fn new_releases(&self) -> Result<Vec<HomeShelf>, ApiError> {
        match self {
            Source::SignedOut => Err(ApiError::AuthExpired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.new_releases().await,
            Source::Fixtures(f) => f.new_releases(),
        }
    }

    /// Explore's charts for `country`, or the account's country. Fixtures have one country.
    pub async fn charts(&self, country: Option<&str>) -> Result<Charts, ApiError> {
        match self {
            Source::SignedOut => Err(ApiError::AuthExpired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.charts(country).await,
            Source::Fixtures(f) => f.charts(),
        }
    }

    /// Explore's moods and genres.
    pub async fn moods(&self) -> Result<Vec<MoodSection>, ApiError> {
        match self {
            Source::SignedOut => Err(ApiError::AuthExpired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.moods().await,
            Source::Fixtures(f) => f.moods(),
        }
    }

    /// A mood or genre's playlists. Fixtures have one mood's, for any params.
    pub async fn mood_playlists(&self, params: &str) -> Result<MoodPage, ApiError> {
        match self {
            Source::SignedOut => Err(ApiError::AuthExpired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.mood_playlists(params).await,
            Source::Fixtures(f) => f.mood_playlists(),
        }
    }

    /// The listening history, newest day first.
    pub async fn history(&self) -> Result<Vec<HistoryDay>, ApiError> {
        match self {
            Source::SignedOut => Err(ApiError::AuthExpired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.history().await,
            Source::Fixtures(f) => f.history(),
        }
    }

    pub async fn library_playlists(&self) -> Result<Vec<PlaylistSummary>, ApiError> {
        match self {
            Source::SignedOut => Err(ApiError::AuthExpired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.library_playlists().await,
            Source::Fixtures(f) => f.library_playlists(),
        }
    }

    pub async fn liked_songs(&self) -> Result<Vec<Track>, ApiError> {
        match self {
            Source::SignedOut => Err(ApiError::AuthExpired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.liked_songs().await,
            Source::Fixtures(f) => f.liked_songs(),
        }
    }

    pub async fn library_albums(&self) -> Result<Vec<AlbumSummary>, ApiError> {
        match self {
            Source::SignedOut => Err(ApiError::AuthExpired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.library_albums().await,
            Source::Fixtures(f) => f.library_albums(),
        }
    }

    pub async fn library_artists(&self) -> Result<Vec<ArtistSummary>, ApiError> {
        match self {
            Source::SignedOut => Err(ApiError::AuthExpired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.library_artists().await,
            Source::Fixtures(f) => f.library_artists(),
        }
    }

    pub async fn playlist(&self, id: &PlaylistId) -> Result<Playlist, ApiError> {
        match self {
            Source::SignedOut => Err(ApiError::AuthExpired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.playlist(id).await,
            Source::Fixtures(f) => f.playlist(id),
        }
    }

    pub async fn album(&self, id: &AlbumId) -> Result<Album, ApiError> {
        match self {
            Source::SignedOut => Err(ApiError::AuthExpired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.album(id).await,
            Source::Fixtures(f) => f.album(id),
        }
    }

    pub async fn artist(&self, id: &ArtistId) -> Result<Artist, ApiError> {
        match self {
            Source::SignedOut => Err(ApiError::AuthExpired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.artist(id).await,
            Source::Fixtures(f) => f.artist(),
        }
    }

    pub async fn search(&self, q: &str, filter: SearchFilter) -> Result<SearchResults, ApiError> {
        match self {
            Source::SignedOut => Err(ApiError::AuthExpired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.search(q, filter).await,
            Source::Fixtures(f) => f.search(filter),
        }
    }

    /// What the top bar suggests while `q` is typed. Fixtures have the one recording (for
    /// "daft pu"), whatever is typed; the demo set has none.
    pub async fn search_suggestions(&self, q: &str) -> Result<SearchSuggestions, ApiError> {
        match self {
            Source::SignedOut => Err(ApiError::AuthExpired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.search_suggestions(q).await,
            Source::Fixtures(f) => f.search_suggestions(),
        }
    }

    pub async fn radio(&self, seed: &TrackId) -> Result<Vec<Track>, ApiError> {
        match self {
            Source::SignedOut => Err(ApiError::AuthExpired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.radio(seed).await,
            Source::Fixtures(f) => f.radio(),
        }
    }

    /// One track by its id (a pasted link), or `None` when it doesn't play. Fixtures know
    /// the tracks of their radio recordings.
    pub async fn track(&self, id: &TrackId) -> Result<Option<Track>, ApiError> {
        match self {
            Source::SignedOut => Err(ApiError::AuthExpired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.track(id).await,
            Source::Fixtures(f) => f.track(id),
        }
    }

    /// The track's own lyrics from its Lyrics tab: timed when YouTube Music has them, else
    /// plain. Fixtures have none.
    pub async fn lyrics(&self, id: &TrackId) -> Result<Option<SongLyrics>, ApiError> {
        match self {
            Source::SignedOut => Err(ApiError::AuthExpired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.lyrics(id).await,
            Source::Fixtures(_) => Ok(None),
        }
    }

    /// A live write. Fixtures accept it and change nothing.
    pub async fn rate(&self, id: &TrackId, status: LikeStatus) -> Result<(), ApiError> {
        match self {
            Source::SignedOut => Err(ApiError::AuthExpired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.rate(id, status).await,
            Source::Fixtures(_) => Ok(()),
        }
    }

    /// The first batch of a playlist's radio. Fixtures give the radio recording.
    pub async fn playlist_radio(&self, id: &PlaylistId) -> Result<Vec<Track>, ApiError> {
        match self {
            Source::SignedOut => Err(ApiError::AuthExpired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.playlist_radio(id).await,
            Source::Fixtures(f) => f.radio(),
        }
    }

    /// A live write. Fixtures accept it and change nothing.
    pub async fn rename_playlist(&self, id: &PlaylistId, title: &str) -> Result<(), ApiError> {
        match self {
            Source::SignedOut => Err(ApiError::AuthExpired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.rename_playlist(id, title).await,
            Source::Fixtures(_) => Ok(()),
        }
    }

    /// A live write that can't be undone. Fixtures accept it and change nothing.
    pub async fn delete_playlist(&self, id: &PlaylistId) -> Result<(), ApiError> {
        match self {
            Source::SignedOut => Err(ApiError::AuthExpired),
            Source::Unreachable => Err(unreachable()),
            Source::Live(api) => api.delete_playlist(id).await,
            Source::Fixtures(_) => Ok(()),
        }
    }

    /// A live write. Fixtures accept it and change nothing.
    pub async fn remove_playlist_from_library(&self, id: &PlaylistId) -> Result<(), ApiError> {
        match self {
            Source::SignedOut => Err(ApiError::AuthExpired),
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
    rt: Handle,
}

impl Catalog {
    pub fn new(rt: Handle, source: Source) -> Catalog {
        Catalog {
            source: Arc::new(RwLock::new(source)),
            rt,
        }
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
    fn playlist_writes_signed_out_are_auth_expired() {
        let id = PlaylistId("PLx".into());
        assert!(matches!(
            block_on(Source::SignedOut.delete_playlist(&id)),
            Err(ApiError::AuthExpired)
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
