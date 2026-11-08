//! "Not for me" (the thumbs-down beside the heart): it rates the song Dislike and skips it;
//! Undo puts the rating back and returns to the song where it was. The actions run on a real
//! player, as the controller runs them.

use std::collections::HashMap;
use std::time::{Duration, SystemTime};

use featherplay::controller::{Action, back_to_inputs, with_likes};
use featherplay::not_for_me;
use featherplay_core::model::{LikeStatus, Track, TrackId, VideoType};
use featherplay_core::player::{Input, PlaybackPrefs, Player, StreamInfo};

fn track(id: &str) -> Track {
    Track {
        id: TrackId(id.into()),
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

fn player_with(ids: &[&str]) -> Player {
    let mut player = Player::new(PlaybackPrefs {
        prefer_song_versions: true,
        skip_non_music_in_radio: true,
        scrobble_other_media: false,
        list_end: Default::default(),
        skip_disliked: true,
        crossfade: None,
        fade_on_skip: false,
        speed_percent: 100,
    });
    player.handle(Input::PlayTracks {
        tracks: ids.iter().map(|id| track(id)).collect(),
        start: 0,
        radio: false,
    });
    player
}

fn current(player: &Player) -> Option<&str> {
    player.queue().current().map(|t| t.id.0.as_str())
}

fn upcoming(player: &Player) -> Vec<&str> {
    player
        .queue()
        .upcoming()
        .iter()
        .map(|t| t.id.0.as_str())
        .collect()
}

/// What the controller does with the actions that move the player.
fn run(player: &mut Player, actions: Vec<Action>) {
    for action in actions {
        match action {
            Action::Next => {
                player.handle(Input::Next);
            }
            Action::BackTo { track, at } => {
                for input in back_to_inputs(player.queue(), &track, at) {
                    player.handle(input);
                }
            }
            _ => {}
        }
    }
}

const AT: Duration = Duration::from_secs(83);

#[test]
fn not_for_me_dislikes_the_song_then_skips_it() {
    let mut player = player_with(&["a", "b", "c"]);
    let (actions, _) = not_for_me::actions(&track("a"), AT);
    assert!(
        matches!(
            &actions[..],
            [Action::Rate(id, LikeStatus::Dislike), Action::Next] if id.0 == "a"
        ),
        "{actions:?}"
    );
    run(&mut player, actions);
    assert_eq!(current(&player), Some("b"));
}

#[test]
fn a_song_already_disliked_is_only_skipped_and_undo_writes_no_rating() {
    let mut player = player_with(&["a", "b", "c"]);
    let mut disliked = track("a");
    disliked.like = LikeStatus::Dislike;
    let (actions, undo) = not_for_me::actions(&disliked, AT);
    assert!(matches!(&actions[..], [Action::Next]), "{actions:?}");
    run(&mut player, actions);
    assert_eq!(current(&player), Some("b"));

    let back = undo.actions();
    assert!(matches!(&back[..], [Action::BackTo { .. }]), "{back:?}");
    run(&mut player, back);
    assert_eq!(current(&player), Some("a"));
}

#[test]
fn undo_puts_the_like_back_and_returns_to_the_song_where_it_was() {
    let mut player = player_with(&["a", "b", "c"]);
    player.handle(Input::SeekTo(AT));
    let mut liked = track("a");
    liked.like = LikeStatus::Like;
    let (actions, undo) = not_for_me::actions(&liked, AT);
    run(&mut player, actions);

    let back = undo.actions();
    assert!(
        matches!(&back[0], Action::Rate(id, LikeStatus::Like) if id.0 == "a"),
        "{back:?}"
    );
    run(&mut player, back);
    assert_eq!(current(&player), Some("a"));
    assert_eq!(player.position(), AT);
    assert_eq!(upcoming(&player), ["b", "c"]);
}

#[test]
fn undo_after_moving_on_plays_the_song_again_where_it_was() {
    let mut player = player_with(&["a", "b", "c"]);
    let (actions, undo) = not_for_me::actions(&track("a"), AT);
    run(&mut player, actions);
    // The user skipped once more before pressing Undo.
    player.handle(Input::Next);

    let back = undo.actions();
    assert!(
        matches!(&back[0], Action::Rate(id, LikeStatus::Indifferent) if id.0 == "a"),
        "{back:?}"
    );
    run(&mut player, back);
    assert_eq!(current(&player), Some("a"));
    assert_eq!(player.position(), AT);
}

/// As `run`, with the rating's write going through: the queue and the session's ratings
/// take it, as the controller does on `Msg::Rated`.
fn run_rated(player: &mut Player, likes: &mut HashMap<TrackId, LikeStatus>, actions: Vec<Action>) {
    for action in actions {
        match action {
            Action::Rate(id, status) => {
                player.handle(Input::SetLike(id.clone(), status));
                likes.insert(id, status);
            }
            Action::Next => {
                player.handle(Input::Next);
            }
            _ => {}
        }
    }
}

/// The current track `id` loads, starts and plays to its end.
fn play_to_the_end(player: &mut Player, id: &str) {
    player.handle(Input::Resolved {
        id: TrackId(id.into()),
        result: Ok(StreamInfo {
            expires_at: SystemTime::now() + Duration::from_secs(6 * 3600),
            duration: Some(Duration::from_secs(200)),
        }),
    });
    player.handle(Input::AudioStarted);
    player.handle(Input::AudioEnded);
}

/// A list's page plays its tracks as they were fetched, before any rating made since.
fn play_page(player: &mut Player, likes: &HashMap<TrackId, LikeStatus>, ids: &[&str]) {
    let play = Input::PlayTracks {
        tracks: ids.iter().map(|id| track(id)).collect(),
        start: 0,
        radio: false,
    };
    player.handle(with_likes(play, likes));
}

#[test]
fn a_song_set_aside_stays_passed_over_when_its_list_is_played_again() {
    // "Not for me" on the second song, then the first one picked again on the playlist's page.
    let mut player = player_with(&["a", "b", "c"]);
    let mut likes = HashMap::new();
    player.handle(Input::Next);
    let (actions, _) = not_for_me::actions(&track("b"), AT);
    run_rated(&mut player, &mut likes, actions);
    play_page(&mut player, &likes, &["a", "b", "c"]);

    play_to_the_end(&mut player, "a");
    assert_eq!(current(&player), Some("c"));
}

#[test]
fn a_song_set_aside_still_plays_when_picked_on_its_page() {
    let mut player = player_with(&["a", "b", "c"]);
    let mut likes = HashMap::new();
    let (actions, _) = not_for_me::actions(&track("a"), AT);
    run_rated(&mut player, &mut likes, actions);
    play_page(&mut player, &likes, &["a", "b", "c"]);

    assert_eq!(current(&player), Some("a"));
    assert_eq!(
        player.queue().current().map(|t| t.like),
        Some(LikeStatus::Dislike)
    );
}
