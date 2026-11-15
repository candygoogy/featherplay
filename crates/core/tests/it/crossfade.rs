//! Crossfade: when the end of a song fades into the next, and when it cuts.

use std::time::Duration;

use featherplay_core::model::{AlbumId, AlbumRef, RepeatMode, Track};
use featherplay_core::player::{Effect, Input, PlaybackPrefs, Player, Status, StreamInfo};

use crate::support::{current, id, t0};

const HOUR: Duration = Duration::from_secs(3600);

fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// A 200 s song, on album `album` if given.
fn track(s: &str, album: Option<&str>) -> Track {
    Track {
        album: album.map(|a| AlbumRef {
            id: AlbumId(a.to_string()),
            title: format!("Album {a}"),
        }),
        ..crate::support::track(s)
    }
}

fn prefs(crossfade: Option<Duration>, fade_on_skip: bool) -> PlaybackPrefs {
    PlaybackPrefs {
        crossfade,
        fade_on_skip,
        ..crate::support::prefs()
    }
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

fn load(s: &str) -> Effect {
    Effect::Load {
        id: id(s),
        start: Duration::ZERO,
    }
}

/// Where the end of a song starts to fade into the next one, and over how long: a tick just
/// before is quiet, the tick at that point gives the crossfade.
#[test]
fn the_end_of_a_song_fades_into_the_next_one() {
    struct Case {
        case: &'static str,
        fade: u64,
        speed_percent: u16,
        /// The queue's albums: "a" on the first, "b" on the second, and so on.
        albums: &'static [Option<&'static str>],
        length: Duration,
        /// Where the tail scan says the music ends.
        music_end: Option<Duration>,
        seek: Option<Duration>,
        quiet_at: Option<Duration>,
        fade_at: Duration,
        want: Duration,
    }
    let three = &[Some("x"), Some("y"), Some("z")];
    let two = &[Some("x"), Some("y")];
    let cases = [
        Case {
            case: "the fade's length before the end",
            fade: 5,
            speed_percent: 100,
            albums: three,
            length: secs(200),
            music_end: None,
            seek: None,
            quiet_at: Some(ms(194_900)),
            fade_at: secs(195),
            want: secs(5),
        },
        Case {
            case: "at 2x, twice as far from the end and as long",
            fade: 5,
            speed_percent: 200,
            albums: two,
            length: secs(200),
            music_end: None,
            seek: None,
            quiet_at: Some(ms(189_900)),
            fade_at: secs(190),
            want: secs(5),
        },
        Case {
            case: "where the music ends, not at the trailing silence",
            fade: 5,
            speed_percent: 100,
            albums: three,
            length: secs(200),
            music_end: Some(secs(188)),
            seek: None,
            quiet_at: Some(ms(182_900)),
            fade_at: secs(183),
            want: secs(5),
        },
        Case {
            case: "a seek into the last seconds fades over what is left",
            fade: 12,
            speed_percent: 100,
            albums: three,
            length: secs(200),
            music_end: None,
            seek: Some(ms(197_500)),
            quiet_at: None,
            fade_at: ms(197_500),
            want: ms(2_500),
        },
        Case {
            case: "past the music, the next song fades in over a second",
            fade: 12,
            speed_percent: 100,
            albums: three,
            length: secs(200),
            music_end: Some(secs(190)),
            seek: None,
            quiet_at: None,
            fade_at: secs(195),
            want: secs(1),
        },
        Case {
            case: "a short song fades over half its length at most",
            fade: 12,
            speed_percent: 100,
            albums: two,
            length: secs(10),
            music_end: None,
            seek: None,
            quiet_at: Some(ms(4_900)),
            fade_at: secs(5),
            want: secs(5),
        },
        Case {
            case: "songs without an album still fade",
            fade: 5,
            speed_percent: 100,
            albums: &[None, None],
            length: secs(200),
            music_end: None,
            seek: None,
            quiet_at: None,
            fade_at: secs(195),
            want: secs(5),
        },
    ];
    for c in cases {
        let case = c.case;
        let queue = ["a", "b", "c"]
            .into_iter()
            .zip(c.albums)
            .map(|(s, album)| track(s, *album))
            .collect();
        let prefs = PlaybackPrefs {
            speed_percent: c.speed_percent,
            ..prefs(Some(secs(c.fade)), false)
        };
        let mut p = playing_with(prefs, queue, c.length);
        let at = |position| match c.music_end {
            Some(end) => tick_ending(position, end),
            None => tick(position),
        };
        if let Some(to) = c.seek {
            run(&mut p, Input::SeekTo(to));
        }
        if let Some(quiet) = c.quiet_at {
            assert_eq!(run(&mut p, at(quiet)), [], "{case}: before the fade");
        }
        assert_eq!(
            run(&mut p, at(c.fade_at)),
            [crossfade("b", c.want)],
            "{case}"
        );
        assert_eq!(current(&p).as_deref(), Some("b"), "{case}");
        // The sound never stops, so the player shows playing throughout.
        assert_eq!(p.status(), &Status::Playing, "{case}");
    }
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
    assert_eq!(current(&p).as_deref(), Some("b"));
    // Once "b" is heard, its own end moves on.
    run(&mut p, Input::AudioStarted);
    run(&mut p, resolved("c", secs(200)));
    run(&mut p, Input::AudioEnded);
    assert_eq!(current(&p).as_deref(), Some("c"));
}

#[test]
fn a_song_whose_stream_ends_before_the_fade_moves_on_with_a_cut() {
    // Skip silence trimmed the end, so the stream ran out before the fade's tick.
    let mut p = playing(5);
    run(&mut p, tick(secs(190)));
    assert_eq!(run(&mut p, Input::AudioEnded), [load("b"), Effect::Play]);
    // The next tick fades nothing.
    assert!(run(&mut p, tick(secs(1))).is_empty());
    assert_eq!(current(&p).as_deref(), Some("b"));
}

/// Where the end of a song doesn't fade: the last tick before it is quiet, and the end
/// loads the next song with a cut (or stops at the end of the queue).
#[test]
fn some_ends_cut_without_a_fade() {
    // (case, crossfade, the queue's albums, repeat, what the end loads; None: it stops)
    type Case = (
        &'static str,
        Option<u64>,
        &'static [Option<&'static str>],
        Option<RepeatMode>,
        Option<&'static str>,
    );
    let cases: [Case; 4] = [
        (
            "songs from the same album play on",
            Some(5),
            &[Some("x"), Some("x")],
            None,
            Some("b"),
        ),
        (
            "Repeat One starts the song over",
            Some(5),
            &[Some("x"), Some("y"), Some("z")],
            Some(RepeatMode::One),
            Some("a"),
        ),
        (
            "the last song of the queue ends",
            Some(5),
            &[Some("x")],
            None,
            None,
        ),
        (
            "crossfade off waits for the end",
            None,
            &[Some("x"), Some("y")],
            None,
            Some("b"),
        ),
    ];
    for (case, fade, albums, repeat, next) in cases {
        let queue = ["a", "b", "c"]
            .into_iter()
            .zip(albums)
            .map(|(s, album)| track(s, *album))
            .collect();
        // Crossfade off with "Fade on skip" on: the end still cuts.
        let mut p = playing_with(prefs(fade.map(secs), fade.is_none()), queue, secs(200));
        if let Some(mode) = repeat {
            run(&mut p, Input::SetRepeat(mode));
        }
        assert_eq!(run(&mut p, tick(ms(199_750))), [], "{case}");
        let fx = run(&mut p, Input::AudioEnded);
        match next {
            Some(s) => assert_eq!(fx, [load(s), Effect::Play], "{case}"),
            None => assert_eq!(p.status(), &Status::Idle, "{case}"),
        }
    }
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
fn paused_near_the_end_nothing_fades() {
    let mut p = playing(5);
    run(&mut p, tick(secs(190)));
    run(&mut p, Input::Toggle);
    assert!(run(&mut p, tick(secs(197))).is_empty());
    assert_eq!(current(&p).as_deref(), Some("a"));
}

/// Next cuts by default, while paused with "Fade on skip" on, and with "Fade on skip" on but
/// crossfade off; by default Previous and a click cut too.
#[test]
fn a_skip_cuts_unless_it_fades() {
    let cases = [
        ("by default", playing(5), false),
        ("fade on skip, paused", fading_on_skip(), true),
        (
            "fade on skip, crossfade off",
            playing_with(
                prefs(None, true),
                vec![track("a", Some("x")), track("b", Some("y"))],
                secs(200),
            ),
            false,
        ),
    ];
    for (case, mut p, paused) in cases {
        if paused {
            run(&mut p, Input::Toggle);
        }
        assert_eq!(
            run(&mut p, Input::Next),
            [Effect::Pause, load("b"), Effect::Play],
            "{case}"
        );
    }

    let mut p = playing(5);
    run(&mut p, Input::Next);
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
    assert_eq!(current(&p).as_deref(), Some("a"));
}

#[test]
fn the_sleep_timers_song_end_pauses_on_the_next_song_without_a_fade() {
    let mut p = playing(5);
    run(&mut p, Input::PauseAtEnd(true));
    assert!(p.pauses_at_end());
    // Where it would fade, it plays on to the end.
    assert!(!run(&mut p, tick(secs(196))).contains(&crossfade("b", secs(5))));
    assert_eq!(current(&p).as_deref(), Some("a"));
    let fx = run(&mut p, Input::AudioEnded);
    assert_eq!(current(&p).as_deref(), Some("b"));
    assert!(fx.contains(&load("b")), "{fx:?}");
    assert!(!fx.contains(&Effect::Play), "{fx:?}");
    assert!(!p.wants_play() && !p.pauses_at_end());
}

#[test]
fn the_sleep_timers_song_end_once_turned_off_plays_on() {
    let mut p = playing_with(
        prefs(None, false),
        vec![track("a", Some("x")), track("b", Some("y"))],
        secs(200),
    );
    run(&mut p, Input::PauseAtEnd(true));
    run(&mut p, Input::PauseAtEnd(false));
    run(&mut p, Input::AudioEnded);
    assert_eq!(current(&p).as_deref(), Some("b"));
    assert!(p.wants_play());
}
