//! Settings' Local API section (ui/pages/settings/local_api.slint): the server's state, its
//! port and its token, and the Amuse endpoint.

use featherplay_core::tr;
use featherplay_integrations::local_api::{AMUSE_PORT, Token};
use featherplay_store::Settings;
use slint::ComponentHandle;

use super::{API_TOKEN, SETTINGS, commit};
use crate::app::Ctx;
use crate::controller::{Change, LocalApiNotice, LocalApiStatus};
use crate::toast;
use crate::ui::{LocalApiState, SettingsData};

/// The local API's and the Amuse endpoint's states and the token, from the controller.
pub fn local_api_changed(ctx: &Ctx, notice: LocalApiNotice) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SettingsData>();
    let (state, message) = server_state(notice.status);
    data.set_local_api_state(state);
    data.set_local_api_message(message.into());
    let (state, message) = server_state(notice.amuse);
    data.set_amuse_state(state);
    data.set_amuse_message(message.into());
    data.set_local_api_token(notice.token.as_ref().map(mask).unwrap_or_default().into());
    API_TOKEN.with(|t| *t.borrow_mut() = notice.token);
}

/// A server's state for the page, and why it failed.
fn server_state(status: LocalApiStatus) -> (LocalApiState, String) {
    match status {
        LocalApiStatus::Off => (LocalApiState::Off, String::new()),
        LocalApiStatus::Running { .. } => (LocalApiState::Running, String::new()),
        LocalApiStatus::Failed(why) => (LocalApiState::Failed, why),
    }
}

/// `--snapshot settings-local-api`: the local API and Amuse on and running, with a
/// made-up token.
pub fn show_local_api_for_snapshot(ctx: &Ctx) {
    if let Some(ui) = ctx.window() {
        let data = ui.global::<SettingsData>();
        data.set_local_api(true);
        data.set_amuse(true);
    }
    let token = Token::new("0123456789abcdef".repeat(4));
    local_api_changed(
        ctx,
        LocalApiNotice {
            status: LocalApiStatus::Running { port: 26538 },
            token,
            amuse: LocalApiStatus::Running { port: 9863 },
        },
    );
}

/// The token as the page shows it: dots and its last four digits, so it can be told apart
/// from a new one without showing on a stream.
pub fn mask(token: &Token) -> String {
    let t = token.as_str();
    let tail = t.get(t.len().saturating_sub(4)..).unwrap_or_default();
    format!("{}{tail}", "•".repeat(12))
}

pub(super) fn copy_api_token() {
    let Some(ctx) = Ctx::get() else { return };
    let copied = API_TOKEN.with(|t| {
        t.borrow()
            .as_ref()
            .map(|t| featherplay_platform::clipboard::copy_secret(t.as_str()))
    });
    match copied {
        Some(Ok(())) => toast::show(&ctx, tr!("Token copied")),
        Some(Err(e)) => {
            log!("copying the local API token: {e}");
            toast::show_error(&ctx, tr!("Couldn't copy the token. Try again."));
        }
        None => {}
    }
}

/// The port as typed, if it's one the local API can use: not Amuse's, which its own switch
/// takes.
pub fn parse_port(text: &str) -> Result<u16, &'static str> {
    let port = text
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|&p| p >= Settings::LOCAL_API_MIN_PORT)
        .ok_or(tr!("Pick a port from 1024 to 65535."))?;
    if port == AMUSE_PORT {
        return Err(tr!("9863 is the Amuse switch's port. Pick another one."));
    }
    Ok(port)
}

pub(super) fn local_api_port_entered(ctx: &Ctx, text: &str) {
    let Some(ui) = ctx.window() else { return };
    let data = ui.global::<SettingsData>();
    match parse_port(text) {
        Ok(port) => {
            data.set_local_api_port_error("".into());
            data.set_local_api_port(port.to_string().into());
            if SETTINGS.with(|s| s.borrow().local_api_port != port) {
                commit(ctx, Change::LocalApiPort(port));
            }
        }
        Err(why) => data.set_local_api_port_error(why.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_local_api_port_is_1024_or_higher() {
        assert_eq!(parse_port(" 26538 "), Ok(26538));
        assert_eq!(parse_port("1024"), Ok(1024));
        assert_eq!(parse_port("65535"), Ok(65535));
        for bad in ["", "80", "1023", "65536", "0", "-1", "26538x"] {
            assert!(parse_port(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_local_api_leaves_amuses_port_to_it() {
        let why = parse_port("9863").expect_err("Amuse's port");
        assert!(why.contains("Amuse"), "{why}");
    }

    #[test]
    fn the_token_shows_only_its_last_four_digits() {
        let token = Token::new("0123456789abcdef".repeat(4)).expect("a token");
        let shown = mask(&token);
        assert!(shown.ends_with("cdef"));
        assert!(!shown.contains("0123"));
        assert_eq!(shown.chars().count(), 16);
    }
}
