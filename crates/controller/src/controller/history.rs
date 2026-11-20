//! Plays reported to the account's history, as the web player reports them
//! (`YtMusic::report_play`): signed in and with "Add what I play to my history" on, a song
//! that has really played (`Effect::Heard`, 5 s) is reported, off this loop, with one try. A
//! failure is a log line, without the URL. Last.fm, ListenBrainz and Home's Shortcuts don't
//! come into it.

use featherplay_core::model::Track;
use featherplay_ytm::YtMusic;

use super::Controller;

/// The client to report a play with: signed in (`api`) and with the setting on, else none.
fn reporter(api: Option<&YtMusic>, on: bool) -> Option<YtMusic> {
    api.filter(|_| on).cloned()
}

impl Controller {
    /// `Effect::Heard`: reports `t`'s play (its song version's, when that's what plays).
    pub(super) fn heard(&mut self, t: &Track) {
        let Some(api) = reporter(self.api.as_ref(), self.settings.play_history) else {
            return;
        };
        let id = self.player.play_id(t);
        let at = self.player.position();
        crate::task::spawn_detached(async move {
            match api.report_play(&id, at).await {
                Ok(()) => log!("history: {} reported", id.0),
                Err(e) => log!("history: {} not reported: {e}", id.0),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use featherplay_ytm::Identity;

    use super::*;

    /// Signed out or with the setting off, nothing is reported.
    #[test]
    fn signed_out_or_off_nothing_is_sent() {
        let signed_in = YtMusic::resume(
            "SAPISID=x; SID=y;",
            &Identity::default(),
            "1.20261101.01.00",
        )
        .expect("a resumed session, which has sent nothing");
        assert!(reporter(None, true).is_none());
        assert!(reporter(Some(&signed_in), false).is_none());
        assert!(reporter(Some(&signed_in), true).is_some());
    }
}
