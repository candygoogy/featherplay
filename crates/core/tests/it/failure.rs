use featherplay_core::failure::{
    FailKind, HelperFault, Notice, StopCause, getting_ready, notice, stop_cause,
};

use FailKind::{Broken, Other, Setup, Unavailable};
use HelperFault::{Blocked, Damaged, NoSpace, Unreachable};

#[test]
fn a_broken_resolver_in_the_streak_makes_the_stop_broken() {
    assert_eq!(stop_cause(&[Broken, Broken, Broken]), StopCause::Broken);
    assert_eq!(stop_cause(&[Unavailable, Broken, Other]), StopCause::Broken);
}

#[test]
fn songs_that_are_gone_stop_as_unavailable() {
    assert_eq!(stop_cause(&[Unavailable]), StopCause::Unavailable);
    assert_eq!(
        stop_cause(&[Unavailable, Unavailable, Unavailable]),
        StopCause::Unavailable
    );
}

#[test]
fn anything_else_stops_as_other() {
    assert_eq!(stop_cause(&[Other]), StopCause::Other);
    assert_eq!(stop_cause(&[Unavailable, Other]), StopCause::Other);
    assert_eq!(stop_cause(&[]), StopCause::Other);
}

#[test]
fn a_failed_helpers_download_makes_the_stop_setup() {
    assert_eq!(
        stop_cause(&[Setup(Unreachable)]),
        StopCause::Setup(Unreachable)
    );
    assert_eq!(
        stop_cause(&[Unavailable, Broken, Setup(Blocked)]),
        StopCause::Setup(Blocked)
    );
}

#[test]
fn the_last_helpers_failure_says_why_they_didnt_download() {
    assert_eq!(
        stop_cause(&[Setup(Unreachable), Other, Setup(Damaged)]),
        StopCause::Setup(Damaged)
    );
}

#[test]
fn a_setup_stop_says_why_the_helpers_didnt_download() {
    for fault in [Unreachable, Blocked, Damaged, NoSpace] {
        for tried in [false, true] {
            assert_eq!(
                notice(StopCause::Setup(fault), tried),
                Notice::HelpersFailed(fault)
            );
        }
    }
}

#[test]
fn a_broken_stop_first_updates_the_helpers() {
    assert_eq!(notice(StopCause::Broken, false), Notice::Updating);
}

#[test]
fn a_broken_stop_after_the_helpers_update_asks_for_a_featherplay_update() {
    assert_eq!(notice(StopCause::Broken, true), Notice::UpdateNeeded);
}

#[test]
fn unavailable_and_other_stops_ignore_the_helpers() {
    for tried in [false, true] {
        assert_eq!(notice(StopCause::Unavailable, tried), Notice::Unavailable);
        assert_eq!(notice(StopCause::Other, tried), Notice::CouldNotPlay);
    }
}

#[test]
fn notices_are_plain_words_without_tool_names() {
    for text in [
        Notice::Updating,
        Notice::UpdateNeeded,
        Notice::Unavailable,
        Notice::CouldNotPlay,
        Notice::HelpersFailed(Unreachable),
        Notice::HelpersFailed(Blocked),
        Notice::HelpersFailed(Damaged),
        Notice::HelpersFailed(NoSpace),
    ]
    .map(Notice::text)
    .into_iter()
    .chain([getting_ready()])
    {
        assert!(!text.is_empty());
        for word in ["yt-dlp", "ERROR", "stderr", "sidecar", "deno"] {
            assert!(!text.contains(word), "{text:?} names {word}");
        }
        // One line in the toast, beside its action button.
        assert!(text.chars().count() <= 56, "{text:?} is too long");
    }
}

#[test]
fn the_wording_says_what_happened_and_what_happens_next() {
    assert_eq!(
        Notice::Updating.text(),
        "Songs can't play. featherplay is updating its helpers."
    );
    assert_eq!(
        Notice::UpdateNeeded.text(),
        "Songs can't play. featherplay may need an update."
    );
    assert_eq!(Notice::Unavailable.text(), "This song isn't available.");
    assert_eq!(
        Notice::CouldNotPlay.text(),
        "Couldn't play this song. Check your connection."
    );
    assert_eq!(
        Notice::HelpersFailed(Unreachable).text(),
        "Couldn't download helpers. Check VPN or proxy."
    );
    assert_eq!(
        Notice::HelpersFailed(Blocked).text(),
        "Your antivirus may have blocked the helpers."
    );
    assert_eq!(
        Notice::HelpersFailed(Damaged).text(),
        "A helper download arrived damaged."
    );
    assert_eq!(
        Notice::HelpersFailed(NoSpace).text(),
        "Couldn't save helpers. Is the disk full?"
    );
    assert_eq!(
        getting_ready(),
        "Getting ready to play: downloading helpers (one time)…"
    );
}
