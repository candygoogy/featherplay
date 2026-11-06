//! Sign-in, sign-out, the Last.fm connection and settings changes from the window.
//!
//! - Sign-in: a cookie header from the sign-in window or the paste field is checked with
//!   one read, stored with `set_secret(YtCookies)` once it works, and the app signs in
//!   without a restart (`self.api`, `Source::Live`, `UiEvent::Account`).
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

use std::fmt;

use featherplay_core::player::Input;
use featherplay_core::tr;
use featherplay_lastfm::{LastFm, LastFmError};
use featherplay_listenbrainz::{ListenBrainz, ListenBrainzError};
use featherplay_store::{
    Accent, AccentColour, CloseAction, DiscordOptions, Hotkeys, SecretKey, Settings,
    StreamingQuality, ThemeMode, delete_secret, get_secret, set_secret,
};
use featherplay_ytm::{ApiError, Identity, YtMusic};

use super::{Controller, Msg, UiEvent, prefs, stream_quality};
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

/// One settings change. `apply` is shared by the window's copy and the controller's.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    PreferSongVersions(bool),
    SkipNonMusicInRadio(bool),
    /// Moving on passes over songs the user disliked.
    SkipDisliked(bool),
    Sponsorblock(bool),
    /// `None` is the system default.
    OutputDevice(Option<String>),
    /// Scrobbling as a whole, for every service.
    Scrobbling(bool),
    /// Last.fm scrobbling on or off; the Last.fm session is kept either way.
    Scrobble(bool),
    /// ListenBrainz on or off; the token is kept either way.
    ListenBrainz(bool),
    ScrobbleOtherMedia(bool),
    AlternativeTitles(bool),
    AlternativeArtists(bool),
    Discord(bool),
    DiscordOptions(DiscordOptions),
    Notifications(bool),
    CloseAction(CloseAction),
    /// The one-time "still playing in the tray" notice has been shown.
    TrayNoticeShown,
    Accent(Accent),
    /// A colour picked for the custom accent, which also becomes the accent.
    CustomAccent(AccentColour),
    Theme(ThemeMode),
    Hotkeys(Option<Hotkeys>),
    /// Lyrics show at all: the Lyrics toggles and tab, and their lookups.
    Lyrics(bool),
    /// The lyrics view looks up synced lyrics on LRCLIB.
    LyricsLrclib(bool),
    /// A list that runs out plays on with similar songs (on) or restarts (off).
    Autoplay(bool),
    /// What an identity (its key) pinned to the top of the sidebar's list, newest first.
    Pins(String, Vec<String>),
    /// Ask GitHub once a day whether a newer featherplay is out.
    CheckForUpdates(bool),
    /// Download a newer release's setup and offer "Restart to update" (the installed copy).
    InstallUpdates(bool),
    /// The local API on or off.
    LocalApi(bool),
    /// The local API's port (1024 or higher; the page checks it).
    LocalApiPort(u16),
    /// The Language setting: "" for Windows default, "en", or a bundled code.
    Language(String),
    /// What a wheel notch and Up or Down move the volume, in percent (one of
    /// `Settings::VOLUME_STEPS`; the window applies it).
    VolumeStep(u8),
    /// The format streams are asked for, from the next resolve on.
    StreamingQuality(StreamingQuality),
    /// A left click on the tray icon opens the main window (on) or the mini player (off).
    TrayClickOpensWindow(bool),
}

impl Change {
    pub fn apply(&self, s: &mut Settings) {
        match self.clone() {
            Change::PreferSongVersions(v) => s.prefer_song_versions = v,
            Change::SkipNonMusicInRadio(v) => s.skip_non_music_in_radio = v,
            Change::SkipDisliked(v) => s.skip_disliked = v,
            Change::Sponsorblock(v) => s.sponsorblock = v,
            Change::OutputDevice(v) => s.output_device = v,
            Change::Scrobbling(v) => s.scrobbling = v,
            Change::Scrobble(v) => s.lastfm_enabled = v,
            Change::ListenBrainz(v) => s.listenbrainz_enabled = v,
            Change::ScrobbleOtherMedia(v) => s.scrobble_other_media = v,
            Change::AlternativeTitles(v) => s.alternative_titles = v,
            Change::AlternativeArtists(v) => s.alternative_artists = v,
            Change::Discord(v) => s.discord = v,
            Change::DiscordOptions(v) => s.discord_options = v,
            Change::Notifications(v) => s.notifications = v,
            Change::CloseAction(v) => s.close_action = v,
            Change::TrayNoticeShown => s.tray_notice_shown = true,
            Change::Accent(v) => s.accent = v,
            Change::CustomAccent(v) => {
                s.accent = Accent::Custom;
                s.custom_accent = v;
            }
            Change::Theme(v) => s.theme = v,
            Change::Hotkeys(v) => s.hotkeys = v,
            Change::Lyrics(v) => s.lyrics = v,
            Change::LyricsLrclib(v) => s.lyrics_lrclib = v,
            Change::Autoplay(v) => s.autoplay = v,
            Change::CheckForUpdates(v) => s.check_for_updates = v,
            Change::InstallUpdates(v) => s.install_updates = v,
            Change::Pins(identity, pins) => {
                if pins.is_empty() {
                    s.pins.remove(&identity);
                } else {
                    s.pins.insert(identity, pins);
                }
            }
            Change::LocalApi(v) => s.local_api = v,
            Change::LocalApiPort(v) => s.local_api_port = v,
            Change::Language(v) => s.language = v,
            Change::VolumeStep(v) => s.volume_step = v,
            Change::StreamingQuality(v) => s.streaming_quality = v,
            Change::TrayClickOpensWindow(v) => s.tray_click_opens_window = v,
        }
    }
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
    SignedIn(Result<YtMusic, String>),
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
                tokio::task::spawn_blocking(move || {
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
                tokio::task::spawn_blocking(|| {
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
                tokio::task::spawn_blocking(|| {
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
            AccountMsg::SignedIn(Ok(api)) => {
                log!("signed in");
                self.unreachable = None;
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

    /// "Session expired, sign in again": the music keeps playing; the window asks to sign
    /// in. The cookie secret stays until a new sign-in replaces it.
    pub(super) fn session_expired(&mut self) {
        log!("session expired, sign in again");
        self.api = None;
        self.unreachable = None;
        self.ui.catalog.sign_out();
        (self.ui.events)(UiEvent::AccountNotice(AccountEvent::SessionExpired));
        (self.ui.events)(UiEvent::Account { signed_in: false });
    }

    fn sign_in(&mut self, cookies: Cookies) {
        log!("signing in ({} byte cookie header)", cookies.0.len());
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = check_session(cookies).await;
            let _ = tx.send(Msg::Account(AccountMsg::SignedIn(result)));
        });
    }

    /// Acts as `identity` from now on: the catalog, likes and playlist edits. The music
    /// keeps playing. Saved for the next start.
    fn switch_identity(&mut self, identity: Identity) {
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
    pub(super) fn check_identity(&self) {
        let Some(api) = self.api.clone() else { return };
        if api.identity().is_default() {
            return;
        }
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let identity = api.identity().clone();
            let listed = api
                .accounts()
                .await
                .map(|list| list.iter().any(|a| a.identity == identity));
            let _ = tx.send(Msg::Account(AccountMsg::IdentityChecked(identity, listed)));
        });
    }

    fn sign_out(&mut self) {
        log!("signing out");
        self.api = None;
        self.unreachable = None;
        if self.settings.identity.take().is_some() {
            self.save_settings();
        }
        self.ui.catalog.sign_out();
        forget_library(&self.paths);
        // The sign-in window's profile too, so the next sign-in can pick another account.
        let profile = self.paths.cache.join("webview2");
        tokio::task::spawn_blocking(move || {
            if let Err(e) = delete_secret(SecretKey::YtCookies) {
                log!("deleting the session failed: {e}");
            }
            remove_profile(&profile);
        });
        (self.ui.events)(UiEvent::Account { signed_in: false });
    }

    fn change(&mut self, change: Change) {
        log!("setting {}", describe(&change));
        change.apply(&mut self.settings);
        self.save_settings();
        match change {
            Change::PreferSongVersions(_)
            | Change::SkipNonMusicInRadio(_)
            | Change::ScrobbleOtherMedia(_) => {
                self.input(Input::SetPrefs(prefs(&self.settings)));
                self.scrobblers.set_options(&self.settings);
            }
            // The autoplay tracks not played yet go when it's turned off.
            Change::Autoplay(_) => self.input(Input::SetPrefs(prefs(&self.settings))),
            Change::SkipDisliked(_) => self.input(Input::SetPrefs(prefs(&self.settings))),
            // Streams already resolved keep their format.
            Change::StreamingQuality(q) => self.resolver.set_quality(stream_quality(q)),
            Change::Sponsorblock(false) => self.segments = None,
            Change::OutputDevice(name) => self.audio.set_device(name.as_deref()),
            Change::Scrobbling(_) | Change::Scrobble(_) | Change::ListenBrainz(_) => {
                self.restart_scrobbler()
            }
            Change::AlternativeTitles(_) | Change::AlternativeArtists(_) => {
                self.scrobblers.set_options(&self.settings)
            }
            Change::Discord(true) => {
                if self.presence.is_none() {
                    self.presence = start_presence(&self.settings, &self.tx);
                    if self.presence.is_none() {
                        (self.ui.events)(UiEvent::Discord(presence::off_status()));
                    }
                    // Shows the current track on the new connection.
                    self.shown = None;
                    self.push_now_playing();
                }
            }
            Change::Discord(false) => {
                if let Some(p) = self.presence.take() {
                    // Clears the activity; waits up to a second for Discord.
                    tokio::task::spawn_blocking(move || p.quit());
                }
                (self.ui.events)(UiEvent::Discord(presence::off_status()));
            }
            Change::DiscordOptions(o) => {
                if let Some(p) = &self.presence {
                    p.set_options(&o);
                }
            }
            Change::LocalApi(true) => self.start_local_api(),
            Change::LocalApi(false) => self.stop_local_api(),
            Change::LocalApiPort(_) => self.local_api_port_changed(),
            // Read when needed (notifications, SponsorBlock on) or applied by the window
            // (the close button, the accent, the theme, hotkeys).
            _ => {}
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
        tokio::spawn(async move {
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
                    let opened = tokio::task::spawn_blocking(move || open_url(&url))
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
        tokio::spawn(async move {
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
        tokio::task::spawn_blocking(move || {
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
        tokio::spawn(async move {
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
fn forget_library(paths: &featherplay_store::Paths) {
    match featherplay_store::LibraryCache::new(paths).clear() {
        Ok(()) => log!("library cache removed"),
        Err(e) => log!("removing the library cache failed: {}", e.kind()),
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

/// The identity saved by the account menu, or the session's own when there is none (or
/// the file was edited into something that isn't one).
pub(super) fn saved_identity(settings: &Settings) -> Identity {
    settings
        .identity
        .as_deref()
        .and_then(Identity::from_key)
        .unwrap_or_default()
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

/// Signs in with `cookies` and checks the session with one read; stores the header once
/// it works. The error text never contains the header.
async fn check_session(cookies: Cookies) -> Result<YtMusic, String> {
    let api = YtMusic::from_cookie_header(&cookies.0)
        .await
        .map_err(|e| sign_in_error(&e))?;
    api.library_playlists()
        .await
        .map_err(|e| sign_in_error(&e))?;
    let saved = tokio::task::spawn_blocking(move || set_secret(SecretKey::YtCookies, &cookies.0))
        .await
        .map_err(|e| e.to_string())
        .and_then(|r| r.map_err(|e| e.to_string()));
    if let Err(e) = saved {
        // Signed in for this run; the next start asks again.
        log!("saving the session failed: {e}");
    }
    Ok(api)
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

fn lastfm_error(e: &LastFmError) -> String {
    match e {
        LastFmError::Http(_) => {
            tr!("Couldn't reach Last.fm. Check the connection and try again.").into()
        }
        LastFmError::Api { code, message } => tr!("Last.fm said: {0} ({1})", message, code),
        other => other.to_string(),
    }
}

/// The change for the log: the setting and its new value (device names included, never
/// secrets; there are none in settings).
fn describe(c: &Change) -> String {
    match c {
        Change::Hotkeys(Some(h)) => {
            format!("hotkeys {} / {} / {}", h.play_pause, h.next, h.previous)
        }
        other => format!("{other:?}"),
    }
}

/// Opens `url` in the default browser.
pub(crate) fn open_url(url: &str) -> Result<(), String> {
    use windows::Win32::System::Com::{
        COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE, CoInitializeEx, CoUninitialize,
    };
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    use windows::core::{HSTRING, w};

    let url = HSTRING::from(url);
    // SAFETY: COM is initialised on this (blocking pool) thread for the shell call and
    // released after; the strings outlive the call.
    unsafe {
        let com = CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
        let done = ShellExecuteW(None, w!("open"), &url, None, None, SW_SHOWNORMAL);
        if com.is_ok() {
            CoUninitialize();
        }
        // Values above 32 mean success.
        if done.0 as isize > 32 {
            Ok(())
        } else {
            Err(format!("ShellExecute returned {}", done.0 as isize))
        }
    }
}

#[cfg(test)]
mod tests {
    use featherplay_core::player::ListEnd;

    use super::*;

    #[test]
    fn a_network_failure_reads_plainly_on_the_sign_in_page() {
        let raw = "error sending request for url (https://music.youtube.com/)";
        let text = sign_in_error(&ApiError::Network(raw.into()));
        assert!(!text.contains("error sending request"), "{text}");
        assert!(!text.contains("https://"), "{text}");
        assert!(text.contains("try again"), "{text}");
    }

    #[test]
    fn a_custom_colour_is_picked_and_kept_while_another_accent_is_in_use() {
        let mut s = Settings::default();
        Change::CustomAccent(AccentColour(0x3A7BFF)).apply(&mut s);
        assert_eq!(s.accent, Accent::Custom);
        assert_eq!(s.custom_accent, AccentColour(0x3A7BFF));
        Change::Accent(Accent::Teal).apply(&mut s);
        assert_eq!(s.custom_accent, AccentColour(0x3A7BFF));
    }

    #[test]
    fn every_change_lands_in_its_field() {
        let mut s = Settings::default();
        let changes = [
            Change::PreferSongVersions(false),
            Change::SkipNonMusicInRadio(false),
            Change::Sponsorblock(false),
            Change::OutputDevice(Some("Headphones".into())),
            Change::Scrobbling(false),
            Change::Scrobble(true),
            Change::ListenBrainz(true),
            Change::ScrobbleOtherMedia(false),
            Change::AlternativeTitles(false),
            Change::AlternativeArtists(false),
            Change::Discord(true),
            Change::DiscordOptions(DiscordOptions {
                play_button: false,
                ..DiscordOptions::default()
            }),
            Change::Notifications(true),
            Change::CloseAction(CloseAction::Quit),
            Change::TrayNoticeShown,
            Change::Accent(Accent::Violet),
            Change::Theme(ThemeMode::Light),
            Change::Hotkeys(Some(Hotkeys {
                play_pause: "Ctrl+Alt+Space".into(),
                next: "Ctrl+Alt+Right".into(),
                previous: "Ctrl+Alt+Left".into(),
            })),
            Change::Lyrics(false),
            Change::LyricsLrclib(false),
            Change::Autoplay(false),
            Change::CheckForUpdates(false),
            Change::InstallUpdates(false),
            Change::LocalApi(true),
            Change::LocalApiPort(31000),
            Change::SkipDisliked(false),
            Change::StreamingQuality(StreamingQuality::Standard),
            Change::TrayClickOpensWindow(true),
        ];
        for c in &changes {
            c.apply(&mut s);
        }
        let d = Settings::default();
        assert_ne!(s.prefer_song_versions, d.prefer_song_versions);
        assert_ne!(s.skip_non_music_in_radio, d.skip_non_music_in_radio);
        assert_ne!(s.sponsorblock, d.sponsorblock);
        assert_eq!(s.output_device.as_deref(), Some("Headphones"));
        assert!(s.lastfm_enabled && s.discord && s.notifications);
        assert_eq!(s.accent, Accent::Violet);
        assert!(!s.scrobble_other_media && s.tray_notice_shown);
        assert_eq!(s.close_action, CloseAction::Quit);
        assert!(!s.scrobbling && s.listenbrainz_enabled);
        assert!(!s.alternative_titles && !s.alternative_artists);
        assert!(s.hotkeys.is_some());
        assert_eq!(s.theme, ThemeMode::Light);
        assert!(s.local_api);
        assert_eq!(s.local_api_port, 31000);
        assert!(!s.discord_options.play_button);
        assert!(!s.lyrics && !s.lyrics_lrclib);
        assert!(!s.autoplay);
        assert!(!s.skip_disliked);
        assert_eq!(s.streaming_quality, StreamingQuality::Standard);
        assert!(s.tray_click_opens_window);
        assert!(!s.check_for_updates && !s.install_updates);
        assert_eq!(s.volume, d.volume, "the volume isn't a page setting");
    }

    #[test]
    fn autoplay_on_plays_on_and_off_restarts_the_list() {
        let mut s = Settings::default();
        let on = crate::controller::prefs(&s).list_end;
        s.autoplay = false;
        let off = crate::controller::prefs(&s).list_end;
        assert_eq!((on, off), (ListEnd::Autoplay, ListEnd::Restart));
    }

    #[test]
    fn skip_disliked_reaches_the_player() {
        let mut s = Settings::default();
        let on = crate::controller::prefs(&s).skip_disliked;
        s.skip_disliked = false;
        let off = crate::controller::prefs(&s).skip_disliked;
        assert_eq!((on, off), (true, false));
    }

    #[test]
    fn streaming_quality_picks_the_resolver_quality() {
        use featherplay_streams::Quality;
        let q = crate::controller::stream_quality;
        assert_eq!(q(StreamingQuality::High), Quality::High);
        assert_eq!(q(StreamingQuality::Standard), Quality::Standard);
        assert_eq!(q(StreamingQuality::default()), Quality::default());
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
        forget_library(&paths);
        assert_eq!(cache.load::<Vec<String>>("", LibraryList::Liked), None);
        assert_eq!(cache.load::<Vec<String>>("2", LibraryList::Albums), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

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
