//! The song menu's Play next and Add to queue reach the player as the Up next panel's edits.

use std::time::Duration;

use featherplay_controller::controller::{Action, QueueAction, add_input, queue_edit};
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

fn upcoming(player: &Player) -> Vec<String> {
    player
        .queue()
        .upcoming()
        .iter()
        .map(|t| t.id.0.clone())
        .collect()
}

/// What the controller does with a song menu action.
fn send(player: &mut Player, action: Action) {
    let edit = queue_edit(action).expect("a queue edit");
    let input = add_input(edit).expect("a player input");
    player.handle(input);
}

/// Each edit on "now", then the rest of the list: the current track stays, and what comes
/// after it is the row's.
#[test]
fn each_queue_edit_lands_in_its_place() {
    type Edit = fn(&mut Player);
    let cases: [(&str, &[&str], Edit, &[&str]); 3] = [
        (
            "play next goes right after the current track",
            &["now", "a", "b"],
            |p| send(p, Action::PlayNext(track("x"))),
            &["x", "a", "b"],
        ),
        (
            "add to queue goes to the end",
            &["now", "a", "b"],
            |p| send(p, Action::AddToQueue(track("x"))),
            &["a", "b", "x"],
        ),
        // The sidebar playlist menu's Play next: the whole playlist, in order.
        (
            "a playlist's play next goes after the current track in order",
            &["now", "a"],
            |p| {
                let edit = QueueAction::PlayNextAll(vec![track("x"), track("y")]);
                p.handle(add_input(edit).expect("a player input"));
            },
            &["x", "y", "a"],
        ),
    ];
    for (label, ids, edit, want) in cases {
        let mut player = player_with(ids);
        edit(&mut player);
        assert_eq!(
            player.queue().current().map(|t| t.id.0.as_str()),
            Some("now"),
            "{label}"
        );
        assert_eq!(upcoming(&player), want, "{label}");
    }
}

#[test]
fn other_actions_are_not_queue_edits() {
    assert!(queue_edit(Action::Toggle).is_none());
    assert!(add_input(QueueAction::Clear).is_none());
}

/// The quick search's Play next, picked twice: the second goes after the first, so they play
/// in the order picked; once the first has played, the next pick goes right after the current
/// track again.
#[test]
fn quick_search_picks_play_next_in_the_order_picked() {
    use featherplay_controller::controller::play_after_inputs;
    let mut player = player_with(&["now", "a", "b"]);
    for input in play_after_inputs(player.queue(), vec![track("x")], None) {
        player.handle(input);
    }
    let x = TrackId("x".into());
    let two = vec![track("y"), track("z")];
    for input in play_after_inputs(player.queue(), two, Some(&x)) {
        player.handle(input);
    }
    assert_eq!(upcoming(&player), ["x", "y", "z", "a", "b"]);
    // `x` has played: what comes after it goes right after the current track.
    let mut player = player_with(&["now", "a"]);
    for input in play_after_inputs(player.queue(), vec![track("w")], Some(&x)) {
        player.handle(input);
    }
    assert_eq!(upcoming(&player), ["w", "a"]);
}
