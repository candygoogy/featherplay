use featherplay_core::failure::{
    FailKind, HelperFault, Notice, StopCause, getting_ready, notice, stop_cause,
};

use FailKind::{Broken, Other, Setup, Unavailable};
use HelperFault::{Blocked, Damaged, NoSpace, Unreachable};

/// What a streak of failures stops as.
#[test]
fn the_streak_gives_the_stop_its_cause() {
    let cases: [(&str, &[FailKind], StopCause); 10] = [
        (
            "a broken resolver",
            &[Broken, Broken, Broken],
            StopCause::Broken,
        ),
        (
            "a broken resolver in the streak",
            &[Unavailable, Broken, Other],
            StopCause::Broken,
        ),
        ("a song that's gone", &[Unavailable], StopCause::Unavailable),
        (
            "songs that are gone",
            &[Unavailable, Unavailable, Unavailable],
            StopCause::Unavailable,
        ),
        ("anything else", &[Other], StopCause::Other),
        (
            "anything else after one gone",
            &[Unavailable, Other],
            StopCause::Other,
        ),
        ("no failures", &[], StopCause::Other),
        (
            "a failed helpers download",
            &[Setup(Unreachable)],
            StopCause::Setup(Unreachable),
        ),
        (
            "a failed helpers download after others",
            &[Unavailable, Broken, Setup(Blocked)],
            StopCause::Setup(Blocked),
        ),
        (
            "the last helpers failure says why",
            &[Setup(Unreachable), Other, Setup(Damaged)],
            StopCause::Setup(Damaged),
        ),
    ];
    for (case, streak, want) in cases {
        assert_eq!(stop_cause(streak), want, "{case}");
    }
}

/// The notice a stop shows, given whether the helpers were already updated for it.
#[test]
fn the_stop_and_the_helpers_pick_the_notice() {
    let mut cases = vec![
        (
            "a broken stop first updates the helpers",
            StopCause::Broken,
            false,
            Notice::Updating,
        ),
        (
            "a broken stop after the helpers update asks for an update",
            StopCause::Broken,
            true,
            Notice::UpdateNeeded,
        ),
    ];
    for tried in [false, true] {
        cases.push((
            "a song that's gone ignores the helpers",
            StopCause::Unavailable,
            tried,
            Notice::Unavailable,
        ));
        cases.push((
            "any other stop ignores the helpers",
            StopCause::Other,
            tried,
            Notice::CouldNotPlay,
        ));
        for fault in [Unreachable, Blocked, Damaged, NoSpace] {
            cases.push((
                "a setup stop says why the helpers didn't download",
                StopCause::Setup(fault),
                tried,
                Notice::HelpersFailed(fault),
            ));
        }
    }
    for (case, cause, tried, want) in cases {
        assert_eq!(
            notice(cause, tried),
            want,
            "{case} ({cause:?}, tried {tried})"
        );
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
