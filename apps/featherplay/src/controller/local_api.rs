//! The local API (Settings > Local API, off by default): the server in
//! `featherplay_integrations::local_api`, started at launch when it's on and from Settings.
//!
//! - Its token is made on the first start (`Token::generate`, the system's CSPRNG) and kept
//!   in Credential Manager as `SecretKey::LocalApiToken`, never in the settings file and
//!   never in the log. Regenerate replaces it and restarts the server, which closes the
//!   connections made with the old one.
//! - Its controls arrive as `UiCommand`s on the channel the media keys and the tray use.
//! - `push_state` publishes every player state to it; the server decides what's worth a
//!   WebSocket message, so a tick costs a comparison.

use std::time::{Duration, SystemTime};

use featherplay_core::model::RepeatMode;
use featherplay_integrations::local_api::{LocalApi, Snapshot, Token};
use featherplay_store::{SecretKey, get_secret, set_secret};

use super::{Controller, PlayerState, UiEvent};

/// Where the local API stands, for Settings.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocalApiStatus {
    Off,
    Running {
        port: u16,
    },
    /// It couldn't start; the text says why, for Settings.
    Failed(String),
}

/// The local API's state and its token, for Settings (the page copies the token; it only
/// ever shows a masked form). `Token`'s `Debug` never shows it.
#[derive(Clone, Debug)]
pub struct LocalApiNotice {
    pub status: LocalApiStatus,
    pub token: Option<Token>,
}

/// The controller's side of the server.
#[derive(Default)]
pub(super) struct ApiServer {
    server: Option<LocalApi>,
    /// Read or made on the first start, kept for the run.
    token: Option<Token>,
    failed: Option<String>,
}

impl Controller {
    /// Starts the server on the configured port, or restarts it there; reports to Settings.
    pub(super) fn start_local_api(&mut self) {
        // The old server lets go of its port before the new one binds.
        self.local_api.server = None;
        self.local_api.failed = None;
        let Some(controls) = self.ui.controls.upgrade() else {
            // The window is gone; the app is closing.
            return;
        };
        let token = match self.local_api.token.clone() {
            Some(t) => Some(t),
            None => stored_or_new_token(),
        };
        let Some(token) = token else {
            self.local_api.failed =
                Some("featherplay couldn't make a token for the local API.".into());
            self.report_local_api();
            return;
        };
        self.local_api.token = Some(token.clone());
        let port = self.settings.local_api_port;
        let now = self.api_snapshot();
        match LocalApi::start(port, token, controls, now) {
            Ok(server) => {
                log!("local API on 127.0.0.1:{}", server.port());
                self.local_api.server = Some(server);
            }
            Err(e) => {
                log!("local API on port {port} failed: {e}");
                self.local_api.failed = Some(start_error(port, e.kind()));
            }
        }
        self.report_local_api();
    }

    pub(super) fn stop_local_api(&mut self) {
        if self.local_api.server.take().is_some() {
            log!("local API stopped");
        }
        self.local_api.failed = None;
        self.report_local_api();
    }

    /// A new token, stored, and the server restarted with it.
    pub(super) fn regenerate_api_token(&mut self) {
        let token = match Token::generate() {
            Ok(t) => t,
            Err(e) => {
                log!("local API: a new token failed: {e}");
                return;
            }
        };
        // A few ms of Credential Manager on the controller's own thread, as at startup.
        if let Err(e) = set_secret(SecretKey::LocalApiToken, token.as_str()) {
            log!("local API: saving the new token failed: {e}");
        }
        log!("local API: new token");
        self.local_api.token = Some(token);
        if self.settings.local_api {
            self.start_local_api();
        } else {
            self.report_local_api();
        }
    }

    /// The port setting changed: a running (or failed) server moves to it.
    pub(super) fn local_api_port_changed(&mut self) {
        if self.settings.local_api {
            self.start_local_api();
        }
    }

    /// Settings opened: report the state, and the token once there is one.
    pub(super) fn report_local_api(&self) {
        let status = match (&self.local_api.server, &self.local_api.failed) {
            (Some(server), _) => LocalApiStatus::Running {
                port: server.port(),
            },
            (None, Some(why)) if self.settings.local_api => LocalApiStatus::Failed(why.clone()),
            _ => LocalApiStatus::Off,
        };
        (self.ui.events)(UiEvent::LocalApi(LocalApiNotice {
            status,
            token: self.local_api.token.clone(),
        }));
    }

    /// Hands the player's state to a running server.
    pub(super) fn publish_api(&self, state: &PlayerState) {
        if let Some(server) = &self.local_api.server {
            server.publish(api_snapshot(state, SystemTime::now()));
        }
    }

    fn api_snapshot(&self) -> Snapshot {
        match &self.shown_state {
            Some(state) => api_snapshot(state, SystemTime::now()),
            None => Snapshot::default(),
        }
    }
}

/// The token in Credential Manager, or a new one stored there. A new token that can't be
/// stored still serves this run.
fn stored_or_new_token() -> Option<Token> {
    // A few ms of Credential Manager, on the controller's own thread (as the cookie read at
    // startup).
    if let Some(t) = get_secret(SecretKey::LocalApiToken).and_then(Token::new) {
        return Some(t);
    }
    let token = match Token::generate() {
        Ok(t) => t,
        Err(e) => {
            log!("local API: making a token failed: {e}");
            return None;
        }
    };
    if let Err(e) = set_secret(SecretKey::LocalApiToken, token.as_str()) {
        log!("local API: saving the token failed: {e}");
    }
    Some(token)
}

/// What Settings says when the server can't listen.
pub fn start_error(port: u16, kind: std::io::ErrorKind) -> String {
    match kind {
        std::io::ErrorKind::AddrInUse => format!(
            "Port {port} is in use by another app (Pear Desktop's API Server uses 26538 too). \
             Close it or pick another port."
        ),
        std::io::ErrorKind::PermissionDenied => {
            format!("Windows keeps port {port} for itself. Pick another port.")
        }
        _ => format!("The local API couldn't listen on port {port}. Pick another port."),
    }
}

/// The repeat mode after `presses` of the repeat button (Off, All, One, then Off), as
/// Pear's `switch-repeat` counts them.
pub fn cycle_repeat(mode: RepeatMode, presses: u8) -> RepeatMode {
    (0..presses % 3).fold(mode, |m, _| match m {
        RepeatMode::Off => RepeatMode::All,
        RepeatMode::All => RepeatMode::One,
        RepeatMode::One => RepeatMode::Off,
    })
}

/// Where a skip of `by` from `position` lands: not before the start, nor past the end of a
/// track whose length is known.
pub fn skip(position: Duration, length: Option<Duration>, by: Duration, forward: bool) -> Duration {
    if forward {
        let to = position.saturating_add(by);
        length.map_or(to, |end| to.min(end))
    } else {
        position.saturating_sub(by)
    }
}

/// The server's view of the window's player state, read at `read_at`.
pub fn api_snapshot(state: &PlayerState, read_at: SystemTime) -> Snapshot {
    let volume = if state.volume.is_finite() {
        (state.volume.clamp(0.0, 1.0) * 100.0).round() as u8
    } else {
        0
    };
    Snapshot {
        track: state.current.clone(),
        playing: state.playing,
        audible: state.audible,
        position: state.position,
        read_at,
        volume,
        shuffle: state.shuffle,
        repeat: state.repeat,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeat_presses_go_round() {
        assert_eq!(cycle_repeat(RepeatMode::Off, 0), RepeatMode::Off);
        assert_eq!(cycle_repeat(RepeatMode::Off, 1), RepeatMode::All);
        assert_eq!(cycle_repeat(RepeatMode::Off, 2), RepeatMode::One);
        assert_eq!(cycle_repeat(RepeatMode::One, 1), RepeatMode::Off);
        assert_eq!(cycle_repeat(RepeatMode::All, 3), RepeatMode::All);
    }

    #[test]
    fn skips_stay_within_the_track() {
        let s = Duration::from_secs;
        assert_eq!(skip(s(5), Some(s(100)), s(10), false), s(0));
        assert_eq!(skip(s(50), Some(s(100)), s(10), false), s(40));
        assert_eq!(skip(s(95), Some(s(100)), s(10), true), s(100));
        assert_eq!(skip(s(95), None, s(10), true), s(105));
    }

    fn state() -> PlayerState {
        PlayerState {
            current: None,
            upcoming: Vec::new(),
            playing: true,
            audible: false,
            loading: true,
            position: Duration::from_secs(3),
            shuffle: true,
            repeat: RepeatMode::All,
            radio: false,
            volume: 0.456,
            queue: Default::default(),
        }
    }

    #[test]
    fn the_snapshot_takes_the_players_state() {
        let at = SystemTime::now();
        let s = api_snapshot(&state(), at);
        assert_eq!(s.volume, 46);
        assert!(s.playing && !s.audible && s.shuffle);
        assert_eq!(s.repeat, RepeatMode::All);
        assert_eq!((s.position, s.read_at), (Duration::from_secs(3), at));
    }

    #[test]
    fn a_bad_volume_reads_as_silence() {
        let mut st = state();
        st.volume = f32::NAN;
        assert_eq!(api_snapshot(&st, SystemTime::now()).volume, 0);
        st.volume = 2.0;
        assert_eq!(api_snapshot(&st, SystemTime::now()).volume, 100);
    }

    #[test]
    fn a_taken_port_says_so() {
        let text = start_error(26538, std::io::ErrorKind::AddrInUse);
        assert!(text.starts_with("Port 26538 is in use"), "{text}");
        let text = start_error(5000, std::io::ErrorKind::PermissionDenied);
        assert!(text.contains("Windows keeps port 5000"), "{text}");
    }
}
