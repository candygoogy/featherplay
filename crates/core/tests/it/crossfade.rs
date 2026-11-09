//! Crossfade: when the end of a song fades into the next, and when it cuts.

use std::time::{Duration, SystemTime};

use featherplay_core::model::{
    AlbumId, AlbumRef, LikeStatus, RepeatMode, Track, TrackId, VideoType,
};
use featherplay_core::player::{Effect, Input, ListEnd, PlaybackPrefs, Player, Status, StreamInfo};

const HOUR: Duration = Duration::from_secs(3600);

fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

fn id(s: &str) -> TrackId {
    TrackId(s.to_string())
}

/// A 200 s song, on album `album` if given.
fn track(s: &str, album: Option<&str>) -> Track {
    Track {
        id: id(s),
        title: format!("Title {s}"),
        artists: Vec::new(),
        album: album.map(|a| AlbumRef {
            id: AlbumId(a.to_string()),
            title: format!("Album {a}"),
        }),
        duration: Some(secs(200)),
        video_type: VideoType::Song,
        thumbnail: None,
        like: LikeStatus::Indifferent,
        counterpart: None,
    }
}

fn prefs(crossfade: Option<Duration>, fade_on_skip: bool) -> PlaybackPrefs {
    PlaybackPrefs {
        prefer_song_versions: true,
        skip_non_music_in_radio: true,
        scrobble_other_media: true,
        list_end: ListEnd::Stop,
        skip_disliked: true,
        crossfade,
        fade_on_skip,
        speed_percent: 100,
    }
}

fn t0() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000)
}

fn resolved(s: &str, length: Duration) -> Input {
    Input::Resolved {
        id: id(s),
        result: Ok(StreamInfo {
            expires_at: t0() + 6 * HOUR,
            duration: Some(length),
        }),
    }
}

fn tick(position: Duration) -> Input {
    Input::Tick {
        position,
        now: t0() + position,
        music_end: None,
    }
}

/// A tick that knows where the current song's music ends (the engine's tail scan).
fn tick_ending(position: Duration, music_end: Duration) -> Input {
    Input::Tick {
        position,
        now: t0() + position,
        music_end: Some(music_end),
    }
}

/// The effects that drive playback, without the UI refreshes and the saves.
fn run(p: &mut Player, input: Input) -> Vec<Effect> {
    p.handle(input)
        .into_iter()
        .filter(|e| !matches!(e, Effect::StateChanged | Effect::PersistSession))
        .collect()
}

/// `queue` playing from its first track, every song `length` long, the next one resolved
/// and buffered.
fn playing_with(prefs: PlaybackPrefs, queue: Vec<Track>, length: Duration) -> Player {
    let mut p = Player::new(prefs);
    let ids: Vec<String> = queue.iter().map(|t| t.id.0.clone()).collect();
    run(
        &mut p,
        Input::PlayTracks {
            tracks: queue,
            start: 0,
            radio: false,
        },
    );
    run(&mut p, resolved(&ids[0], length));
    let fx = run(&mut p, Input::AudioStarted);
    if let Some(next) = ids.get(1)
        && fx.contains(&Effect::Resolve(id(next)))
    {
        run(&mut p, resolved(next, length));
    }
    run(&mut p, tick(secs(1)));
    p
}

/// "a", "b" and "c", from three albums, crossfading over `fade`.
fn playing(fade: u64) -> Player {
    playing_with(
        prefs(Some(secs(fade)), false),
        vec![
            track("a", Some("x")),
            track("b", Some("y")),
            track("c", Some("z")),
        ],
        secs(200),
    )
}

fn crossfade(s: &str, fade: Duration) -> Effect {
    Effect::Crossfade { id: id(s), fade }
}

fn current(p: &Player) -> String {
    p.queue()
        .current()
        .map(|t| t.id.0.clone())
        .unwrap_or_default()
}

fn load(s: &str) -> Effect {
    Effect::Load {
        id: id(s),
        start: Duration::ZERO,
    }
}

#[test]
fn the_end_of_a_song_fades_into_the_next_one() {
    let mut p = playing(5);
    assert!(run(&mut p, tick(ms(194_900))).is_empty());
    let fx = run(&mut p, tick(secs(195)));
    assert_eq!(fx, [crossfade("b", secs(5))]);
    assert_eq!(current(&p), "b");
    // The sound never stops, so the player shows playing throughout.
    assert_eq!(p.status(), &Status::Playing);
}

#[test]
fn at_2x_a_fade_starts_twice_as_far_from_the_end_and_lasts_as_long() {
    let mut p = playing_with(
        PlaybackPrefs {
            speed_percent: 200,
            ..prefs(Some(secs(5)), false)
        },
        vec![track("a", Some("x")), track("b", Some("y"))],
        secs(200),
    );
    // 5 s of listening is 10 s of the song.
    assert!(run(&mut p, tick(ms(189_900))).is_empty());
    assert_eq!(run(&mut p, tick(secs(190))), [crossfade("b", secs(5))]);
}

#[test]
fn the_next_song_starts_as_usual_once_it_is_heard() {
    let mut p = playing(5);
    run(&mut p, tick(secs(195)));
    let fx = run(&mut p, Input::AudioStarted);
    assert!(fx.contains(&Effect::TrackStarted(track("b", Some("y")))));
    // And "c" is resolved ahead, for the next fade.
    assert!(fx.contains(&Effect::Resolve(id("c"))));
}

#[test]
fn the_old_songs_end_after_a_crossfade_does_not_move_on_again() {
    let mut p = playing(5);
    run(&mut p, tick(secs(195)));
    // The old song's stream ended before the engine took the fade over.
    assert!(run(&mut p, Input::AudioEnded).is_empty());
    assert_eq!(current(&p), "b");
    // Once "b" is heard, its own end moves on.
    run(&mut p, Input::AudioStarted);
    run(&mut p, resolved("c", secs(200)));
    run(&mut p, Input::AudioEnded);
    assert_eq!(current(&p), "c");
}

#[test]
fn a_song_whose_stream_ends_before_the_fade_moves_on_with_a_cut() {
    // Skip silence trimmed the end, so the stream ran out before the fade's tick.
    let mut p = playing(5);
    run(&mut p, tick(secs(190)));
    assert_eq!(run(&mut p, Input::AudioEnded), [load("b"), Effect::Play]);
    // The next tick fades nothing.
    assert!(run(&mut p, tick(secs(1))).is_empty());
    assert_eq!(current(&p), "b");
}

#[test]
fn the_fade_starts_where_the_music_ends_not_at_the_trailing_silence() {
    let mut p = playing(5);
    // The last 12 s are silent.
    assert!(run(&mut p, tick_ending(ms(182_900), secs(188))).is_empty());
    let fx = run(&mut p, tick_ending(secs(183), secs(188)));
    assert_eq!(fx, [crossfade("b", secs(5))]);
}

#[test]
fn a_seek_into_the_last_seconds_fades_over_what_is_left() {
    let mut p = playing(12);
    run(&mut p, Input::SeekTo(ms(197_500)));
    let fx = run(&mut p, tick(ms(197_500)));
    assert_eq!(fx, [crossfade("b", ms(2_500))]);
}

#[test]
fn past_the_music_the_next_song_fades_in_over_a_second() {
    let mut p = playing(12);
    let fx = run(&mut p, tick_ending(secs(195), secs(190)));
    assert_eq!(fx, [crossfade("b", secs(1))]);
}

#[test]
fn a_short_song_fades_over_half_its_length_at_most() {
    let mut p = playing_with(
        prefs(Some(secs(12)), false),
        vec![track("a", Some("x")), track("b", Some("y"))],
        secs(10),
    );
    assert!(run(&mut p, tick(ms(4_900))).is_empty());
    assert_eq!(run(&mut p, tick(secs(5))), [crossfade("b", secs(5))]);
}

#[test]
fn songs_from_the_same_album_play_on_without_a_fade() {
    let mut p = playing_with(
        prefs(Some(secs(5)), false),
        vec![track("a", Some("x")), track("b", Some("x"))],
        secs(200),
    );
    assert!(run(&mut p, tick(ms(199_750))).is_empty());
    assert_eq!(run(&mut p, Input::AudioEnded), [load("b"), Effect::Play]);
}

#[test]
fn songs_without_an_album_still_fade() {
    let mut p = playing_with(
        prefs(Some(secs(5)), false),
        vec![track("a", None), track("b", None)],
        secs(200),
    );
    assert_eq!(run(&mut p, tick(secs(195))), [crossfade("b", secs(5))]);
}

#[test]
fn repeat_one_starts_the_song_over_without_a_fade() {
    let mut p = playing(5);
    run(&mut p, Input::SetRepeat(RepeatMode::One));
    assert!(run(&mut p, tick(ms(199_750))).is_empty());
    assert_eq!(run(&mut p, Input::AudioEnded), [load("a"), Effect::Play]);
}

#[test]
fn the_last_song_of_the_queue_ends_without_a_fade() {
    let mut p = playing_with(
        prefs(Some(secs(5)), false),
        vec![track("a", Some("x"))],
        secs(200),
    );
    assert!(run(&mut p, tick(ms(199_750))).is_empty());
    run(&mut p, Input::AudioEnded);
    assert_eq!(p.status(), &Status::Idle);
}

#[test]
fn no_fade_while_the_next_song_is_still_resolving() {
    let mut p = Player::new(prefs(Some(secs(5)), false));
    run(
        &mut p,
        Input::PlayTracks {
            tracks: vec![track("a", Some("x")), track("b", Some("y"))],
            start: 0,
            radio: false,
        },
    );
    run(&mut p, resolved("a", secs(200)));
    run(&mut p, Input::AudioStarted);
    assert!(run(&mut p, tick(secs(197))).is_empty());
    // The end waits for the resolve already under way, then loads with a cut.
    assert!(run(&mut p, Input::AudioEnded).is_empty());
    assert_eq!(
        run(&mut p, resolved("b", secs(200))),
        [load("b"), Effect::Play]
    );
}

#[test]
fn crossfade_off_waits_for_the_end() {
    let mut p = playing_with(
        prefs(None, true),
        vec![track("a", Some("x")), track("b", Some("y"))],
        secs(200),
    );
    assert!(run(&mut p, tick(ms(199_750))).is_empty());
    assert_eq!(run(&mut p, Input::AudioEnded), [load("b"), Effect::Play]);
}

#[test]
fn paused_near_the_end_nothing_fades() {
    let mut p = playing(5);
    run(&mut p, tick(secs(190)));
    run(&mut p, Input::Toggle);
    assert!(run(&mut p, tick(secs(197))).is_empty());
    assert_eq!(current(&p), "a");
}

#[test]
fn next_previous_and_a_click_cut_by_default() {
    let mut p = playing(5);
    assert_eq!(
        run(&mut p, Input::Next),
        [Effect::Pause, load("b"), Effect::Play]
    );
    run(&mut p, Input::AudioStarted);
    assert_eq!(
        run(&mut p, Input::Previous),
        [Effect::Pause, load("a"), Effect::Play]
    );
    run(&mut p, Input::AudioStarted);
    assert_eq!(
        run(&mut p, Input::SkipTo(1)),
        [Effect::Pause, Effect::Resolve(id("c"))]
    );
}

/// "a", "b" and "c" crossfading over 5 s, with "Fade on skip" on.
fn fading_on_skip() -> Player {
    playing_with(
        prefs(Some(secs(5)), true),
        vec![
            track("a", Some("x")),
            track("b", Some("y")),
            track("c", Some("z")),
        ],
        secs(200),
    )
}

#[test]
fn fade_on_skip_fades_next_over_two_seconds_at_most() {
    let mut p = fading_on_skip();
    assert_eq!(
        run(&mut p, Input::Next),
        [Effect::FadeOut(secs(2)), crossfade("b", secs(2))]
    );
    assert_eq!(p.status(), &Status::Playing);
}

#[test]
fn fade_on_skip_is_never_longer_than_the_crossfade() {
    let mut p = playing_with(
        prefs(Some(secs(2)), true),
        vec![track("a", Some("x")), track("b", Some("y"))],
        secs(200),
    );
    assert_eq!(
        run(&mut p, Input::Next),
        [Effect::FadeOut(secs(2)), crossfade("b", secs(2))]
    );
}

#[test]
fn fade_on_skip_fades_back_to_the_previous_song() {
    let mut p = fading_on_skip();
    run(&mut p, Input::Next);
    run(&mut p, Input::AudioStarted);
    assert_eq!(
        run(&mut p, Input::Previous),
        [Effect::FadeOut(secs(2)), crossfade("a", secs(2))]
    );
}

#[test]
fn fade_on_skip_restarting_the_song_is_a_seek() {
    let mut p = fading_on_skip();
    run(&mut p, tick(secs(30)));
    assert_eq!(run(&mut p, Input::Previous), [Effect::Seek(Duration::ZERO)]);
}

#[test]
fn fade_on_skip_fades_out_now_and_in_once_a_clicked_song_resolves() {
    let mut p = fading_on_skip();
    assert_eq!(
        run(&mut p, Input::SkipTo(1)),
        [Effect::FadeOut(secs(2)), Effect::Resolve(id("c"))]
    );
    assert_eq!(p.status(), &Status::Loading);
    assert_eq!(
        run(&mut p, resolved("c", secs(200))),
        [crossfade("c", secs(2))]
    );
}

#[test]
fn a_stop_after_a_faded_skip_leaves_no_fade_for_the_next_play() {
    let mut p = fading_on_skip();
    run(&mut p, Input::SkipTo(1));
    // The helpers can't be fetched: playback stops on "c".
    run(
        &mut p,
        Input::Resolved {
            id: id("c"),
            result: Err(featherplay_core::failure::Failure {
                kind: featherplay_core::failure::FailKind::Setup(
                    featherplay_core::failure::HelperFault::Unreachable,
                ),
                detail: "no helpers".into(),
            }),
        },
    );
    assert!(matches!(p.status(), Status::Stopped(_)));
    assert_eq!(run(&mut p, Input::Toggle), [Effect::Resolve(id("c"))]);
    assert_eq!(
        run(&mut p, resolved("c", secs(200))),
        [load("c"), Effect::Play]
    );
}

#[test]
fn previous_during_a_crossfade_goes_back_to_the_old_song() {
    let mut p = playing(5);
    run(&mut p, tick(secs(195)));
    run(&mut p, Input::AudioStarted);
    run(&mut p, tick(secs(1)));
    assert_eq!(
        run(&mut p, Input::Previous),
        [Effect::Pause, load("a"), Effect::Play]
    );
    assert_eq!(current(&p), "a");
}

#[test]
fn fade_on_skip_cuts_while_paused() {
    let mut p = fading_on_skip();
    run(&mut p, Input::Toggle);
    assert_eq!(
        run(&mut p, Input::Next),
        [Effect::Pause, load("b"), Effect::Play]
    );
}

#[test]
fn fade_on_skip_does_nothing_with_crossfade_off() {
    let mut p = playing_with(
        prefs(None, true),
        vec![track("a", Some("x")), track("b", Some("y"))],
        secs(200),
    );
    assert_eq!(
        run(&mut p, Input::Next),
        [Effect::Pause, load("b"), Effect::Play]
    );
}
