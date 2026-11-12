//! Songs YouTube refuses (`FailKind::Refused`): the player passes over them from then on and
//! plays on, says so once a queue, and still tries one the user starts directly.

use std::time::{Duration, SystemTime};

use featherplay_core::failure::{FailKind, Failure, StopCause};
use featherplay_core::model::{LikeStatus, Track, TrackId, VideoType};
use featherplay_core::player::{Effect, Input, PlaybackPrefs, Player, Status, StreamInfo};

fn id(s: &str) -> TrackId {
    TrackId(s.to_string())
}

fn tracks(ids: &[&str]) -> Vec<Track> {
    ids.iter()
        .map(|s| Track {
            id: id(s),
            title: format!("Title {s}"),
            artists: Vec::new(),
            album: None,
            duration: Some(Duration::from_secs(200)),
            video_type: VideoType::Song,
            thumbnail: None,
            like: LikeStatus::Indifferent,
            counterpart: None,
        })
        .collect()
}

fn player() -> Player {
    Player::new(PlaybackPrefs {
        prefer_song_versions: true,
        skip_non_music_in_radio: true,
        scrobble_other_media: true,
        list_end: Default::default(),
        skip_disliked: true,
        crossfade: None,
        fade_on_skip: false,
        speed_percent: 100,
    })
}

fn play(p: &mut Player, ids: &[&str], start: usize) -> Vec<Effect> {
    run(
        p,
        Input::PlayTracks {
            tracks: tracks(ids),
            start,
            radio: false,
        },
    )
}

fn run(p: &mut Player, input: Input) -> Vec<Effect> {
    p.handle(input)
        .into_iter()
        .filter(|e| *e != Effect::StateChanged)
        .collect()
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

fn refused(s: &str) -> Input {
    Input::Resolved {
        id: id(s),
        result: Err(Failure {
            kind: FailKind::Refused,
            detail: format!("unavailable: This video is not available ({s})"),
        }),
    }
}

fn resolves(fx: &[Effect]) -> Vec<&str> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Resolve(id) => Some(id.0.as_str()),
            _ => None,
        })
        .collect()
}

fn told(fx: &[Effect]) -> usize {
    fx.iter().filter(|e| **e == Effect::SongsRefused).count()
}

fn stopped(fx: &[Effect]) -> Option<StopCause> {
    fx.iter().find_map(|e| match e {
        Effect::ShowError(stop) => Some(stop.cause),
        _ => None,
    })
}

fn current(p: &Player) -> &str {
    p.queue().current().map_or("", |t| t.id.0.as_str())
}

/// More refused songs in a row than the stop after failures counts: each is skipped on to
/// the next, and the toast comes once.
#[test]
fn refused_songs_are_skipped_and_said_once() {
    let mut p = player();
    play(&mut p, &["a", "b", "c", "d", "e"], 0);
    let mut toasts = 0;
    for (s, next) in [("a", "b"), ("b", "c"), ("c", "d"), ("d", "e")] {
        let fx = run(&mut p, refused(s));
        assert_eq!(stopped(&fx), None, "stopped at {s}");
        assert_eq!(resolves(&fx), [next]);
        toasts += told(&fx);
    }
    assert_eq!(toasts, 1);
    run(&mut p, ok("e"));
    run(&mut p, Input::AudioStarted);
    assert_eq!(p.status(), &Status::Playing);
}

/// Once refused, a song is passed over without a resolve: by the preload, the end of the
/// song before it, Next, and the queue played again (the toast comes again for it).
#[test]
fn a_refused_song_is_passed_over_without_a_resolve() {
    let mut p = player();
    play(&mut p, &["a", "b", "c"], 0);
    run(&mut p, Input::SetRefused(id("b"), true));
    run(&mut p, ok("a"));
    let fx = run(&mut p, Input::AudioStarted);
    assert_eq!(resolves(&fx), ["c"], "the preload passes over it");
    assert_eq!(p.ahead(2), [id("c")]);
    let fx = run(&mut p, Input::AudioEnded);
    assert_eq!(current(&p), "c");
    assert_eq!(told(&fx), 1);

    // Back from the start, by Next.
    play(&mut p, &["a", "b", "c"], 0);
    run(&mut p, ok("a"));
    run(&mut p, Input::AudioStarted);
    let fx = run(&mut p, Input::Next);
    assert_eq!(current(&p), "c");
    assert_eq!(told(&fx), 1, "a new queue is told again");
    // It played after all (a click): the queue has it again.
    run(&mut p, Input::SetRefused(id("b"), false));
    play(&mut p, &["a", "b", "c"], 0);
    assert_eq!(p.ahead(2), [id("b"), id("c")]);
}

/// A click on a refused song tries it once: it's the queue's start.
#[test]
fn a_refused_song_clicked_is_tried() {
    let mut p = player();
    run(&mut p, Input::SetRefused(id("b"), true));
    let fx = play(&mut p, &["a", "b", "c"], 1);
    assert_eq!(resolves(&fx), ["b"]);
    run(&mut p, ok("b"));
    run(&mut p, Input::AudioStarted);
    assert_eq!(p.status(), &Status::Playing);
}

/// The last song refused ends the queue as a song that's gone does.
#[test]
fn the_last_song_refused_stops_as_unavailable() {
    let mut p = player();
    play(&mut p, &["a"], 0);
    let fx = run(&mut p, refused("a"));
    assert_eq!(stopped(&fx), Some(StopCause::Unavailable));
    assert_eq!(told(&fx), 0, "the stop's own words");
}

/// So many refused in a row that YouTube likely refuses every song for now: it stops, so a
/// radio can't fetch continuation after continuation.
#[test]
fn ten_refused_in_a_row_stop() {
    let ids: Vec<String> = (0..12).map(|i| format!("s{i}")).collect();
    let ids: Vec<&str> = ids.iter().map(String::as_str).collect();
    let mut p = player();
    play(&mut p, &ids, 0);
    for s in &ids[..9] {
        assert_eq!(stopped(&run(&mut p, refused(s))), None, "stopped at {s}");
    }
    assert_eq!(
        stopped(&run(&mut p, refused(ids[9]))),
        Some(StopCause::Unavailable)
    );
}
