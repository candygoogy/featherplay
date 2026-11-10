//! A stored session that couldn't connect (a network or parse failure, at startup or on a
//! later try) leaves the app `Source::Unreachable`, not signed out: pages say the music
//! service can't be reached and offer Try again.
//!
//! Reconnects use the stored cookies and the saved identity, one connect at a time: a few
//! automatic ones (`BACKOFF` after each failure), then only when a page's Try again asks
//! (`Action::Reconnect`). No timer runs once connected, signed out, or after the last
//! automatic try.

use std::time::{Duration, Instant};

use featherplay_core::player::Input;
use featherplay_store::{Paths, load_client_version, save_client_version};
use featherplay_ytm::{ApiError, Identity, YtMusic};

use super::{Controller, Msg, UiEvent, account, cookie_header};
use crate::catalog::Source;

/// The pauses before the automatic reconnects, one per failure; after the last one, only
/// Try again reconnects.
const BACKOFF: [Duration; 3] = [
    Duration::from_secs(10),
    Duration::from_secs(30),
    Duration::from_secs(120),
];

/// What a connect with the stored session leads to.
pub(super) enum Outcome {
    Live(Box<YtMusic>),
    /// The session was refused: "Session expired, sign in again".
    Expired,
    /// A network or parse failure: the session may well be fine.
    Unreachable,
}

/// Signs in with the stored session at startup: at once with the client version the last
/// sign-in kept (`YtMusic::resume`, no request; `refresh_session` checks it behind), or with
/// `YtMusic::connect`'s request when none is kept (a first start) or it can't be used.
/// The bool: resumed.
pub(super) async fn stored_session(
    cookies: &str,
    identity: &Identity,
    paths: &Paths,
) -> (Result<YtMusic, ApiError>, bool) {
    if let Some(version) = load_client_version(paths) {
        match YtMusic::resume(cookies, identity, &version) {
            Ok(api) => return (Ok(api), true),
            // Refused cookies are `connect`'s to say too.
            Err(e) => log!("resuming with the kept client version failed: {e}"),
        }
    }
    let result = YtMusic::connect(cookies, identity).await;
    if let Ok(api) = &result {
        keep_version(paths, api.client_version());
    }
    (result, false)
}

/// Keeps a sign-in's client version for the next start. A small file; blocks briefly.
pub(super) fn keep_version(paths: &Paths, version: Option<String>) {
    let Some(version) = version else { return };
    if load_client_version(paths).as_deref() == Some(version.as_str()) {
        return;
    }
    if let Err(e) = save_client_version(paths, &version) {
        log!("keeping the client version failed: {}", e.kind());
    }
}

/// Sorts a connect's result. The error goes to the log.
pub(super) fn outcome(result: Result<YtMusic, ApiError>) -> Outcome {
    match result {
        Ok(api) => Outcome::Live(Box::new(api)),
        Err(ApiError::AuthExpired) => Outcome::Expired,
        Err(e) => {
            log!("connecting with the stored session failed: {e}");
            Outcome::Unreachable
        }
    }
}

/// The reconnect schedule while unreachable.
#[derive(Debug)]
pub(super) struct Reconnect {
    /// Automatic tries scheduled so far.
    scheduled: usize,
    in_flight: bool,
    due: Option<Instant>,
}

impl Reconnect {
    /// Right after the first failed connect.
    pub(super) fn after_failure(now: Instant) -> Reconnect {
        let mut r = Reconnect {
            scheduled: 0,
            in_flight: false,
            due: None,
        };
        r.failed(now);
        r
    }

    /// A connect failed at `now`: the next automatic try, if any is left.
    pub(super) fn failed(&mut self, now: Instant) {
        self.in_flight = false;
        self.due = BACKOFF.get(self.scheduled).map(|wait| now + *wait);
        if self.due.is_some() {
            self.scheduled += 1;
        }
    }

    /// Whether a connect may start now; it then counts as running until `failed` (or the
    /// state is dropped), and the pending automatic try is off.
    pub(super) fn begin(&mut self) -> bool {
        if self.in_flight {
            return false;
        }
        self.in_flight = true;
        self.due = None;
        true
    }

    /// When the next automatic try is due; `None` while one runs or after the last.
    pub(super) fn due(&self) -> Option<Instant> {
        self.due
    }
}

impl Controller {
    /// A session resumed at startup signs in behind the first pages (`YtMusic::refresh`,
    /// the request `connect` would have waited for), once.
    pub(super) fn refresh_session(&self) {
        let Some(api) = self.api.clone().filter(|_| self.resumed) else {
            return;
        };
        let tx = self.tx.clone();
        crate::app::spawn_detached(async move {
            let _ = tx.send(Msg::Refreshed(api.refresh().await));
        });
    }

    /// The resumed session's sign-in: its fresh client version is kept for the next start.
    /// Refused cookies or a failure lead where a startup `connect`'s would have: "Session
    /// expired", or unreachable with its reconnects (the page and the API are one host, so
    /// the pages' requests are failing too; a reconnect reloads them).
    pub(super) fn refreshed(&mut self, result: Result<Option<String>, ApiError>) {
        if !std::mem::take(&mut self.resumed) {
            // Signed in, out or expired meanwhile: that state stands.
            return;
        }
        match result {
            Ok(version) => {
                log!("signed in behind the start");
                keep_version(&self.paths, version);
            }
            Err(ApiError::AuthExpired) => self.input(Input::AuthExpired),
            Err(e) => {
                log!("connecting with the stored session failed: {e}");
                self.api = None;
                self.unreachable = Some(Reconnect::after_failure(Instant::now()));
                self.ui.catalog.set(Source::Unreachable);
                (self.ui.events)(UiEvent::Unreachable);
            }
        }
    }

    /// Connects with the stored session again, unless one is running already or the app
    /// isn't unreachable (connected or signed out meanwhile).
    pub(super) fn reconnect(&mut self) {
        let Some(state) = &mut self.unreachable else {
            return;
        };
        if !state.begin() {
            return;
        }
        log!("reconnecting with the stored session");
        let identity = account::saved_identity(&self.settings);
        let tx = self.tx.clone();
        crate::app::spawn_detached(async move {
            let result = match cookie_header() {
                Some(cookies) => Some(YtMusic::connect(&cookies, &identity).await),
                None => None,
            };
            let _ = tx.send(Msg::Reconnected(result));
        });
    }

    /// A reconnect's result; `None`: the stored session is gone.
    pub(super) fn reconnected(&mut self, result: Option<Result<YtMusic, ApiError>>) {
        let Some(state) = &mut self.unreachable else {
            // Signed in, out or expired meanwhile: that state stands.
            return;
        };
        match result.map(outcome) {
            Some(Outcome::Unreachable) => {
                state.failed(Instant::now());
                match state.due() {
                    Some(due) => log!(
                        "next reconnect in {}s",
                        due.saturating_duration_since(Instant::now()).as_secs()
                    ),
                    None => log!("no more automatic reconnects; Try again reconnects"),
                }
                (self.ui.events)(UiEvent::Unreachable);
            }
            Some(Outcome::Live(api)) => {
                let api = *api;
                log!("signed in");
                keep_version(&self.paths, api.client_version());
                self.unreachable = None;
                self.api = Some(api.clone());
                self.ui.catalog.set(Source::Live(api));
                (self.ui.events)(UiEvent::Account { signed_in: true });
                self.check_identity();
            }
            Some(Outcome::Expired) => {
                self.unreachable = None;
                self.input(Input::AuthExpired);
            }
            None => {
                log!("no cookies: signed out");
                self.unreachable = None;
                self.ui.catalog.sign_out();
                (self.ui.events)(UiEvent::Account { signed_in: false });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_tries_come_10s_30s_and_2min_after_each_failure_then_stop() {
        let t0 = Instant::now();
        let mut r = Reconnect::after_failure(t0);
        let mut waits = Vec::new();
        let mut now = t0;
        while let Some(due) = r.due() {
            waits.push(due - now);
            assert!(r.begin());
            now = due + Duration::from_secs(1);
            r.failed(now);
        }
        assert_eq!(
            waits,
            [
                Duration::from_secs(10),
                Duration::from_secs(30),
                Duration::from_secs(120)
            ]
        );
    }

    #[test]
    fn only_one_connect_runs_at_a_time() {
        let mut r = Reconnect::after_failure(Instant::now());
        assert!(r.begin());
        assert!(!r.begin(), "a second connect while one runs");
        assert_eq!(r.due(), None, "no timer while a connect runs");
    }

    #[test]
    fn a_network_or_parse_failure_is_unreachable_and_a_refusal_still_expires() {
        let network = outcome(Err(ApiError::Network("timed out".into())));
        let parse = outcome(Err(ApiError::Parse("odd".into())));
        let refused = outcome(Err(ApiError::AuthExpired));
        assert!(matches!(network, Outcome::Unreachable));
        assert!(matches!(parse, Outcome::Unreachable));
        assert!(matches!(refused, Outcome::Expired));
    }
}
