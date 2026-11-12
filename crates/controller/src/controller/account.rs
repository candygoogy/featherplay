//! Sign-in, sign-out, the Last.fm connection and settings changes from the window.
//!
//! - Sign-in: a cookie header from the sign-in window or the paste field is checked with
//!   one read, stored with `set_secret(YtCookies)` once it works, and the app signs in
//!   without a restart (`self.api`, `Source::Live`, `UiEvent::Account`). When Windows won't
//!   store it, the app signs in all the same and says it isn't saved for the next start
//!   (`UiEvent::SignInNotSaved`).
//! - Sign-out deletes the cookie secret, the sign-in window's browser profile and the
//!   Library's cache (a new sign-in deletes that cache too).
//! - Identities (the account menu): the session acts as one of the identities its cookie
//!   jar holds (`SwitchIdentity`, no request). The pick is saved as `Settings::identity`, a
//!   non-secret key with no names, used at the next start, and checked once then with one
//!   read of the account switcher: one it no longer lists falls back to the session's own.
//!   A new sign-in starts on the session's own identity. Last.fm and ListenBrainz stay.
//! - Last.fm: `get_token`, the approval page in the default browser, then `get_session`
//!   when the user says they approved it; the key goes to `set_secret(LastfmSession)`.
//! - Last.fm API settings: the user's own key and secret go to `set_secret(LastfmApiKey,
//!   LastfmApiSecret)` and replace the build's (`scrobbler::pick_keys`). Saving or removing
//!   them signs out of Last.fm, since a session belongs to the key it was made with.
//! - ListenBrainz: the user token is checked with `validate-token`, then stored with
//!   `set_secret(ListenbrainzToken)`.
//! - `Change`s from the Settings page apply at once where they can and are saved.
//!
//! Cookie headers, API keys, tokens and session keys are never logged.
//!
//! `sign_in.rs` has the sign-in, the sign-out and the identities; `settings.rs` the
//! settings changes.

mod settings;
mod sign_in;

use std::fmt;

use featherplay_core::player::Input;
use featherplay_core::tr;
use featherplay_lastfm::{LastFm, LastFmError};
use featherplay_listenbrainz::{ListenBrainz, ListenBrainzError};
use featherplay_store::{SecretKey, Settings, delete_secret, get_secret, set_secret};
use featherplay_ytm::{ApiError, Identity, YtMusic};

pub use settings::Change;
use sign_in::forget_library;
pub(super) use sign_in::saved_identity;

use super::{Controller, Msg, UiEvent};
use crate::catalog::Source;
use crate::presence::{self, Presence};
use crate::scrobbler::{LastfmKeys, Scrobblers, build_keys, lastfm_keys};

/// A cookie header. Its `Debug` never shows the value.
pub struct Cookies(pub String);

impl fmt::Debug for Cookies {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Cookies({} bytes)", self.0.len())
    }
}

/// A Last.fm API key and secret as entered. Its `Debug` never shows them.
pub struct ApiKeys {
    pub key: String,
    pub secret: String,
}

impl fmt::Debug for ApiKeys {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKeys(..)")
    }
}

/// A ListenBrainz user token. Its `Debug` never shows the value.
pub struct Token(pub String);

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Token({} bytes)", self.0.len())
    }
}

#[derive(Debug)]
pub enum AccountAction {
    /// From the sign-in window or the paste field.
    SignIn(Cookies),
    SignOut,
    /// The account menu: act as this identity of the session from now on.
    SwitchIdentity(Identity),
    /// A change on the Settings page.
    Set(Change),
    /// The Settings page opened: report the Last.fm state.
    LastfmStatus,
    LastfmConnect,
    /// The user approved featherplay on the Last.fm page.
    LastfmApproved,
    LastfmCancel,
    LastfmSignOut,
    /// Save the user's own API key and secret, or remove them (`None`). Signs out of Last.fm.
    LastfmKeys(Option<ApiKeys>),
    /// Check the token and store it.
    ListenbrainzSave(Token),
    ListenbrainzRemove,
    /// "Clear activity" on the Settings page.
    DiscordClear,
    /// "Connect" on the Settings page.
    DiscordConnect,
    /// The Settings page opened: check the Discord connection.
    DiscordCheck,
    /// The Settings page opened: report the local API's state and token.
    LocalApiCheck,
    /// A new local API token; apps using the old one are disconnected.
    LocalApiRegenerate,
}

/// Where the Last.fm connection stands, for the Settings page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LastfmStatus {
    SignedOut,
    /// Asking for a token and opening the browser.
    Opening,
    /// The approval page is open.
    Waiting,
    /// Exchanging the token for a session.
    Finishing,
    SignedIn,
}

/// Where ListenBrainz stands, for the Settings page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListenBrainzStatus {
    /// No token stored.
    SignedOut,
    /// Checking a new token.
    Checking,
    SignedIn,
}

/// The controller's answers, on the UI thread.
#[derive(Clone, Debug)]
pub enum AccountEvent {
    /// The cookie header didn't work; the text says why (never the header).
    SignInFailed(String),
    /// "Session expired, sign in again". `UiEvent::Account { signed_in: false }` follows.
    SessionExpired,
    /// The Last.fm state, with a line to show (an error or a hint) if any.
    Lastfm(LastfmStatus, Option<String>),
    /// Whether the user's own Last.fm API key is stored (else the build's is used).
    LastfmKeys { custom: bool },
    /// The ListenBrainz state, with a line to show if any.
    ListenBrainz(ListenBrainzStatus, Option<String>),
}

/// Results of the account's background work.
pub(super) enum AccountMsg {
    /// The session, and whether it was stored for the next start (`keep_session`).
    SignedIn(Result<(YtMusic, bool), String>),
    /// Whether the account switcher still lists the saved identity the session started as.
    IdentityChecked(Identity, Result<bool, ApiError>),
    /// What Credential Manager holds, for the Settings page.
    Checked {
        lastfm_session: bool,
        custom_keys: bool,
        listenbrainz: bool,
    },
    /// The token and the keys it was asked with.
    LastfmToken(Result<(String, LastfmKeys), String>),
    /// `Ok(custom)`: the user's keys are now stored (`true`) or removed (`false`).
    LastfmKeysSaved(Result<bool, String>),
    /// `Ok(user name)`.
    ListenbrainzSaved(Result<String, String>),
    /// `Err((message, keep_token))`.
    LastfmSession(Result<(), (String, bool)>),
}

/// The account's own state in the controller.
#[derive(Default)]
pub(super) struct AccountState {
    /// The token waiting for the user's approval, and the keys it belongs to.
    lastfm_token: Option<(String, LastfmKeys)>,
    /// A ListenBrainz token is stored.
    listenbrainz: bool,
}

impl Controller {
    pub(super) fn account_action(&mut self, action: AccountAction) {
        match action {
            AccountAction::SignIn(cookies) => self.sign_in(cookies),
            AccountAction::SignOut => self.sign_out(),
            AccountAction::SwitchIdentity(identity) => self.switch_identity(identity),
            AccountAction::Set(change) => self.change(change),
            AccountAction::LastfmStatus => {
                if self.account.lastfm_token.is_some() {
                    self.lastfm_event(LastfmStatus::Waiting, None);
                }
                let tx = self.tx.clone();
                crate::task::blocking_detached(&tokio::runtime::Handle::current(), move || {
                    let checked = AccountMsg::Checked {
                        lastfm_session: get_secret(SecretKey::LastfmSession).is_some(),
                        custom_keys: get_secret(SecretKey::LastfmApiKey).is_some(),
                        listenbrainz: get_secret(SecretKey::ListenbrainzToken).is_some(),
                    };
                    let _ = tx.send(Msg::Account(checked));
                });
            }
            AccountAction::LastfmConnect => self.lastfm_connect(),
            AccountAction::DiscordClear => {
                if let Some(p) = &self.presence {
                    p.clear();
                }
            }
            AccountAction::DiscordConnect => {
                if let Some(p) = &self.presence {
                    p.connect();
                }
            }
            AccountAction::DiscordCheck => match &self.presence {
                Some(p) => p.check(),
                None => (self.ui.events)(UiEvent::Discord(presence::off_status())),
            },
            AccountAction::LocalApiCheck => self.report_local_api(),
            AccountAction::LocalApiRegenerate => self.regenerate_api_token(),
            AccountAction::LastfmApproved => self.lastfm_approved(),
            AccountAction::LastfmCancel => {
                self.account.lastfm_token = None;
                self.lastfm_event(LastfmStatus::SignedOut, None);
            }
            AccountAction::LastfmSignOut => {
                log!("Last.fm: signing out");
                self.account.lastfm_token = None;
                self.settings.lastfm_enabled = false;
                self.save_settings();
                self.restart_scrobbler();
                crate::task::blocking_detached(&tokio::runtime::Handle::current(), || {
                    if let Err(e) = delete_secret(SecretKey::LastfmSession) {
                        log!("Last.fm: deleting the session failed: {e}");
                    }
                });
                self.lastfm_event(LastfmStatus::SignedOut, None);
            }
            AccountAction::LastfmKeys(keys) => self.lastfm_keys_changed(keys),
            AccountAction::ListenbrainzSave(token) => self.listenbrainz_save(token),
            AccountAction::ListenbrainzRemove => {
                log!("ListenBrainz: removing the token");
                self.account.listenbrainz = false;
                self.settings.listenbrainz_enabled = false;
                self.save_settings();
                self.restart_scrobbler();
                crate::task::blocking_detached(&tokio::runtime::Handle::current(), || {
                    if let Err(e) = delete_secret(SecretKey::ListenbrainzToken) {
                        log!("ListenBrainz: deleting the token failed: {e}");
                    }
                });
                self.listenbrainz_event(ListenBrainzStatus::SignedOut, None);
            }
        }
    }

    pub(super) fn account_msg(&mut self, msg: AccountMsg) {
        match msg {
            AccountMsg::SignedIn(Ok((api, kept))) => {
                log!("signed in");
                super::reconnect::keep_version(&self.paths, api.client_version());
                self.unreachable = None;
                self.resumed = false;
                // A new session starts as its own identity, and may be another account:
                // the Library's cached lists go before the window hears of it.
                if self.settings.identity.take().is_some() {
                    self.save_settings();
                }
                forget_library(&self.paths);
                self.api = Some(api.clone());
                self.ui.catalog.set(Source::Live(api));
                // No identity is saved now: the window asks which one when there are several,
                // holding the sign-in until it's chosen (`account_picker`).
                (self.ui.events)(UiEvent::AskIdentity);
                if !kept {
                    (self.ui.events)(UiEvent::SignInNotSaved);
                }
                (self.ui.events)(UiEvent::Account { signed_in: true });
                self.input(Input::SignedIn);
            }
            AccountMsg::IdentityChecked(identity, listed) => {
                let current = self.api.as_ref().map(|a| a.identity() == &identity);
                match listed {
                    _ if current != Some(true) => {}
                    Ok(true) => log!("saved identity {identity:?} is listed"),
                    // Gone from the cookie jar, or refused: the session's own works.
                    Ok(false) | Err(ApiError::AuthExpired) => {
                        log!("saved identity {identity:?} isn't usable; using the session's own");
                        self.switch_identity(Identity::default());
                    }
                    Err(e) => log!("checking the saved identity: {e}"),
                }
            }
            AccountMsg::SignedIn(Err(why)) => {
                log!("sign-in failed: {why}");
                (self.ui.events)(UiEvent::AccountNotice(AccountEvent::SignInFailed(why)));
            }
            AccountMsg::Checked {
                lastfm_session,
                custom_keys,
                listenbrainz,
            } => {
                if self.account.lastfm_token.is_none() {
                    let status = if lastfm_session {
                        LastfmStatus::SignedIn
                    } else {
                        LastfmStatus::SignedOut
                    };
                    self.lastfm_event(status, None);
                }
                (self.ui.events)(UiEvent::AccountNotice(AccountEvent::LastfmKeys {
                    custom: custom_keys,
                }));
                self.account.listenbrainz = listenbrainz;
                let lb = if listenbrainz {
                    ListenBrainzStatus::SignedIn
                } else {
                    ListenBrainzStatus::SignedOut
                };
                self.listenbrainz_event(lb, None);
            }
            AccountMsg::LastfmKeysSaved(Ok(custom)) => {
                log!(
                    "Last.fm: API key {}; signed out",
                    if custom { "saved" } else { "removed" }
                );
                self.restart_scrobbler();
                (self.ui.events)(UiEvent::AccountNotice(AccountEvent::LastfmKeys { custom }));
                let hint = match (custom, build_keys().is_some()) {
                    (true, _) => {
                        tr!("Your API key is saved. Connect to sign in to Last.fm with it.")
                    }
                    (false, true) => tr!("Using featherplay's API key again. Connect to sign in."),
                    (false, false) => tr!("Your API key is removed. Last.fm needs one to connect."),
                };
                self.lastfm_event(LastfmStatus::SignedOut, Some(hint.into()));
            }
            AccountMsg::LastfmKeysSaved(Err(why)) => {
                log!("Last.fm: saving the API key failed: {why}");
                self.lastfm_event(
                    LastfmStatus::SignedOut,
                    Some(tr!("Saving the API key failed: {}", why)),
                );
            }
            AccountMsg::ListenbrainzSaved(Ok(user)) => {
                log!("ListenBrainz: token saved");
                self.account.listenbrainz = true;
                self.settings.listenbrainz_enabled = true;
                self.save_settings();
                self.restart_scrobbler();
                self.listenbrainz_event(
                    ListenBrainzStatus::SignedIn,
                    Some(tr!(
                        "Connected as {}. Listens go to your ListenBrainz profile.",
                        user
                    )),
                );
            }
            AccountMsg::ListenbrainzSaved(Err(why)) => {
                log!("ListenBrainz: token not saved: {why}");
                let status = if self.account.listenbrainz {
                    ListenBrainzStatus::SignedIn
                } else {
                    ListenBrainzStatus::SignedOut
                };
                self.listenbrainz_event(status, Some(why));
            }
            AccountMsg::LastfmToken(Ok(token)) => {
                log!("Last.fm: approval page opened");
                self.account.lastfm_token = Some(token);
                self.lastfm_event(LastfmStatus::Waiting, None);
            }
            AccountMsg::LastfmToken(Err(why)) => {
                log!("Last.fm: no token: {why}");
                self.lastfm_event(LastfmStatus::SignedOut, Some(why));
            }
            AccountMsg::LastfmSession(Ok(())) => {
                log!("Last.fm: signed in");
                self.account.lastfm_token = None;
                self.settings.lastfm_enabled = true;
                self.save_settings();
                self.restart_scrobbler();
                self.lastfm_event(LastfmStatus::SignedIn, None);
            }
            AccountMsg::LastfmSession(Err((why, keep))) => {
                log!("Last.fm: no session: {why}");
                if keep {
                    self.lastfm_event(LastfmStatus::Waiting, Some(why));
                } else {
                    self.account.lastfm_token = None;
                    self.lastfm_event(LastfmStatus::SignedOut, Some(why));
                }
            }
        }
    }

    fn save_settings(&self) {
        if let Err(e) = self.settings.save(&self.paths) {
            log!("saving the settings failed: {}", e.kind());
        }
    }

    /// New scrobblers for the current settings, keys, session and token (or none).
    fn restart_scrobbler(&mut self) {
        let tx = self.tx.clone();
        let mut scrobblers = Scrobblers::new(
            &self.settings,
            &self.paths,
            std::sync::Arc::new(move |sent| {
                let _ = tx.send(Msg::Scrobbled(sent));
            }),
        );
        scrobblers.carry_over(&mut self.scrobblers);
        self.scrobblers = scrobblers;
        self.scrobblers.flush();
    }

    fn lastfm_connect(&mut self) {
        log!("Last.fm: asking for a token");
        self.lastfm_event(LastfmStatus::Opening, None);
        let tx = self.tx.clone();
        crate::task::spawn_detached(async move {
            let keys = tokio::task::spawn_blocking(lastfm_keys)
                .await
                .ok()
                .flatten();
            let Some(keys) = keys else {
                log!("Last.fm: no API key");
                let why = "Last.fm needs an API key, and this build has none. Create one at \
                    last.fm/api/account/create, then enter it under API settings.";
                let _ = tx.send(Msg::Account(AccountMsg::LastfmToken(Err(why.into()))));
                return;
            };
            let asked = async {
                let client = LastFm::new(&keys.key, &keys.secret, None)?;
                let token = client.get_token().await?;
                Ok::<_, LastFmError>((client.auth_url(&token), token))
            }
            .await;
            let result = match asked {
                Ok((url, token)) => {
                    let opened =
                        tokio::task::spawn_blocking(move || featherplay_platform::open::url(&url))
                            .await
                            .unwrap_or_else(|e| Err(e.to_string()));
                    match opened {
                        Ok(()) => Ok((token, keys)),
                        Err(e) => Err(tr!("The browser didn't open: {}", e)),
                    }
                }
                Err(e) => Err(lastfm_error(&e)),
            };
            let _ = tx.send(Msg::Account(AccountMsg::LastfmToken(result)));
        });
    }

    fn lastfm_approved(&mut self) {
        let Some((token, keys)) = self.account.lastfm_token.clone() else {
            self.lastfm_event(LastfmStatus::SignedOut, None);
            return;
        };
        log!("Last.fm: asking for the session");
        self.lastfm_event(LastfmStatus::Finishing, None);
        let tx = self.tx.clone();
        crate::task::spawn_detached(async move {
            let session = async {
                let client = LastFm::new(&keys.key, &keys.secret, None)?;
                client.get_session(&token).await
            }
            .await;
            let result = match session {
                Ok(session) => tokio::task::spawn_blocking(move || {
                    set_secret(SecretKey::LastfmSession, &session)
                        .map_err(|e| (tr!("Saving the session failed: {}", e), false))
                })
                .await
                .unwrap_or_else(|e| Err((e.to_string(), false))),
                // Not approved yet: the token stays valid for an hour, so try again.
                Err(LastFmError::Api { code: 14, .. }) => Err((
                    tr!("Last.fm hasn't been approved yet. Approve featherplay in the browser, then try again.")
                        .into(),
                    true,
                )),
                Err(LastFmError::Api { code: 15, .. }) => Err((
                    tr!("That approval expired. Connect again.").into(),
                    false,
                )),
                Err(e @ LastFmError::Http(_)) => Err((lastfm_error(&e), true)),
                Err(e) => Err((lastfm_error(&e), false)),
            };
            let _ = tx.send(Msg::Account(AccountMsg::LastfmSession(result)));
        });
    }

    fn lastfm_event(&self, status: LastfmStatus, message: Option<String>) {
        (self.ui.events)(UiEvent::AccountNotice(AccountEvent::Lastfm(
            status, message,
        )));
    }

    /// Stores or removes the user's API key, then signs out of Last.fm: the session was
    /// made with the old key.
    fn lastfm_keys_changed(&mut self, keys: Option<ApiKeys>) {
        log!(
            "Last.fm: {} the user's API key",
            if keys.is_some() { "saving" } else { "removing" }
        );
        self.account.lastfm_token = None;
        self.settings.lastfm_enabled = false;
        self.save_settings();
        self.restart_scrobbler();
        let tx = self.tx.clone();
        crate::task::blocking_detached(&tokio::runtime::Handle::current(), move || {
            let result = store_keys(keys.as_ref())
                .and_then(|()| delete_secret(SecretKey::LastfmSession))
                .map(|()| keys.is_some())
                .map_err(|e| e.to_string());
            let _ = tx.send(Msg::Account(AccountMsg::LastfmKeysSaved(result)));
        });
    }

    fn listenbrainz_save(&mut self, token: Token) {
        log!("ListenBrainz: checking a token ({} bytes)", token.0.len());
        self.listenbrainz_event(ListenBrainzStatus::Checking, None);
        let tx = self.tx.clone();
        crate::task::spawn_detached(async move {
            let token = token.0.trim().to_string();
            let checked = async { ListenBrainz::new(&token)?.validate_token().await }.await;
            let result = match checked {
                Ok(user) => tokio::task::spawn_blocking(move || {
                    set_secret(SecretKey::ListenbrainzToken, &token)
                        .map(|()| user)
                        .map_err(|e| tr!("Saving the token failed: {}", e))
                })
                .await
                .unwrap_or_else(|e| Err(e.to_string())),
                Err(e) => Err(listenbrainz_error(&e)),
            };
            let _ = tx.send(Msg::Account(AccountMsg::ListenbrainzSaved(result)));
        });
    }

    fn listenbrainz_event(&self, status: ListenBrainzStatus, message: Option<String>) {
        (self.ui.events)(UiEvent::AccountNotice(AccountEvent::ListenBrainz(
            status, message,
        )));
    }
}

/// Stores both halves of the user's API key, or deletes both.
fn store_keys(keys: Option<&ApiKeys>) -> Result<(), featherplay_store::StoreError> {
    match keys {
        Some(k) => {
            set_secret(SecretKey::LastfmApiKey, k.key.trim())?;
            set_secret(SecretKey::LastfmApiSecret, k.secret.trim())
        }
        None => {
            delete_secret(SecretKey::LastfmApiKey)?;
            delete_secret(SecretKey::LastfmApiSecret)
        }
    }
}

fn listenbrainz_error(e: &ListenBrainzError) -> String {
    match e {
        ListenBrainzError::InvalidToken => tr!(
            "ListenBrainz didn't accept that token. Copy it again from listenbrainz.org/settings."
        )
        .into(),
        ListenBrainzError::Http(_) => {
            tr!("Couldn't reach ListenBrainz. Check the connection and try again.").into()
        }
        other => other.to_string(),
    }
}

/// A Discord worker for `settings`, reporting its connection to the controller; `None`
/// when Discord is off or this build has no client ID.
pub(super) fn start_presence(
    settings: &Settings,
    tx: &tokio::sync::mpsc::UnboundedSender<Msg>,
) -> Option<Presence> {
    let tx = tx.clone();
    Presence::new(settings, move |status| {
        let _ = tx.send(Msg::Discord(status));
    })
}

fn lastfm_error(e: &LastFmError) -> String {
    match e {
        LastFmError::Http(_) => {
            tr!("Couldn't reach Last.fm. Check the connection and try again.").into()
        }
        LastFmError::Api { code, message } => tr!("Last.fm said: {0} ({1})", message, code),
        other => other.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookie_debug_hides_the_value() {
        let c = Cookies("SAPISID=secret-value".into());
        assert!(!format!("{c:?}").contains("secret"));
        assert!(!format!("{:?}", AccountAction::SignIn(c)).contains("secret"));
        let keys = ApiKeys {
            key: "key-value".into(),
            secret: "secret-value".into(),
        };
        let shown = format!("{:?}", AccountAction::LastfmKeys(Some(keys)));
        assert!(!shown.contains("value"));
        let token = Token("token-value".into());
        assert!(!format!("{:?}", AccountAction::ListenbrainzSave(token)).contains("value"));
    }
}
