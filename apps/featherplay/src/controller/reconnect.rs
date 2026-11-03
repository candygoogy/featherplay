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
use featherplay_ytm::{ApiError, YtMusic};

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
    Live(YtMusic),
    /// The session was refused: "Session expired, sign in again".
    Expired,
    /// A network or parse failure: the session may well be fine.
    Unreachable,
}

/// Sorts a connect's result. The error goes to the log.
pub(super) fn outcome(result: Result<YtMusic, ApiError>) -> Outcome {
    match result {
        Ok(api) => Outcome::Live(api),
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
        tokio::spawn(async move {
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
                log!("signed in");
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
                self.ui.catalog.set(Source::SignedOut);
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
