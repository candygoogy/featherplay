//! What the window says when playback stops: plain words in a toast, the raw text in the
//! log only.
//!
//! When the resolver broke (yt-dlp failed after the direct request), yt-dlp is brought up
//! to date at once, once a session (`YtDlpResolver::update_helpers`; a failing resolve
//! usually ran it already). If songs still fail after that, the toast says featherplay may
//! need an update and links to a newer release, else to the open "Playback broken" issue
//! the maintainer opens, if there is one. That lookup is a fresh update check plus at most
//! one issue search, once a session, and only with "Check for updates" on.

#[cfg(test)]
use featherplay_core::failure::HelperFault;
use featherplay_core::failure::{
    FailKind, Failure, Notice, notice, songs_refused, songs_refused_by_network,
};
use featherplay_core::player::Stop;
use featherplay_streams::ResolveError;
#[cfg(test)]
use featherplay_streams::SidecarError;

use super::{Controller, Msg, UiEvent};
use crate::updates::{self, Help, Release};

/// The once-a-session lookup for the "may need an update" toast.
#[derive(Debug, Default)]
pub(super) enum Lookup {
    #[default]
    NotYet,
    /// Its answer brings the toast.
    Running,
    Done(Option<Help>),
}

/// A failed resolve as the player counts it, with the raw text for the log.
pub(super) fn failure(e: &ResolveError) -> Failure {
    let kind = match e {
        e @ ResolveError::Unavailable(_) if e.is_refusal() => FailKind::Refused,
        ResolveError::Unavailable(_) => FailKind::Unavailable,
        ResolveError::ToolFailed { .. } => FailKind::Broken,
        ResolveError::Sidecar(e) => FailKind::Setup(e.fault),
        // Only a resolve ahead of a click ends so, and the player never gets it (`streams.rs`).
        ResolveError::ToolMissing | ResolveError::Deferred => FailKind::Other,
    };
    Failure {
        kind,
        detail: describe(e),
    }
}

/// The toast for songs passed over as refused. `redirected`: the DNS sends YouTube's hosts
/// away from Google (`featherplay_proxy::dns`) and "Look up YouTube directly" is off, which
/// the first such toast of the session says, as an error with a button that turns it on;
/// `told` keeps that it did.
pub(super) fn refused_toast(redirected: bool, told: &mut bool) -> UiEvent {
    if redirected && !std::mem::replace(told, true) {
        UiEvent::RefusedByDns(songs_refused_by_network())
    } else {
        UiEvent::Toast(songs_refused())
    }
}

fn describe(e: &ResolveError) -> String {
    match e {
        ResolveError::Unavailable(m) => format!("unavailable: {m}"),
        ResolveError::ToolMissing => "yt-dlp or deno is missing".into(),
        ResolveError::ToolFailed { code, stderr_tail } => {
            format!("yt-dlp failed ({code}): {stderr_tail}")
        }
        ResolveError::Sidecar(e) => format!("sidecar ({}): {}", e.fault.name(), e.message),
        ResolveError::Deferred => "not natively, left for a click".into(),
    }
}

impl Controller {
    /// `Effect::ShowError`: playback stopped after failures.
    pub(super) fn stopped(&mut self, stop: Stop) {
        log!("stopped ({:?}): {}", stop.cause, stop.detail);
        let said = notice(stop.cause, self.resolver.helpers_tried());
        match said {
            Notice::Updating => {
                self.tell(said, None);
                let resolver = self.resolver.clone();
                crate::task::spawn_detached(async move {
                    resolver.update_helpers().await;
                    log!("helpers updated after the stop");
                });
            }
            Notice::UpdateNeeded => match &self.lookup {
                Lookup::Done(help) => self.tell(said, help.clone()),
                Lookup::Running => {}
                Lookup::NotYet if !self.settings.check_for_updates => self.tell(said, None),
                Lookup::NotYet => {
                    self.lookup = Lookup::Running;
                    let (cache, tx) = (self.paths.cache.clone(), self.tx.clone());
                    let install = self.settings.install_updates;
                    crate::task::spawn_detached(async move {
                        let (help, release) = updates::help(cache, install).await;
                        let _ = tx.send(Msg::Help(help, release));
                    });
                }
            },
            Notice::Unavailable | Notice::CouldNotPlay => self.tell(said, None),
            // With Copy diagnostics, for a report: the cause is on the user's side, and
            // differs from one PC to the next.
            Notice::HelpersFailed(_) => (self.ui.events)(UiEvent::HelpersFailed(said.text())),
        }
    }

    /// The lookup for the "may need an update" toast answered.
    pub(super) fn helped(&mut self, help: Option<Help>, release: Option<Release>) {
        if let Some(r) = release {
            (self.ui.events)(UiEvent::Release(r));
        }
        self.lookup = Lookup::Done(help.clone());
        self.tell(Notice::UpdateNeeded, help);
    }

    fn tell(&self, said: Notice, help: Option<Help>) {
        (self.ui.events)(UiEvent::Stopped {
            text: said.text(),
            help,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_song_that_is_gone_is_unavailable() {
        let f = failure(&ResolveError::Unavailable("Video unavailable".into()));
        assert_eq!(f.kind, FailKind::Unavailable);
    }

    /// YouTube's "This video is not available" (since 2026-11-11, for some songs on every
    /// route but its music web player): the player passes over the song and plays on.
    #[test]
    fn a_song_youtube_refuses_is_refused() {
        for m in [
            "UNPLAYABLE: This video is not available",
            "[youtube] mCkBsSEG440: Video unavailable. This video is not available",
        ] {
            let f = failure(&ResolveError::Unavailable(m.into()));
            assert_eq!(f.kind, FailKind::Refused, "{m}");
        }
        let gone = "[youtube] abc: Video unavailable. This video has been removed by the uploader";
        let f = failure(&ResolveError::Unavailable(gone.into()));
        assert_eq!(f.kind, FailKind::Unavailable);
    }

    #[test]
    fn the_refused_songs_toast_names_a_dns_redirect_once() {
        let text = |e: UiEvent| match e {
            UiEvent::Toast(t) => ("plain", t),
            UiEvent::RefusedByDns(t) => ("error", t),
            _ => unreachable!(),
        };
        let mut told = false;
        assert_eq!(
            text(refused_toast(false, &mut told)),
            ("plain", songs_refused())
        );
        assert!(!told);
        assert_eq!(
            text(refused_toast(true, &mut told)),
            ("error", songs_refused_by_network())
        );
        assert_eq!(
            text(refused_toast(true, &mut told)),
            ("plain", songs_refused())
        );
    }

    #[test]
    fn a_failed_yt_dlp_is_a_broken_resolver() {
        let failed = ResolveError::ToolFailed {
            code: 1,
            stderr_tail: "ERROR: [youtube] abc: Signature extraction failed".into(),
        };
        assert_eq!(failure(&failed).kind, FailKind::Broken);
    }

    /// Offline on a first run, or a download that didn't match its pin: not something a
    /// `yt-dlp -U` could fix, and every song would fail the same way.
    fn sidecar(fault: HelperFault, message: &str) -> ResolveError {
        ResolveError::Sidecar(SidecarError {
            fault,
            message: message.into(),
            host: None,
            status: None,
            os_error: None,
        })
    }

    #[test]
    fn a_failed_helpers_download_is_setup_with_its_fault() {
        let offline = sidecar(
            HelperFault::Unreachable,
            "https://github.com/…: connection refused",
        );
        assert_eq!(
            failure(&offline).kind,
            FailKind::Setup(HelperFault::Unreachable)
        );
        let damaged = sidecar(HelperFault::Damaged, "sha256 mismatch");
        assert_eq!(
            failure(&damaged).kind,
            FailKind::Setup(HelperFault::Damaged)
        );
    }

    #[test]
    fn a_failed_helpers_download_logs_its_fault() {
        let held = sidecar(HelperFault::Blocked, "installing deno.exe: in use");
        assert_eq!(
            failure(&held).detail,
            "sidecar (blocked): installing deno.exe: in use"
        );
    }

    #[test]
    fn missing_helpers_are_other() {
        assert_eq!(failure(&ResolveError::ToolMissing).kind, FailKind::Other);
    }

    #[test]
    fn the_raw_text_is_kept_for_the_log() {
        let failed = ResolveError::ToolFailed {
            code: 1,
            stderr_tail: "ERROR: boom".into(),
        };
        assert_eq!(failure(&failed).detail, "yt-dlp failed (1): ERROR: boom");
    }
}
