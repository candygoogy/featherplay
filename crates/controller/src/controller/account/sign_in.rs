//! Signing in with a cookie header, signing out, a session that expired, and the
//! identities a session acts as (see `account.rs`).

use std::fmt;

use featherplay_core::tr;

use featherplay_store::{SecretKey, Settings, delete_secret, set_secret};
use featherplay_ytm::{ApiError, Identity, YtMusic};

use super::{AccountEvent, AccountMsg, Cookies};
use crate::catalog::Source;
use crate::controller::{Controller, Msg, UiEvent};

impl Controller {
    /// "Session expired, sign in again": the music keeps playing; the window asks to sign
    /// in. The cookie secret stays until a new sign-in replaces it.
    pub(crate) fn session_expired(&mut self) {
        log!("session expired, sign in again");
        self.api = None;
        self.unreachable = None;
        self.resumed = false;
        self.ui.catalog.sign_out();
        // Before the window hears of it, so it can't show the feed again.
        forget_home(&self.paths);
        (self.ui.events)(UiEvent::AccountNotice(AccountEvent::SessionExpired));
        (self.ui.events)(UiEvent::Account { signed_in: false });
    }

    pub(super) fn sign_in(&mut self, cookies: Cookies) {
        log!("signing in ({} byte cookie header)", cookies.0.len());
        let tx = self.tx.clone();
        crate::task::spawn_detached(async move {
            let result = check_session(cookies).await;
            let _ = tx.send(Msg::Account(AccountMsg::SignedIn(result)));
        });
    }

    /// Acts as `identity` from now on: the catalog, likes and playlist edits. The music
    /// keeps playing. Saved for the next start.
    pub(super) fn switch_identity(&mut self, identity: Identity) {
        let Some(api) = &self.api else {
            log!("switching identities: signed out");
            return;
        };
        let next = match api.as_identity(&identity) {
            Ok(next) => next,
            Err(e) => {
                log!("switching identities failed: {e}");
                // Still the same identity; the first sign-in's picker waits for an answer.
                (self.ui.events)(UiEvent::Identity(api.identity().clone()));
                return;
            }
        };
        log!("acting as {identity:?}");
        self.api = Some(next.clone());
        self.ui.catalog.set(Source::Live(next));
        let key = (!identity.is_default()).then(|| identity.key());
        if self.settings.identity != key {
            self.settings.identity = key;
            self.save_settings();
        }
        // Likes changed this session were the other identity's.
        self.likes.clear();
        self.push_state();
        (self.ui.events)(UiEvent::Identity(identity));
    }

    /// At startup with a saved identity: one read of the account switcher checks that the
    /// cookie jar still has it (`AccountMsg::IdentityChecked`).
    pub(crate) fn check_identity(&self) {
        let Some(api) = self.api.clone() else { return };
        if api.identity().is_default() {
            return;
        }
        let tx = self.tx.clone();
        crate::task::spawn_detached(async move {
            let identity = api.identity().clone();
            let listed = api
                .accounts()
                .await
                .map(|list| list.iter().any(|a| a.identity == identity));
            let _ = tx.send(Msg::Account(AccountMsg::IdentityChecked(identity, listed)));
        });
    }

    pub(super) fn sign_out(&mut self) {
        log!("signing out");
        self.api = None;
        self.unreachable = None;
        self.resumed = false;
        if self.settings.identity.take().is_some() {
            self.save_settings();
        }
        self.ui.catalog.sign_out();
        forget_library(&self.paths);
        self.forget_saved_streams();
        // The sign-in window's profile too, so the next sign-in can pick another account.
        let profile = self.paths.cache.join("webview2");
        crate::task::blocking_detached(&tokio::runtime::Handle::current(), move || {
            if let Err(e) = delete_secret(SecretKey::YtCookies) {
                log!("deleting the session failed: {e}");
            }
            remove_profile(&profile);
        });
        (self.ui.events)(UiEvent::Account { signed_in: false });
    }
}

/// Deletes the sign-in window's WebView2 profile, which holds Google's own sign-in
/// cookies. Its msedgewebview2.exe processes can keep files open for a moment after the
/// window closes, so a failed delete is tried again for up to ~5 s. Blocks.
fn remove_profile(profile: &std::path::Path) {
    const TRIES: u32 = 10;
    for attempt in 1..=TRIES {
        match std::fs::remove_dir_all(profile) {
            Ok(()) => {
                log!("sign-in profile removed");
                return;
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
            Err(e) if attempt == TRIES => {
                log!("removing the sign-in profile failed: {}", e.kind());
            }
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(500)),
        }
    }
}

/// Deletes the Library's cached lists (every identity's). Blocks briefly: a few small
/// files, removed before the window is told about the new account, so it can't read the
/// old one's lists.
pub(super) fn forget_library(paths: &featherplay_store::Paths) {
    match featherplay_store::LibraryCache::new(paths).clear() {
        Ok(()) => log!("library cache removed"),
        Err(e) => log!("removing the library cache failed: {}", e.kind()),
    }
}

/// Deletes Home's cached feed (every identity's), keeping the Library's lists: a session
/// that ended. Blocks briefly, as `forget_library`.
fn forget_home(paths: &featherplay_store::Paths) {
    let cache = featherplay_store::LibraryCache::new(paths);
    match cache.forget(featherplay_store::LibraryList::Home) {
        Ok(()) => log!("home feed cache removed"),
        Err(e) => log!("removing the home feed cache failed: {}", e.kind()),
    }
}

/// The identity saved by the account menu, or the session's own when there is none (or
/// the file was edited into something that isn't one).
pub(crate) fn saved_identity(settings: &Settings) -> Identity {
    settings
        .identity
        .as_deref()
        .and_then(Identity::from_key)
        .unwrap_or_default()
}

/// Signs in with `cookies` and checks the session with one read; stores the header once
/// it works, and says whether that worked. The error text never contains the header.
async fn check_session(cookies: Cookies) -> Result<(YtMusic, bool), String> {
    let api = YtMusic::from_cookie_header(&cookies.0)
        .await
        .map_err(|e| sign_in_error(&e))?;
    api.library_playlists()
        .await
        .map_err(|e| sign_in_error(&e))?;
    let kept = keep_session(cookies, |header| set_secret(SecretKey::YtCookies, header)).await;
    Ok((api, kept))
}

/// Stores a session's cookie header for the next start with `store` (Credential Manager),
/// off the async threads. `false` when it refuses: the session works for this run, and the
/// next start asks again. Why goes to the log and Copy diagnostics: the store's error with
/// Windows' code, never the header.
async fn keep_session<E: fmt::Display + 'static>(
    cookies: Cookies,
    store: impl FnOnce(&str) -> Result<(), E> + Send + 'static,
) -> bool {
    let refused = tokio::task::spawn_blocking(move || store(&cookies.0).map_err(|e| e.to_string()))
        .await
        .unwrap_or_else(|e| Err(e.to_string()))
        .err();
    let kept = refused.is_none();
    crate::diagnostics::sign_in_kept(refused);
    kept
}

/// What the sign-in page says when a session doesn't work. The details go to the log.
pub fn sign_in_error(e: &ApiError) -> String {
    log!("sign-in: {e}");
    match e {
        ApiError::AuthExpired | ApiError::SignInRequired => {
            tr!("That session isn't signed in. Sign in again, or copy a fresh cookie header.")
                .into()
        }
        ApiError::Network(_) => {
            tr!("Couldn't reach YouTube Music. Check the connection, then try again.").into()
        }
        ApiError::Parse(_) | ApiError::NotFound => {
            tr!("YouTube Music answered in a way featherplay doesn't understand. Try again later.")
                .into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_network_failure_reads_plainly_on_the_sign_in_page() {
        let raw = "error sending request for url (https://music.youtube.com/)";
        let text = sign_in_error(&ApiError::Network(raw.into()));
        assert!(!text.contains("error sending request"), "{text}");
        assert!(!text.contains("https://"), "{text}");
        assert!(text.contains("try again"), "{text}");
    }

    #[tokio::test]
    async fn a_session_the_store_refuses_isnt_kept_and_copy_diagnostics_says_why() {
        let header = "SAPISID=secret-value";
        let refusal = "credential store: Platform failure: Windows error code 1783";
        let kept = keep_session(Cookies(header.into()), move |_| Err(refusal)).await;
        assert!(!kept, "a refused session counted as kept");
        let report = crate::diagnostics::report(true, &Settings::default());
        assert_eq!(report.sign_in_save.as_deref(), Some(refusal));

        // The next sign-in is stored, header and all: nothing is left to report.
        let kept = keep_session(Cookies(header.into()), move |stored| {
            if stored == header {
                Ok(())
            } else {
                Err("not the header")
            }
        })
        .await;
        assert!(kept, "a stored session counted as refused");
        let report = crate::diagnostics::report(true, &Settings::default());
        assert_eq!(report.sign_in_save, None);
    }

    #[test]
    fn the_saved_identity_is_read_back_or_ignored() {
        let mut s = Settings::default();
        assert!(saved_identity(&s).is_default());
        s.identity = Some("0/100000000000000000002".into());
        let id = saved_identity(&s);
        assert_eq!(id.auth_user(), Some(0));
        assert_eq!(id.page_id(), Some("100000000000000000002"));
        s.identity = Some("not an identity".into());
        assert!(saved_identity(&s).is_default());
    }

    #[test]
    fn sign_out_removes_the_whole_profile() {
        let home = std::env::var_os("FEATHERPLAY_HOME").expect("set FEATHERPLAY_HOME");
        let profile = std::path::PathBuf::from(home)
            .join(format!("test-profile-{}", std::process::id()))
            .join("webview2");
        let cookies = profile.join("EBWebView/Default/Network");
        std::fs::create_dir_all(&cookies).unwrap();
        std::fs::write(cookies.join("Cookies"), b"x").unwrap();
        remove_profile(&profile);
        assert!(!profile.exists());
        // Already gone: nothing to do, no error.
        remove_profile(&profile);
        std::fs::remove_dir(profile.parent().unwrap()).unwrap();
    }

    #[test]
    fn ending_a_session_forgets_the_library_cache() {
        use featherplay_store::{LibraryCache, LibraryList, Paths};
        let home = std::env::var_os("FEATHERPLAY_HOME").expect("set FEATHERPLAY_HOME");
        let dir =
            std::path::PathBuf::from(home).join(format!("test-forget-{}", std::process::id()));
        let paths = Paths::from_home(&dir);
        let cache = LibraryCache::new(&paths);
        cache
            .save("", LibraryList::Liked, &vec!["a".to_string()])
            .unwrap();
        cache
            .save("2", LibraryList::Albums, &vec!["b".to_string()])
            .unwrap();
        // Sign-out's and a new sign-in's: Home's feed goes with the lists.
        cache
            .save("", LibraryList::Home, &vec!["feed".to_string()])
            .unwrap();
        forget_library(&paths);
        assert_eq!(cache.load::<Vec<String>>("", LibraryList::Liked), None);
        assert_eq!(cache.load::<Vec<String>>("2", LibraryList::Albums), None);
        assert_eq!(cache.load::<Vec<String>>("", LibraryList::Home), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_expired_session_forgets_the_home_feed() {
        use featherplay_store::{LibraryCache, LibraryList, Paths};
        let home = std::env::var_os("FEATHERPLAY_HOME").expect("set FEATHERPLAY_HOME");
        let dir =
            std::path::PathBuf::from(home).join(format!("test-expired-{}", std::process::id()));
        let paths = Paths::from_home(&dir);
        let cache = LibraryCache::new(&paths);
        for identity in ["", "2"] {
            cache
                .save(identity, LibraryList::Home, &vec!["feed".to_string()])
                .unwrap();
        }
        cache
            .save("", LibraryList::Liked, &vec!["a".to_string()])
            .unwrap();
        forget_home(&paths);
        assert_eq!(cache.load::<Vec<String>>("", LibraryList::Home), None);
        assert_eq!(cache.load::<Vec<String>>("2", LibraryList::Home), None);
        // The Library's lists stay until a new sign-in, as before.
        assert!(cache.load::<Vec<String>>("", LibraryList::Liked).is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
