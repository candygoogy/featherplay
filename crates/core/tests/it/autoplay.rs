//! What happens when a list (not a radio) runs out: autoplay continues into a radio of its
//! last track, or the list restarts from the top.

use std::time::{Duration, SystemTime};

use featherplay_core::model::{LikeStatus, RepeatMode, Track, TrackId, VideoType};
use featherplay_core::player::{Effect, Input, ListEnd, PlaybackPrefs, Player, Status, StreamInfo};

fn id(s: &str) -> TrackId {
    TrackId(s.to_string())
}

fn track(s: &str) -> Track {
    Track {
        id: id(s),
        title: format!("Title {s}"),
        artists: Vec::new(),
        album: None,
        duration: Some(Duration::from_secs(200)),
        video_type: VideoType::Song,
        thumbnail: None,
        like: LikeStatus::Indifferent,
        song_version: None,
    }
}

fn tracks(ids: &[&str]) -> Vec<Track> {
    ids.iter().map(|s| track(s)).collect()
}

fn prefs(list_end: ListEnd) -> PlaybackPrefs {
    PlaybackPrefs {
        prefer_song_versions: true,
        skip_non_music_in_radio: true,
        scrobble_other_media: true,
        list_end,
    }
}

fn ok(s: &str) -> Input {
    Input::Resolved {
        id: id(s),
        result: Ok(StreamInfo {
            expires_at: SystemTime::now() + Duration::from_secs(6 * 3600),
            duration: Some(Duration::from_secs(200)),
        }),
    }
}

fn list(ids: &[&str], start: usize) -> Input {
    Input::PlayTracks {
        tracks: tracks(ids),
        start,
        radio: false,
    }
}

fn fetches(fx: &[Effect]) -> Vec<String> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::FetchContinuation(id) => Some(id.0.clone()),
            _ => None,
        })
        .collect()
}

fn current(p: &Player) -> Option<String> {
    p.queue().current().map(|t| t.id.0.clone())
}

fn upcoming(p: &Player) -> Vec<String> {
    p.queue()
        .upcoming()
        .iter()
        .map(|t| t.id.0.clone())
        .collect()
}

/// A player in `list_end` mode, playing `ids` (a list) from `start`, audible.
fn playing(list_end: ListEnd, ids: &[&str], start: usize) -> (Player, Vec<Effect>) {
    let mut p = Player::new(prefs(list_end));
    let mut fx = p.handle(list(ids, start));
    fx.extend(p.handle(ok(ids[start])));
    fx.extend(p.handle(Input::AudioStarted));
    (p, fx)
}

#[test]
fn a_list_asks_for_autoplay_two_songs_before_its_end() {
    let (mut p, fx) = playing(ListEnd::Autoplay, &["a", "b", "c", "d"], 0);
    assert!(fetches(&fx).is_empty());
    let fx = p.handle(Input::Next);
    assert_eq!(fetches(&fx), ["d"]);
}

#[test]
fn a_list_asks_for_autoplay_only_once() {
    let (mut p, mut fx) = playing(ListEnd::Autoplay, &["a", "b", "c"], 0);
    fx.extend(p.handle(Input::Next));
    fx.extend(p.handle(Input::Previous));
    fx.extend(p.handle(Input::Next));
    assert_eq!(fetches(&fx), ["c"]);
}

#[test]
fn autoplay_follows_the_list_in_up_next() {
    let (mut p, _) = playing(ListEnd::Autoplay, &["a", "b", "c"], 2);
    p.handle(Input::ContinuationLoaded(id("c"), tracks(&["x", "y"])));
    assert_eq!(upcoming(&p), ["x", "y"]);
    assert_eq!(p.queue().autoplay_start(), Some(0));
}

#[test]
fn next_on_the_last_song_continues_into_autoplay() {
    let (mut p, _) = playing(ListEnd::Autoplay, &["a", "b", "c"], 2);
    p.handle(Input::ContinuationLoaded(id("c"), tracks(&["x", "y"])));
    p.handle(Input::Next);
    assert_eq!(current(&p).as_deref(), Some("x"));
}

#[test]
fn the_end_of_the_list_plays_on_into_autoplay() {
    let (mut p, _) = playing(ListEnd::Autoplay, &["a", "b", "c"], 2);
    p.handle(Input::ContinuationLoaded(id("c"), tracks(&["x", "y"])));
    p.handle(Input::AudioEnded);
    assert_eq!(current(&p).as_deref(), Some("x"));
}

#[test]
fn the_current_track_is_autoplay_once_the_list_is_left() {
    let (mut p, _) = playing(ListEnd::Autoplay, &["a", "b", "c"], 2);
    p.handle(Input::ContinuationLoaded(id("c"), tracks(&["x", "y"])));
    assert!(!p.queue().current_is_autoplay());
    p.handle(Input::Next);
    assert!(p.queue().current_is_autoplay());
}

#[test]
fn next_while_autoplay_is_on_its_way_waits_for_it() {
    let (mut p, _) = playing(ListEnd::Autoplay, &["a", "b", "c"], 2);
    p.handle(Input::Next);
    assert_eq!(p.status(), &Status::Loading);
    p.handle(Input::ContinuationLoaded(id("c"), tracks(&["x"])));
    assert_eq!(current(&p).as_deref(), Some("x"));
}

#[test]
fn a_failed_autoplay_leaves_the_end_as_it_was() {
    let (mut p, _) = playing(ListEnd::Autoplay, &["a", "b", "c"], 2);
    p.handle(Input::ContinuationLoaded(id("c"), Vec::new()));
    let fx = p.handle(Input::Next);
    assert_eq!((current(&p).as_deref(), fetches(&fx).len()), (Some("c"), 0));
}

#[test]
fn repeat_all_restarts_the_list_without_autoplay() {
    let mut p = Player::new(prefs(ListEnd::Autoplay));
    p.handle(Input::SetRepeat(RepeatMode::All));
    let fx = p.handle(list(&["a", "b", "c"], 2));
    assert!(fetches(&fx).is_empty());
}

#[test]
fn repeat_all_drops_the_autoplay_songs_not_played_yet() {
    let (mut p, _) = playing(ListEnd::Autoplay, &["a", "b", "c"], 2);
    p.handle(Input::ContinuationLoaded(id("c"), tracks(&["x", "y"])));
    p.handle(Input::SetRepeat(RepeatMode::All));
    assert_eq!(p.queue().peek_next().map(|t| t.id.0.as_str()), Some("a"));
}

#[test]
fn autoplay_turned_off_drops_the_autoplay_songs_not_played_yet() {
    let (mut p, _) = playing(ListEnd::Autoplay, &["a", "b", "c"], 2);
    p.handle(Input::ContinuationLoaded(id("c"), tracks(&["x", "y"])));
    p.handle(Input::SetPrefs(prefs(ListEnd::Restart)));
    assert!(upcoming(&p).is_empty());
}

#[test]
fn shuffle_asks_for_autoplay_after_the_last_song_it_will_play() {
    let mut p = Player::new(prefs(ListEnd::Autoplay));
    p.handle(Input::SetShuffle(true));
    let mut fx = p.handle(list(&["a", "b", "c", "d"], 0));
    let first = current(&p).unwrap_or_default();
    fx.extend(p.handle(ok(&first)));
    fx.extend(p.handle(Input::AudioStarted));
    fx.extend(p.handle(Input::Next));
    let last = upcoming(&p).last().cloned();
    assert_eq!(fetches(&fx), Vec::from_iter(last));
}

#[test]
fn a_radio_continues_whatever_the_list_end_setting() {
    let mut p = Player::new(prefs(ListEnd::Restart));
    let fx = p.handle(Input::PlayTracks {
        tracks: tracks(&["a", "b"]),
        start: 0,
        radio: true,
    });
    assert_eq!(fetches(&fx), ["b"]);
}

#[test]
fn restart_takes_next_on_the_last_song_to_the_top() {
    let (mut p, _) = playing(ListEnd::Restart, &["a", "b", "c"], 2);
    p.handle(Input::Next);
    assert_eq!(current(&p).as_deref(), Some("a"));
}

#[test]
fn restart_plays_the_list_again_from_the_top_at_its_end() {
    let (mut p, _) = playing(ListEnd::Restart, &["a", "b", "c"], 2);
    p.handle(Input::AudioEnded);
    assert_eq!((current(&p).as_deref(), p.wants_play()), (Some("a"), true));
}

#[test]
fn restart_never_asks_for_autoplay() {
    let (_, fx) = playing(ListEnd::Restart, &["a", "b", "c"], 2);
    assert!(fetches(&fx).is_empty());
}

#[test]
fn stop_keeps_the_last_song_at_the_end() {
    let (mut p, _) = playing(ListEnd::Stop, &["a", "b", "c"], 2);
    p.handle(Input::Next);
    assert_eq!(current(&p).as_deref(), Some("c"));
}
