//! "Not for me" (the ⊘ beside the heart): it rates the song Dislike and skips it; Undo puts
//! the rating back and returns to the song where it was. The actions run on a real player,
//! as the controller runs them.

use std::time::Duration;

use featherplay::controller::{Action, back_to_inputs};
use featherplay::not_for_me;
use featherplay_core::model::{LikeStatus, Track, TrackId, VideoType};
use featherplay_core::player::{Input, PlaybackPrefs, Player};

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
        song_version: None,
    }
}

fn player_with(ids: &[&str]) -> Player {
    let mut player = Player::new(PlaybackPrefs {
        prefer_song_versions: true,
        skip_non_music_in_radio: true,
        scrobble_other_media: false,
        list_end: Default::default(),
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
