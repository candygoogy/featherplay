//! Settings' Scrobbling section (ui/pages/settings/scrobbling.slint): the Last.fm connection
//! and API settings, and the ListenBrainz token.

use std::rc::Rc;

use featherplay_core::tr;
use slint::ComponentHandle;

use super::{LASTFM, LISTENBRAINZ, SETTINGS, close_dialog, open_dialog};
use crate::app::Ctx;
use crate::controller::{
    AccountAction, Action, ApiKeys, LastfmStatus, ListenBrainzStatus, Token as ListenBrainzToken,
};
use crate::ui::{LastfmState, ListenBrainzState, SettingsData, SettingsDialog};

/// The controller's Last.fm news.
pub fn lastfm_changed(ctx: &Rc<Ctx>, status: LastfmStatus, message: Option<String>) {
    let was = LASTFM.with(|l| l.replace(status));
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SettingsData>();
    data.set_lastfm_state(lastfm_state(status));
    data.set_lastfm_message(message.unwrap_or_default().into());
    // The controller turns scrobbling on with a new session and off on sign-out; a status
    // check leaves the switch as the user set it.
    let scrobble = match (was, status) {
        (LastfmStatus::Finishing, LastfmStatus::SignedIn) => Some(true),
        (LastfmStatus::SignedIn, LastfmStatus::SignedOut) => Some(false),
        _ => None,
    };
    if let Some(on) = scrobble {
        SETTINGS.with(|s| s.borrow_mut().lastfm_enabled = on);
        data.set_scrobble(on);
    }
}

/// Whether the user's own Last.fm API key is stored.
pub fn lastfm_keys_changed(ctx: &Rc<Ctx>, custom: bool) {
    let Some(ui) = ctx.window() else { return };
    ui.global::<SettingsData>().set_lastfm_custom_key(custom);
}

/// The controller's ListenBrainz news.
pub fn listenbrainz_changed(ctx: &Rc<Ctx>, status: ListenBrainzStatus, message: Option<String>) {
    let was = LISTENBRAINZ.with(|l| l.replace(status));
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SettingsData>();
    data.set_listenbrainz_state(listenbrainz_state(status));
    data.set_listenbrainz_message(message.unwrap_or_default().into());
    // As for Last.fm: a new token turns it on, removing the token turns it off.
    let on = match (was, status) {
        (ListenBrainzStatus::Checking, ListenBrainzStatus::SignedIn) => Some(true),
        (ListenBrainzStatus::SignedIn, ListenBrainzStatus::SignedOut) => Some(false),
        _ => None,
    };
    if let Some(on) = on {
        SETTINGS.with(|s| s.borrow_mut().listenbrainz_enabled = on);
        data.set_listenbrainz(on);
    }
}

/// `--snapshot settings-lastfm-keys` and `settings-listenbrainz`: the page with a dialog open.
pub fn show_dialog_for_snapshot(ctx: &Rc<Ctx>, lastfm: bool) {
    let which = if lastfm {
        SettingsDialog::LastfmKeys
    } else {
        SettingsDialog::ListenbrainzToken
    };
    open_dialog(ctx, which);
}

/// OK in the Last.fm API settings: both fields empty changes nothing; otherwise both must
/// look like Last.fm keys.
pub(super) fn save_keys(ctx: &Ctx, key: &str, secret: &str) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SettingsData>();
    match check_keys(key, secret) {
        Ok(None) => close_dialog(ctx),
        Ok(Some(keys)) => {
            close_dialog(ctx);
            ctx.send(Action::Account(AccountAction::LastfmKeys(Some(keys))));
        }
        Err(why) => data.set_dialog_error(why.into()),
    }
}

/// The keys to save from the dialog's fields: `None` when both are empty.
pub fn check_keys(key: &str, secret: &str) -> Result<Option<ApiKeys>, &'static str> {
    let (key, secret) = (key.trim(), secret.trim());
    let looks_right = |s: &str| s.len() == 32 && s.bytes().all(|b| b.is_ascii_hexdigit());
    match (key.is_empty(), secret.is_empty()) {
        (true, true) => Ok(None),
        (false, true) | (true, false) => Err(tr!("Enter both the API key and the shared secret.")),
        _ if !looks_right(key) || !looks_right(secret) => Err(tr!(
            "A Last.fm API key and its shared secret are 32 letters and digits each. Copy them again from last.fm/api/accounts."
        )),
        _ => Ok(Some(ApiKeys {
            key: key.to_string(),
            secret: secret.to_string(),
        })),
    }
}

pub(super) fn save_token(ctx: &Ctx, token: &str) {
    let Some(ui) = ctx.window() else { return };
    let token = token.trim();
    if token.is_empty() {
        ui.global::<SettingsData>()
            .set_dialog_error(tr!("Paste your user token first.").into());
        return;
    }
    let token = ListenBrainzToken(token.to_string());
    close_dialog(ctx);
    ctx.send(Action::Account(AccountAction::ListenbrainzSave(token)));
}

pub(super) fn listenbrainz_state(status: ListenBrainzStatus) -> ListenBrainzState {
    match status {
        ListenBrainzStatus::SignedOut => ListenBrainzState::SignedOut,
        ListenBrainzStatus::Checking => ListenBrainzState::Checking,
        ListenBrainzStatus::SignedIn => ListenBrainzState::SignedIn,
    }
}

pub(super) fn lastfm_state(status: LastfmStatus) -> LastfmState {
    match status {
        LastfmStatus::SignedOut => LastfmState::SignedOut,
        LastfmStatus::Opening => LastfmState::Opening,
        LastfmStatus::Waiting => LastfmState::Waiting,
        LastfmStatus::Finishing => LastfmState::Finishing,
        LastfmStatus::SignedIn => LastfmState::SignedIn,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_keys_are_checked_as_a_pair() {
        let key = "0123456789abcdef0123456789ABCDEF";
        assert!(matches!(check_keys(" ", ""), Ok(None)));
        assert!(check_keys(key, "").unwrap_err().starts_with("Enter both"));
        assert!(check_keys("", key).unwrap_err().starts_with("Enter both"));
        assert!(check_keys("short", key).unwrap_err().contains("32"));
        assert!(check_keys(key, &key.replace('0', "g")).is_err());
        let keys = check_keys(&format!(" {key}\n"), key).unwrap().unwrap();
        assert_eq!((keys.key.as_str(), keys.secret.as_str()), (key, key));
    }
}
