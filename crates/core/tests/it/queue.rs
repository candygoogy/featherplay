use std::time::Duration;

use featherplay_core::model::{LikeStatus, RepeatMode, Track, TrackId, VideoType};
use featherplay_core::queue::{BackAction, Queue};

fn track(id: &str) -> Track {
    Track {
        id: TrackId(id.to_string()),
        title: format!("Title {id}"),
        artists: Vec::new(),
        album: None,
        duration: Some(Duration::from_secs(200)),
        video_type: VideoType::Song,
        thumbnail: None,
        like: LikeStatus::Indifferent,
        counterpart: None,
    }
}

fn tracks(ids: &[&str]) -> Vec<Track> {
    ids.iter().map(|id| track(id)).collect()
}

fn ids(tracks: &[Track]) -> Vec<&str> {
    tracks.iter().map(|t| t.id.0.as_str()).collect()
}

fn current_id(queue: &Queue) -> Option<&str> {
    queue.current().map(|t| t.id.0.as_str())
}

#[test]
fn advance_repeat_all_wraps() {
    let mut q = Queue::new();
    q.replace(tracks(&["a", "b", "c"]), 2, false);
    q.set_repeat(RepeatMode::All);
    assert_eq!(q.advance().map(|t| t.id.0.as_str()), Some("a"));
    assert_eq!(current_id(&q), Some("a"));
}

#[test]
fn advance_repeat_off_at_end_is_none() {
    let mut q = Queue::new();
    q.replace(tracks(&["a", "b", "c"]), 1, false);
    q.set_repeat(RepeatMode::Off);
    assert_eq!(q.advance().map(|t| t.id.0.as_str()), Some("c"));
    assert!(q.advance().is_none());
    // The last track stays current, so a later continuation can still follow it.
    assert_eq!(current_id(&q), Some("c"));
}

/// The tracks that will play next, in the order `advance_past` would reach them.
#[test]
fn peek_ahead_follows_the_play_order() {
    let ahead = |q: &Queue, n| -> Vec<String> {
        q.peek_ahead_past(n, |t| t.id.0 == "c")
            .iter()
            .map(|t| t.id.0.clone())
            .collect()
    };
    let mut q = Queue::new();
    q.replace(tracks(&["a", "b", "c", "d", "e"]), 2, false);
    assert_eq!(ahead(&q, 3), ["d", "e"], "the end of the list");
    q.set_repeat(RepeatMode::All);
    assert_eq!(ahead(&q, 3), ["d", "e", "a"], "round to the start, past c");
    q.replace(tracks(&["a", "b", "c"]), 0, false);
    assert_eq!(ahead(&q, 5), ["b", "a"], "one lap at most");
    q.set_repeat(RepeatMode::One);
    assert_eq!(ahead(&q, 3), ["a"], "the current track again");
}

#[test]
fn advance_repeat_one_returns_same() {
    let mut q = Queue::new();
    q.replace(tracks(&["a", "b", "c"]), 1, false);
    q.set_repeat(RepeatMode::One);
    assert_eq!(q.advance().map(|t| t.id.0.as_str()), Some("b"));
    assert_eq!(q.advance().map(|t| t.id.0.as_str()), Some("b"));
    assert_eq!(ids(q.upcoming()), ["c"]);
}

#[test]
fn back_after_3s_restarts() {
    let mut q = Queue::new();
    q.replace(tracks(&["a", "b", "c"]), 1, false);
    assert_eq!(q.back(Duration::from_millis(3001)), BackAction::Restart);
    assert_eq!(current_id(&q), Some("b"));
    assert_eq!(q.back(Duration::from_secs(2)), BackAction::Previous);
    assert_eq!(current_id(&q), Some("a"));
}

#[test]
fn shuffle_keeps_current_and_is_permutation() {
    let original = ["a", "b", "c", "d", "e", "f", "g", "h", "i", "j"];
    let mut q = Queue::new();
    q.replace(tracks(&original), 2, false);

    q.set_shuffle(true, 42);
    assert_eq!(current_id(&q), Some("c"));
    let shuffled = ids(q.upcoming());
    assert_ne!(
        shuffled,
        &original[3..],
        "seed 42 should reorder the upcoming tracks"
    );
    let mut sorted = shuffled.clone();
    sorted.sort_unstable();
    assert_eq!(sorted, &original[3..]);

    q.set_shuffle(false, 0);
    assert_eq!(current_id(&q), Some("c"));
    assert_eq!(ids(q.upcoming()), &original[3..]);
}

#[test]
fn move_and_remove_adjust_upcoming() {
    let mut q = Queue::new();
    q.replace(tracks(&["a", "b", "c", "d", "e"]), 0, false);
    assert_eq!(ids(q.upcoming()), ["b", "c", "d", "e"]);

    q.move_item(0, 2);
    assert_eq!(ids(q.upcoming()), ["c", "d", "b", "e"]);
    q.move_item(3, 0);
    assert_eq!(ids(q.upcoming()), ["e", "c", "d", "b"]);

    q.remove(1);
    assert_eq!(ids(q.upcoming()), ["e", "d", "b"]);
    // Out-of-range indices are ignored.
    q.remove(9);
    q.move_item(0, 9);
    assert_eq!(ids(q.upcoming()), ["e", "d", "b"]);
    assert_eq!(current_id(&q), Some("a"));
}

#[test]
fn needs_continuation_only_for_radio() {
    let mut q = Queue::new();
    q.replace(tracks(&["a", "b", "c"]), 0, false);
    assert!(!q.needs_continuation());

    q.replace(tracks(&["a", "b", "c", "d"]), 0, true);
    assert!(!q.needs_continuation());
    q.advance();
    assert!(q.needs_continuation());
}

#[test]
fn append_continuation_skips_duplicates() {
    let mut q = Queue::new();
    q.replace(tracks(&["a", "b", "c"]), 1, true);
    q.append_continuation(tracks(&["a", "c", "d", "e", "d"]));
    assert_eq!(ids(q.upcoming()), ["c", "d", "e"]);
}

/// A long playlist's later pages, arriving after a play from it started: they follow the
/// list's own upcoming tracks, before what autoplay added once the list ran short.
#[test]
fn a_list_extended_after_it_started_goes_on_before_autoplay() {
    let mut q = Queue::new();
    q.replace(tracks(&["a", "b"]), 1, false);
    q.append_continuation(tracks(&["x", "y"]));
    q.extend_list(tracks(&["c", "d"]), None);
    assert_eq!(ids(q.upcoming()), ["c", "d", "x", "y"]);
    assert_eq!(q.autoplay_start(), Some(2));
}

/// Up next cleared while a long playlist's later pages still arrive: they don't fill it
/// again. A new play from a list takes them once more.
#[test]
fn a_list_cleared_after_it_started_takes_no_more_of_its_tracks() {
    let mut q = Queue::new();
    q.replace(tracks(&["a", "b", "c"]), 0, false);
    q.clear_upcoming();
    q.extend_list(tracks(&["d", "e"]), None);
    assert!(q.upcoming().is_empty(), "{:?}", ids(q.upcoming()));

    q.replace(tracks(&["a", "b"]), 0, false);
    q.extend_list(tracks(&["c"]), None);
    assert_eq!(ids(q.upcoming()), ["b", "c"]);
}

/// After Shuffle, the songs still to come are mixed in among the upcoming ones, never
/// before the current track nor after autoplay's.
#[test]
fn a_shuffled_list_extended_after_it_started_mixes_the_new_tracks_in() {
    let names = |prefix: &str| (0..20).map(|i| format!("{prefix}{i}")).collect::<Vec<_>>();
    let (first, more) = (names("a"), names("b"));
    let first: Vec<&str> = first.iter().map(String::as_str).collect();
    let more: Vec<&str> = more.iter().map(String::as_str).collect();
    let mut q = Queue::new();
    q.replace(tracks(&first), 0, false);
    q.append_continuation(tracks(&["x"]));
    q.extend_list(tracks(&more), Some(7));

    assert_eq!(current_id(&q), Some("a0"));
    let upcoming = ids(q.upcoming());
    assert_eq!(upcoming.last(), Some(&"x"), "autoplay stays last");
    let list = &upcoming[..upcoming.len() - 1];
    let mut got = list.to_vec();
    got.sort_unstable();
    let mut want: Vec<&str> = first[1..].iter().chain(&more).copied().collect();
    want.sort_unstable();
    assert_eq!(got, want);
    let old = |id: &&str| id.starts_with('a');
    assert!(
        !list[..19].iter().all(old),
        "the new tracks all went after the old ones: {list:?}"
    );
}

#[test]
fn autoplay_starts_at_the_first_continued_track() {
    let mut q = Queue::new();
    q.replace(tracks(&["a", "b", "c"]), 0, true);
    assert!(q.is_radio());
    assert_eq!(q.autoplay_start(), None);

    q.append_continuation(tracks(&["d", "e"]));
    assert_eq!(ids(q.upcoming()), ["b", "c", "d", "e"]);
    assert_eq!(q.autoplay_start(), Some(2));

    // Play next goes before the radio; the divider moves down with it.
    q.play_next(track("x"));
    assert_eq!(q.autoplay_start(), Some(3));
    q.advance();
    q.advance();
    q.advance();
    assert_eq!(ids(q.upcoming()), ["d", "e"]);
    assert_eq!(q.autoplay_start(), Some(0));

    // A new queue has no continuation yet.
    q.replace(tracks(&["a", "b"]), 0, false);
    assert!(!q.is_radio());
    assert_eq!(q.autoplay_start(), None);
}

#[test]
fn play_next_all_puts_the_tracks_right_after_the_current_one_in_order() {
    let mut q = Queue::new();
    q.replace(tracks(&["now", "a", "b"]), 0, false);
    q.play_next_all(tracks(&["x", "y", "z"]));
    assert_eq!(current_id(&q), Some("now"));
    assert_eq!(ids(q.upcoming()), ["x", "y", "z", "a", "b"]);
}

#[test]
fn play_next_all_on_an_empty_queue_plays_the_first() {
    let mut q = Queue::new();
    q.play_next_all(tracks(&["x", "y"]));
    assert_eq!(current_id(&q), Some("x"));
    assert_eq!(ids(q.upcoming()), ["y"]);
}
