use std::time::{Duration, SystemTime};

use featherplay_core::failure::{FailKind, Failure, StopCause};
use featherplay_core::model::{LikeStatus, RepeatMode, Session, Track, TrackId, VideoType};
use featherplay_core::player::{Effect, Input, PlaybackPrefs, Player, Status, Stop, StreamInfo};

const HOUR: Duration = Duration::from_secs(3600);

fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

fn id(s: &str) -> TrackId {
    TrackId(s.to_string())
}

fn track(s: &str) -> Track {
    typed(s, VideoType::Song)
}

fn typed(s: &str, video_type: VideoType) -> Track {
    Track {
        id: id(s),
        title: format!("Title {s}"),
        artists: Vec::new(),
        album: None,
        duration: Some(secs(200)),
        video_type,
        thumbnail: None,
        like: LikeStatus::Indifferent,
        song_version: None,
    }
}

fn tracks(ids: &[&str]) -> Vec<Track> {
    ids.iter().map(|s| track(s)).collect()
}

fn prefs(prefer_song_versions: bool, skip_non_music_in_radio: bool) -> PlaybackPrefs {
    PlaybackPrefs {
        prefer_song_versions,
        skip_non_music_in_radio,
        scrobble_other_media: true,
        list_end: Default::default(),
        skip_disliked: true,
        crossfade: None,
        fade_on_skip: false,
    }
}

fn t0() -> SystemTime {
    SystemTime::UNIX_EPOCH + Duration::from_secs(1_800_000_000)
}

fn info(expires_at: SystemTime) -> StreamInfo {
    StreamInfo {
        expires_at,
        duration: Some(secs(200)),
    }
}

fn ok(s: &str, expires_at: SystemTime) -> Input {
    Input::Resolved {
        id: id(s),
        result: Ok(info(expires_at)),
    }
}

fn err(s: &str) -> Input {
    failed(s, FailKind::Unavailable)
}

fn failed(s: &str, kind: FailKind) -> Input {
    Input::Resolved {
        id: id(s),
        result: Err(Failure {
            kind,
            detail: format!("{kind:?} {s}"),
        }),
    }
}

/// The `ShowError` among `fx`.
fn stop_of(fx: &[Effect]) -> Option<&Stop> {
    fx.iter().find_map(|e| match e {
        Effect::ShowError(stop) => Some(stop),
        _ => None,
    })
}

fn tick(position: Duration, now: SystemTime) -> Input {
    Input::Tick {
        position,
        now,
        music_end: None,
    }
}

/// The effects that drive playback; `StateChanged` only tells the UI to refresh.
fn run(p: &mut Player, input: Input) -> Vec<Effect> {
    p.handle(input)
        .into_iter()
        .filter(|e| *e != Effect::StateChanged)
        .collect()
}

/// A player with `ids` queued (not radio), the first track resolved, loaded and audible.
fn playing(ids: &[&str]) -> Player {
    let mut p = Player::new(prefs(true, true));
    run(
        &mut p,
        Input::PlayTracks {
            tracks: tracks(ids),
            start: 0,
            radio: false,
        },
    );
    run(&mut p, ok(ids[0], t0() + 6 * HOUR));
    run(&mut p, Input::AudioStarted);
    p
}

#[test]
fn play_tracks_resolves_then_loads() {
    let mut p = Player::new(prefs(true, true));
    let fx = run(
        &mut p,
        Input::PlayTracks {
            tracks: tracks(&["a", "b"]),
            start: 0,
            radio: false,
        },
    );
    assert_eq!(fx, [Effect::Resolve(id("a"))]);
    assert_eq!(p.status(), &Status::Loading);

    let fx = run(&mut p, ok("a", t0() + 6 * HOUR));
    assert_eq!(
        fx,
        [
            Effect::Load {
                id: id("a"),
                start: Duration::ZERO
            },
            Effect::Play
        ]
    );

    // The next track is resolved as soon as this one is audible.
    let fx = run(&mut p, Input::AudioStarted);
    assert_eq!(
        fx,
        [Effect::TrackStarted(track("a")), Effect::Resolve(id("b"))]
    );
    assert_eq!(p.status(), &Status::Playing);
}

#[test]
fn preload_once_when_the_track_starts() {
    let mut p = Player::new(prefs(true, true));
    run(
        &mut p,
        Input::PlayTracks {
            tracks: tracks(&["a", "b", "c"]),
            start: 0,
            radio: false,
        },
    );
    run(&mut p, ok("a", t0() + 6 * HOUR));
    let fx = run(&mut p, Input::AudioStarted);
    assert!(fx.contains(&Effect::Resolve(id("b"))));
    let fx = run(&mut p, ok("b", t0() + 6 * HOUR));
    assert_eq!(fx, [Effect::Preload(id("b"))]);

    // Once per track: ticks up to the end don't resolve it again.
    for i in 0..200 {
        let fx = run(&mut p, tick(secs(i), t0() + secs(i)));
        assert!(!fx.iter().any(|e| matches!(e, Effect::Resolve(_))));
    }

    // The preloaded stream is loaded at once when the track ends, without a second resolve.
    let fx = run(&mut p, Input::AudioEnded);
    assert_eq!(
        fx,
        [
            Effect::Load {
                id: id("b"),
                start: Duration::ZERO
            },
            Effect::Play
        ]
    );
    // And "c" as soon as "b" starts.
    assert!(run(&mut p, Input::AudioStarted).contains(&Effect::Resolve(id("c"))));
}

#[test]
fn preload_near_expiry_is_resolved_again() {
    let mut p = playing(&["a", "b"]);
    // "b" was preloaded at the start of "a" with a URL that's expiring by its turn.
    run(&mut p, ok("b", t0() + secs(60)));
    run(&mut p, tick(secs(40), t0() + secs(40)));
    assert_eq!(
        run(&mut p, Input::Next),
        [Effect::Pause, Effect::Resolve(id("b"))]
    );
}

#[test]
fn radio_fetches_continuation() {
    let mut p = Player::new(prefs(true, true));
    let fx = run(
        &mut p,
        Input::PlayTracks {
            tracks: tracks(&["a", "b", "c"]),
            start: 0,
            radio: true,
        },
    );
    assert_eq!(
        fx,
        [Effect::Resolve(id("a")), Effect::FetchContinuation(id("c"))]
    );
    run(&mut p, ok("a", t0() + 6 * HOUR));
    run(&mut p, Input::AudioStarted);

    // Still waiting for the continuation: moving on doesn't ask again.
    let fx = run(&mut p, Input::Next);
    assert!(!fx.iter().any(|e| matches!(e, Effect::FetchContinuation(_))));

    run(
        &mut p,
        Input::ContinuationLoaded(id("c"), tracks(&["d", "e", "f"])),
    );
    let upcoming: Vec<_> = p
        .queue()
        .upcoming()
        .iter()
        .map(|t| t.id.0.as_str())
        .collect();
    assert_eq!(upcoming, ["c", "d", "e", "f"]);

    // Two left again after two more tracks: the next batch follows the new last track.
    run(&mut p, Input::Next);
    let fx = run(&mut p, Input::Next);
    assert_eq!(
        fx.iter()
            .filter(|e| matches!(e, Effect::FetchContinuation(_)))
            .collect::<Vec<_>>(),
        [&Effect::FetchContinuation(id("f"))]
    );
}

#[test]
fn music_video_plays_song_version() {
    let mut video = typed("v", VideoType::MusicVideo);
    video.song_version = Some(id("s"));
    let play = |p: &mut Player| {
        run(
            p,
            Input::PlayTracks {
                tracks: vec![video.clone()],
                start: 0,
                radio: false,
            },
        )
    };

    let mut p = Player::new(prefs(true, true));
    assert_eq!(play(&mut p), [Effect::Resolve(id("s"))]);

    let mut p = Player::new(prefs(false, true));
    assert_eq!(play(&mut p), [Effect::Resolve(id("v"))]);
}

#[test]
fn continuation_drops_non_music() {
    let mut p = Player::new(prefs(true, true));
    run(
        &mut p,
        Input::PlayTracks {
            tracks: tracks(&["a"]),
            start: 0,
            radio: true,
        },
    );
    run(
        &mut p,
        Input::ContinuationLoaded(
            id("a"),
            vec![
                typed("u", VideoType::UserVideo),
                track("s1"),
                typed("e", VideoType::Episode),
                track("s2"),
            ],
        ),
    );
    let upcoming: Vec<_> = p
        .queue()
        .upcoming()
        .iter()
        .map(|t| t.id.0.as_str())
        .collect();
    assert_eq!(upcoming, ["s1", "s2"]);

    // A user video the user picked plays anyway.
    let mut p = Player::new(prefs(true, true));
    let fx = run(
        &mut p,
        Input::PlayTracks {
            tracks: vec![typed("u", VideoType::UserVideo), track("b")],
            start: 0,
            radio: true,
        },
    );
    assert_eq!(fx.first(), Some(&Effect::Resolve(id("u"))));
    assert_eq!(p.queue().current().map(|t| t.id.clone()), Some(id("u")));
}

#[test]
fn three_failures_stop() {
    let mut p = Player::new(prefs(true, true));
    run(
        &mut p,
        Input::PlayTracks {
            tracks: tracks(&["a", "b", "c", "d"]),
            start: 0,
            radio: false,
        },
    );
    assert_eq!(run(&mut p, err("a")), [Effect::Resolve(id("b"))]);
    assert_eq!(run(&mut p, err("b")), [Effect::Resolve(id("c"))]);

    let fx = run(&mut p, err("c"));
    assert!(matches!(p.status(), Status::Stopped(_)));
    assert!(fx.iter().any(|e| matches!(e, Effect::ShowError(_))));
    assert!(!fx.iter().any(|e| matches!(e, Effect::Resolve(_))));
}

#[test]
fn three_songs_that_are_gone_stop_as_unavailable() {
    let mut p = Player::new(prefs(true, true));
    run(
        &mut p,
        Input::PlayTracks {
            tracks: tracks(&["a", "b", "c", "d"]),
            start: 0,
            radio: false,
        },
    );
    run(&mut p, err("a"));
    run(&mut p, err("b"));
    let fx = run(&mut p, err("c"));
    assert_eq!(stop_of(&fx).map(|s| s.cause), Some(StopCause::Unavailable));
}

#[test]
fn a_broken_resolver_stops_as_broken_with_the_raw_text_for_the_log() {
    let mut p = Player::new(prefs(true, true));
    run(
        &mut p,
        Input::PlayTracks {
            tracks: tracks(&["a", "b", "c", "d"]),
            start: 0,
            radio: false,
        },
    );
    run(&mut p, failed("a", FailKind::Broken));
    run(&mut p, err("b"));
    let fx = run(&mut p, failed("c", FailKind::Broken));
    let stop = stop_of(&fx).expect("stopped");
    assert_eq!(stop.cause, StopCause::Broken);
    assert!(stop.detail.contains("Broken c"), "{}", stop.detail);
}

/// Every song would fail the same way until the helpers download, so the player stops on
/// the song asked for (Play tries again) instead of skipping through the queue.
#[test]
fn a_failed_helpers_download_stops_at_once_on_the_same_song() {
    let mut p = Player::new(prefs(true, true));
    run(
        &mut p,
        Input::PlayTracks {
            tracks: tracks(&["a", "b", "c"]),
            start: 0,
            radio: false,
        },
    );
    let fx = run(&mut p, failed("a", FailKind::Setup));
    assert_eq!(stop_of(&fx).map(|s| s.cause), Some(StopCause::Setup));
    assert_eq!(resolves(&fx), Vec::<String>::new(), "nothing skipped to");
    assert_eq!(p.queue().current().map(|t| t.id.0.as_str()), Some("a"));
    // Play asks for the same song again, so its resolve tries the download again.
    let fx = run(&mut p, Input::Toggle);
    assert_eq!(resolves(&fx), ["a"]);
}

#[test]
fn a_track_that_plays_ends_the_streak() {
    let mut p = Player::new(prefs(true, true));
    run(
        &mut p,
        Input::PlayTracks {
            tracks: tracks(&["a", "b", "c", "d", "e"]),
            start: 0,
            radio: false,
        },
    );
    run(&mut p, failed("a", FailKind::Broken));
    run(&mut p, ok("b", t0() + 6 * HOUR));
    run(&mut p, Input::AudioStarted);
    run(&mut p, Input::AudioEnded);
    run(&mut p, err("c"));
    run(&mut p, err("d"));
    let fx = run(&mut p, err("e"));
    // The broken one before the song that played doesn't count.
    assert_eq!(stop_of(&fx).map(|s| s.cause), Some(StopCause::Unavailable));
}

#[test]
fn the_last_song_gone_stops_as_unavailable() {
    let mut p = Player::new(prefs(true, true));
    run(
        &mut p,
        Input::PlayTracks {
            tracks: tracks(&["a"]),
            start: 0,
            radio: false,
        },
    );
    let fx = run(&mut p, err("a"));
    assert_eq!(stop_of(&fx).map(|s| s.cause), Some(StopCause::Unavailable));
}

#[test]
fn a_stream_that_will_not_open_stops_as_other() {
    let mut p = playing(&["a"]);
    let fx = run(&mut p, Input::AudioFailed("decode error".into()));
    assert_eq!(stop_of(&fx).map(|s| s.cause), Some(StopCause::Other));
}

#[test]
fn expired_url_re_resolves_at_position() {
    let mut p = playing(&["a", "b"]);
    run(&mut p, tick(secs(120), t0()));
    assert_eq!(run(&mut p, Input::Toggle), [Effect::Pause]);
    assert_eq!(p.status(), &Status::Paused);

    // Paused overnight: the URL from the first resolve has expired.
    run(&mut p, tick(secs(120), t0() + 8 * HOUR));
    let fx = run(&mut p, Input::Toggle);
    assert_eq!(fx.first(), Some(&Effect::Resolve(id("a"))));
    assert!(
        !fx.iter()
            .any(|e| matches!(e, Effect::Play | Effect::Load { .. }))
    );

    let fx = run(&mut p, ok("a", t0() + 14 * HOUR));
    assert_eq!(
        fx,
        [
            Effect::Load {
                id: id("a"),
                start: secs(120)
            },
            Effect::Play
        ]
    );
    // Same track, so no second TrackStarted.
    assert_eq!(run(&mut p, Input::AudioStarted), []);
    assert_eq!(p.status(), &Status::Playing);
}

#[test]
fn auth_expired_keeps_current_track() {
    let mut p = playing(&["a", "b"]);
    let fx = run(&mut p, Input::AuthExpired);
    assert_eq!(fx, [Effect::ShowReauth]);
    assert_eq!(p.status(), &Status::Playing);
    assert_eq!(p.queue().current().map(|t| t.id.clone()), Some(id("a")));

    // A second 401 doesn't prompt again.
    assert_eq!(run(&mut p, Input::AuthExpired), []);

    // Signed in again, the next expiry prompts again.
    assert_eq!(run(&mut p, Input::SignedIn), []);
    assert_eq!(run(&mut p, Input::AuthExpired), [Effect::ShowReauth]);
}

#[test]
fn device_lost_pauses() {
    let mut p = playing(&["a", "b"]);
    run(&mut p, tick(secs(42), t0()));
    let fx = run(&mut p, Input::DeviceLost);
    assert_eq!(p.status(), &Status::Paused);
    assert!(!fx.iter().any(|e| matches!(e, Effect::ShowError(_))));
    assert_eq!(p.position(), secs(42));
    assert_eq!(p.queue().current().map(|t| t.id.clone()), Some(id("a")));
}

#[test]
fn restore_is_paused_at_position() {
    let mut p = Player::new(prefs(true, true));
    let fx = run(
        &mut p,
        Input::Restore(Session {
            queue: tracks(&["a", "b", "c"]),
            index: 1,
            position_ms: 61_500,
            radio: false,
            shuffle: false,
            repeat: RepeatMode::All,
        }),
    );
    assert!(fx.is_empty(), "restore must not start anything: {fx:?}");
    assert_eq!(p.status(), &Status::Paused);
    assert_eq!(p.position(), Duration::from_millis(61_500));
    assert_eq!(p.queue().current().map(|t| t.id.clone()), Some(id("b")));

    // The session round-trips, history included.
    let s = p.session();
    assert_eq!(s.index, 1);
    assert_eq!(s.queue.len(), 3);
    assert_eq!(s.position_ms, 61_500);
    assert_eq!(s.repeat, RepeatMode::All);

    // Play resolves and loads at the saved position.
    assert_eq!(run(&mut p, Input::Toggle), [Effect::Resolve(id("b"))]);
    assert_eq!(
        run(&mut p, ok("b", t0() + 6 * HOUR)),
        [
            Effect::Load {
                id: id("b"),
                start: Duration::from_millis(61_500)
            },
            Effect::Play
        ]
    );
}

#[test]
fn scrobbles_once_on_play_time_not_seeks() {
    // 600 s: the threshold is 240 s, less than the 300 s seek below.
    let mut long = track("a");
    long.duration = Some(secs(600));
    let mut p = Player::new(prefs(true, true));
    run(
        &mut p,
        Input::PlayTracks {
            tracks: vec![long, track("b")],
            start: 0,
            radio: false,
        },
    );
    run(&mut p, ok_len("a", secs(600)));
    run(&mut p, Input::AudioStarted);
    run(&mut p, tick(Duration::ZERO, t0()));
    run(&mut p, Input::SeekTo(secs(300)));

    let mut fired = Vec::new();
    for i in 1..=300 {
        for e in run(&mut p, tick(secs(300 + i), t0() + secs(i))) {
            if let Effect::Scrobble(t, started) = e {
                fired.push((i, t.id, started));
            }
        }
    }
    // Once, after 240 s of play time; the seek's 300 s jump didn't count.
    assert_eq!(fired, [(240, id("a"), t0())]);
}

#[test]
fn skipped_silence_does_not_count_as_play_time() {
    let mut long = track("a");
    long.duration = Some(secs(600));
    let mut p = Player::new(prefs(true, true));
    run(
        &mut p,
        Input::PlayTracks {
            tracks: vec![long, track("b")],
            start: 0,
            radio: false,
        },
    );
    run(&mut p, ok_len("a", secs(600)));
    run(&mut p, Input::AudioStarted);
    run(&mut p, tick(Duration::ZERO, t0()));

    // The engine skipped 30 s of silence at 10 s: from then on the position runs 30 s
    // ahead of the time played.
    let mut fired = Vec::new();
    for i in 1..=300 {
        let at = if i < 10 { secs(i) } else { secs(i + 30) };
        for e in run(&mut p, tick(at, t0() + secs(i))) {
            if let Effect::Scrobble(t, _) = e {
                fired.push((i, t.id));
            }
        }
    }
    assert_eq!(fired, [(240, id("a"))]);
}

#[test]
fn stale_continuation_is_ignored() {
    let mut p = Player::new(prefs(true, true));
    let radio = |ids: &[&str]| Input::PlayTracks {
        tracks: tracks(ids),
        start: 0,
        radio: true,
    };
    run(&mut p, radio(&["a", "b", "c"]));
    let fx = run(&mut p, radio(&["x", "y", "z"]));
    assert!(fx.contains(&Effect::FetchContinuation(id("z"))));

    // The first queue's answer lands late: it isn't mixed into the new queue.
    run(
        &mut p,
        Input::ContinuationLoaded(id("c"), tracks(&["d", "e"])),
    );
    let upcoming = |p: &Player| -> Vec<String> {
        p.queue()
            .upcoming()
            .iter()
            .map(|t| t.id.0.clone())
            .collect()
    };
    assert_eq!(upcoming(&p), ["y", "z"]);

    run(&mut p, Input::ContinuationLoaded(id("z"), tracks(&["w"])));
    assert_eq!(upcoming(&p), ["y", "z", "w"]);
}

#[test]
fn next_and_previous_silence_the_old_track_while_resolving() {
    let mut p = playing(&["a", "b", "c"]);
    // The preload of "b" failed, so Next has to resolve it.
    run(&mut p, err("b"));

    // The resolve takes ~10 s: the old track must not keep playing meanwhile.
    let fx = run(&mut p, Input::Next);
    assert_eq!(fx, [Effect::Pause, Effect::Resolve(id("b"))]);
    assert_eq!(p.status(), &Status::Loading);
    assert_eq!(p.queue().current().map(|t| &t.id), Some(&id("b")));

    run(&mut p, ok("b", t0() + 6 * HOUR));
    run(&mut p, Input::AudioStarted);
    run(&mut p, tick(secs(10), t0()));

    // More than 3 s in: Previous restarts the track.
    assert_eq!(run(&mut p, Input::Previous), [Effect::Seek(Duration::ZERO)]);
    // At the start: Previous goes back to "a", silencing "b" first.
    let fx = run(&mut p, Input::Previous);
    assert_eq!(fx.first(), Some(&Effect::Pause));
    assert_eq!(p.queue().current().map(|t| &t.id), Some(&id("a")));
}

#[test]
fn previous_reuses_the_stream_of_the_track_left() {
    let mut p = playing(&["a", "b", "c"]);
    run(&mut p, ok("b", t0() + 6 * HOUR));
    run(&mut p, tick(secs(1), t0() + secs(1)));
    run(&mut p, Input::Next);
    run(&mut p, Input::AudioStarted);

    // Back to "a" at once: its stream was kept, no resolve.
    let load = |s: &str| Effect::Load {
        id: id(s),
        start: Duration::ZERO,
    };
    assert_eq!(
        run(&mut p, Input::Previous),
        [Effect::Pause, load("a"), Effect::Play]
    );
    // "b" is next again: its stream is the spare, so it's buffered without a resolve.
    let fx = run(&mut p, Input::AudioStarted);
    assert!(fx.contains(&Effect::Preload(id("b"))));
    assert!(!fx.iter().any(|e| matches!(e, Effect::Resolve(_))));
    // And forward to "b" again, at once.
    assert_eq!(
        run(&mut p, Input::Next),
        [Effect::Pause, load("b"), Effect::Play]
    );
    run(&mut p, Input::AudioStarted);

    // Only one spare, and only while it's fresh: hours later "a" is resolved again.
    run(&mut p, tick(secs(1), t0() + 6 * HOUR - secs(10)));
    assert_eq!(
        run(&mut p, Input::Previous),
        [Effect::Pause, Effect::Resolve(id("a"))]
    );
}

#[test]
fn loading_shows_playing_unless_paused() {
    let mut p = playing(&["a", "b"]);
    assert!(p.wants_play());
    run(&mut p, Input::Next);
    assert_eq!(p.status(), &Status::Loading);
    assert!(p.wants_play(), "loading after Next shows playing");
    run(&mut p, Input::Toggle);
    assert!(!p.wants_play(), "a pause while loading shows paused");
    run(&mut p, Input::Toggle);
    assert_eq!(p.status(), &Status::Loading);
    assert!(p.wants_play());
    run(&mut p, ok("b", t0() + 6 * HOUR));
    assert!(p.wants_play());
    run(&mut p, Input::AudioStarted);
    assert_eq!(p.status(), &Status::Playing);
    assert!(p.wants_play());
}

/// Whether the engine is making sound after `fx`, given whether it was before: `Play`
/// starts it, `Pause` and `Load` (which loads paused) stop it.
fn audible_after(mut audible: bool, fx: &[Effect]) -> bool {
    for e in fx {
        match e {
            Effect::Play => audible = true,
            Effect::Pause | Effect::Load { .. } => audible = false,
            _ => {}
        }
    }
    audible
}

#[test]
fn pause_while_next_resolves_is_silent() {
    let mut p = playing(&["a", "b"]);
    let mut audible = true;
    audible = audible_after(audible, &run(&mut p, Input::Next));

    // Play/pause during the resolve (the taskbar, tray or media card button): silence at
    // once, not when the new track loads, and the new track loads paused.
    audible = audible_after(audible, &run(&mut p, Input::Toggle));
    assert!(!audible, "the old track is still playing after pause");
    assert_eq!(p.status(), &Status::Paused);
    let fx = run(&mut p, ok("b", t0() + 6 * HOUR));
    assert_eq!(
        fx,
        [Effect::Load {
            id: id("b"),
            start: Duration::ZERO
        }]
    );
    assert!(!audible_after(audible, &fx));
    assert_eq!(p.status(), &Status::Paused);
}

fn upcoming(p: &Player) -> Vec<String> {
    p.queue()
        .upcoming()
        .iter()
        .map(|t| t.id.0.clone())
        .collect()
}

fn resolves(fx: &[Effect]) -> Vec<String> {
    fx.iter()
        .filter_map(|e| match e {
            Effect::Resolve(id) => Some(id.0.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn shuffle_and_repeat_are_saved_and_replan_the_preload() {
    let mut p = playing(&["a", "b", "c", "d", "e", "f", "g", "h"]);
    run(&mut p, ok("b", t0() + 6 * HOUR));

    // Repeat One: the next track is the current one, so nothing is preloaded.
    let fx = run(&mut p, Input::SetRepeat(RepeatMode::One));
    assert!(fx.contains(&Effect::PersistSession));
    assert_eq!(resolves(&fx), Vec::<String>::new());
    assert_eq!(p.session().repeat, RepeatMode::One);
    assert_eq!(p.queue().peek_next().map(|t| t.id.clone()), Some(id("a")));

    // Off again: "b" is next once more, and its stream is still good, so no resolve.
    let fx = run(&mut p, Input::SetRepeat(RepeatMode::Off));
    assert_eq!(fx, [Effect::Preload(id("b")), Effect::PersistSession]);

    // Shuffle reorders what's upcoming, keeps the current track, and is saved.
    let fx = run(&mut p, Input::SetShuffle(true));
    assert!(fx.contains(&Effect::PersistSession));
    assert!(p.session().shuffle);
    assert_eq!(p.queue().current().map(|t| t.id.clone()), Some(id("a")));
    let mut shuffled = upcoming(&p);
    assert_ne!(shuffled, ["b", "c", "d", "e", "f", "g", "h"]);
    shuffled.sort();
    assert_eq!(shuffled, ["b", "c", "d", "e", "f", "g", "h"]);
    // The new next track is the one preloaded.
    let next = p.queue().upcoming()[0].id.0.clone();
    if next != "b" {
        assert_eq!(resolves(&fx), [next]);
    }

    // Off restores the queued order.
    let fx = run(&mut p, Input::SetShuffle(false));
    assert!(!p.session().shuffle);
    assert!(fx.contains(&Effect::PersistSession));
    assert_eq!(upcoming(&p), ["b", "c", "d", "e", "f", "g", "h"]);

    // A new queue while shuffle is on is shuffled too.
    run(&mut p, Input::SetShuffle(true));
    run(
        &mut p,
        Input::PlayTracks {
            tracks: tracks(&["m", "n", "o", "p", "q", "r", "s"]),
            start: 0,
            radio: false,
        },
    );
    assert_ne!(upcoming(&p), ["n", "o", "p", "q", "r", "s"]);
}

#[test]
fn reorder_remove_and_clear_edit_upcoming() {
    let mut p = playing(&["a", "b", "c", "d"]);
    run(&mut p, ok("b", t0() + 6 * HOUR));

    let fx = run(&mut p, Input::MoveUpcoming { from: 0, to: 2 });
    assert_eq!(upcoming(&p), ["c", "d", "b"]);
    assert!(fx.contains(&Effect::PersistSession));
    assert_eq!(resolves(&fx), ["c"]);

    let fx = run(&mut p, Input::RemoveUpcoming(1));
    assert_eq!(upcoming(&p), ["c", "b"]);
    assert!(fx.contains(&Effect::PersistSession));
    // "c" is still next: its resolve is already running.
    assert_eq!(resolves(&fx), Vec::<String>::new());

    // Out of range: nothing changes and nothing is saved.
    assert_eq!(run(&mut p, Input::RemoveUpcoming(9)), []);
    assert_eq!(run(&mut p, Input::MoveUpcoming { from: 0, to: 9 }), []);

    let fx = run(&mut p, Input::ClearUpcoming);
    assert!(upcoming(&p).is_empty());
    assert!(fx.contains(&Effect::PersistSession));
    assert_eq!(p.queue().current().map(|t| t.id.clone()), Some(id("a")));
    // The answer for "c" comes in late: it isn't buffered, and Next has nowhere to go.
    assert_eq!(run(&mut p, ok("c", t0() + 6 * HOUR)), []);
    assert_eq!(run(&mut p, Input::Next), []);
    assert_eq!(p.status(), &Status::Playing);

    // A radio asks for more as soon as it's cleared.
    let mut p = Player::new(prefs(true, true));
    run(
        &mut p,
        Input::PlayTracks {
            tracks: tracks(&["a", "b", "c", "d", "e"]),
            start: 0,
            radio: true,
        },
    );
    let fx = run(&mut p, Input::ClearUpcoming);
    assert!(fx.contains(&Effect::FetchContinuation(id("a"))));
}

#[test]
fn play_next_enqueue_and_skip_to() {
    let mut p = playing(&["a", "b"]);
    let fx = run(&mut p, Input::PlayNext(track("x")));
    assert_eq!(upcoming(&p), ["x", "b"]);
    assert!(fx.contains(&Effect::PersistSession));
    assert_eq!(resolves(&fx), ["x"]);

    let fx = run(&mut p, Input::Enqueue(tracks(&["y", "z"])));
    assert_eq!(upcoming(&p), ["x", "b", "y", "z"]);
    assert!(fx.contains(&Effect::PersistSession));
    assert_eq!(resolves(&fx), Vec::<String>::new());
    assert_eq!(p.session().queue.len(), 5);

    // Clicking an upcoming track plays it; the ones before it go to history.
    let fx = run(&mut p, Input::SkipTo(2));
    assert_eq!(fx.first(), Some(&Effect::Pause));
    assert_eq!(resolves(&fx), ["y"]);
    assert_eq!(p.queue().current().map(|t| t.id.clone()), Some(id("y")));
    assert_eq!(upcoming(&p), ["z"]);
    assert_eq!(p.session().index, 3);
    assert!(p.wants_play());
    assert_eq!(run(&mut p, Input::SkipTo(5)), []);

    // On an empty player, adding to the queue starts playing.
    let mut p = Player::new(prefs(true, true));
    let fx = run(&mut p, Input::Enqueue(tracks(&["a", "b"])));
    assert_eq!(resolves(&fx), ["a"]);
    assert!(p.wants_play());
    assert_eq!(upcoming(&p), ["b"]);
    let mut p = Player::new(prefs(true, true));
    let fx = run(&mut p, Input::PlayNext(track("a")));
    assert_eq!(resolves(&fx), ["a"]);
    assert_eq!(p.status(), &Status::Loading);
}

#[test]
fn play_next_all_goes_after_the_current_track_and_starts_an_empty_player() {
    let mut p = playing(&["a", "b"]);
    let fx = run(&mut p, Input::PlayNextAll(tracks(&["x", "y"])));
    assert_eq!(upcoming(&p), ["x", "y", "b"]);
    assert!(fx.contains(&Effect::PersistSession));
    assert_eq!(resolves(&fx), ["x"]);
    assert_eq!(run(&mut p, Input::PlayNextAll(Vec::new())), []);

    let mut p = Player::new(prefs(true, true));
    let fx = run(&mut p, Input::PlayNextAll(tracks(&["x", "y"])));
    assert_eq!(resolves(&fx), ["x"]);
    assert_eq!(upcoming(&p), ["y"]);
}

#[test]
fn a_replaced_preload_never_plays() {
    let mut p = playing(&["a", "b", "c"]);
    assert_eq!(
        run(&mut p, ok("b", t0() + 6 * HOUR)),
        [Effect::Preload(id("b"))]
    );

    // "c" moves ahead of the buffered "b": it's resolved and buffered instead.
    let fx = run(&mut p, Input::MoveUpcoming { from: 1, to: 0 });
    assert_eq!(resolves(&fx), ["c"]);
    assert!(!fx.contains(&Effect::Preload(id("b"))));
    assert_eq!(
        run(&mut p, ok("c", t0() + 6 * HOUR)),
        [Effect::Preload(id("c"))]
    );

    // Next loads "c" at once, not "b".
    let load = |s: &str| Effect::Load {
        id: id(s),
        start: Duration::ZERO,
    };
    assert_eq!(
        run(&mut p, Input::Next),
        [Effect::Pause, load("c"), Effect::Play]
    );
    // "b" is next again; its stream was kept, so it's buffered without a resolve.
    let fx = run(&mut p, Input::AudioStarted);
    assert!(fx.contains(&Effect::Preload(id("b"))));
    assert_eq!(resolves(&fx), Vec::<String>::new());

    // While a resolve for the next track runs, a new next track takes its place: the
    // late answer for the old one is dropped.
    let mut p = playing(&["a", "b", "c"]);
    run(&mut p, Input::PlayNext(track("x")));
    assert_eq!(run(&mut p, ok("b", t0() + 6 * HOUR)), []);
    assert_eq!(
        run(&mut p, ok("x", t0() + 6 * HOUR)),
        [Effect::Preload(id("x"))]
    );
    assert_eq!(
        run(&mut p, Input::Next),
        [Effect::Pause, load("x"), Effect::Play]
    );
}

fn saves(fx: &[Effect]) -> usize {
    fx.iter().filter(|e| **e == Effect::PersistSession).count()
}

#[test]
fn ticks_save_only_a_changed_session() {
    let mut p = playing(&["a", "b"]);
    assert!(p.needs_ticks());
    // Playing: saved every 10 s of ticks, as the position moves.
    let mut saved = 0;
    for i in 0..=20 {
        saved += saves(&run(&mut p, tick(secs(i), t0() + secs(i))));
    }
    assert_eq!(saved, 2);

    // Paused: the ticks can stop, and the tick after the pause saves at once.
    run(&mut p, Input::Toggle);
    assert!(!p.needs_ticks());
    let fx = run(&mut p, tick(secs(20), t0() + secs(21)));
    assert_eq!(saves(&fx), 1);
    assert_eq!(p.session().position_ms, 20_000);

    // Nothing changed: no more saves, however late the ticks come.
    for i in 1..=5 {
        assert_eq!(run(&mut p, tick(secs(20), t0() + secs(21) + HOUR * i)), []);
    }

    // A seek while paused is saved by the next tick, once.
    run(&mut p, Input::SeekTo(secs(90)));
    assert_eq!(saves(&run(&mut p, tick(secs(90), t0() + 6 * HOUR))), 1);
    assert_eq!(p.session().position_ms, 90_000);
    assert_eq!(saves(&run(&mut p, tick(secs(90), t0() + 6 * HOUR))), 0);

    run(&mut p, Input::Toggle);
    assert!(p.needs_ticks());
}

#[test]
fn a_restored_session_is_not_saved_again() {
    let mut p = Player::new(prefs(true, true));
    run(
        &mut p,
        Input::Restore(Session {
            queue: tracks(&["a", "b"]),
            index: 0,
            position_ms: 5_000,
            radio: false,
            shuffle: false,
            repeat: RepeatMode::Off,
        }),
    );
    assert!(!p.needs_ticks());
    for i in 0..30 {
        assert_eq!(run(&mut p, tick(Duration::ZERO, t0() + secs(i))), []);
    }
}

/// A Home shelf or artist top-song track: the catalog gives no length.
fn untimed(s: &str) -> Track {
    let mut t = track(s);
    t.duration = None;
    t
}

fn ok_len(s: &str, len: Duration) -> Input {
    Input::Resolved {
        id: id(s),
        result: Ok(StreamInfo {
            expires_at: t0() + 6 * HOUR,
            duration: Some(len),
        }),
    }
}

#[test]
fn the_stream_length_fills_in_a_missing_duration() {
    let mut p = Player::new(prefs(true, true));
    run(
        &mut p,
        Input::PlayTracks {
            tracks: vec![untimed("a"), untimed("b")],
            start: 0,
            radio: false,
        },
    );
    assert_eq!(p.queue().current().unwrap().duration, None);

    // The bar, the media card and Discord read the queue's current track.
    let fx = p.handle(ok_len("a", secs(215)));
    assert!(fx.contains(&Effect::StateChanged), "{fx:?}");
    assert_eq!(p.queue().current().unwrap().duration, Some(secs(215)));

    // The scrobblers' now playing gets it too, and so does the saved session.
    let fx = run(&mut p, Input::AudioStarted);
    let started = fx.iter().find_map(|e| match e {
        Effect::TrackStarted(t) => Some(t.duration),
        _ => None,
    });
    assert_eq!(started, Some(Some(secs(215))));
    assert_eq!(p.session().queue[0].duration, Some(secs(215)));

    // The next track's preload fills in its length before it plays.
    assert_eq!(p.queue().upcoming()[0].duration, None);
    let fx = p.handle(ok_len("b", secs(180)));
    assert!(fx.contains(&Effect::Preload(id("b"))), "{fx:?}");
    assert!(fx.contains(&Effect::StateChanged), "{fx:?}");
    assert_eq!(p.queue().upcoming()[0].duration, Some(secs(180)));
    run(&mut p, Input::Next);
    assert_eq!(p.queue().current().unwrap().duration, Some(secs(180)));
}

#[test]
fn a_reused_stream_fills_in_the_duration_of_a_new_queue() {
    // Played once (its stream is kept), then queued again from a list without lengths.
    let mut p = Player::new(prefs(true, true));
    run(
        &mut p,
        Input::PlayTracks {
            tracks: vec![untimed("a")],
            start: 0,
            radio: false,
        },
    );
    run(&mut p, ok_len("a", secs(215)));
    run(&mut p, Input::AudioStarted);
    let fx = run(
        &mut p,
        Input::PlayTracks {
            tracks: vec![untimed("a")],
            start: 0,
            radio: false,
        },
    );
    assert!(!resolves(&fx).contains(&"a".to_string()), "{fx:?}");
    assert_eq!(p.queue().current().unwrap().duration, Some(secs(215)));
}

#[test]
fn the_song_versions_length_replaces_the_videos() {
    let mut video = typed("v", VideoType::MusicVideo);
    video.song_version = Some(id("s"));
    video.duration = Some(secs(260));
    let mut p = Player::new(prefs(true, true));
    run(
        &mut p,
        Input::PlayTracks {
            tracks: vec![video],
            start: 0,
            radio: false,
        },
    );
    // What plays is the song version, so its length is the one the bar runs to.
    run(&mut p, ok_len("s", secs(231)));
    assert_eq!(p.queue().current().unwrap().duration, Some(secs(231)));
}

#[test]
fn a_track_without_a_catalog_duration_scrobbles_at_half_the_stream_length() {
    let mut p = Player::new(prefs(true, true));
    run(
        &mut p,
        Input::PlayTracks {
            tracks: vec![untimed("a"), untimed("b")],
            start: 0,
            radio: false,
        },
    );
    run(&mut p, ok_len("a", secs(200)));
    run(&mut p, Input::AudioStarted);
    run(&mut p, tick(Duration::ZERO, t0()));
    let mut fired = Vec::new();
    for i in 1..=150 {
        for e in run(&mut p, tick(secs(i), t0() + secs(i))) {
            if let Effect::Scrobble(t, _) = e {
                fired.push((i, t.id, t.duration));
            }
        }
    }
    assert_eq!(fired, [(100, id("a"), Some(secs(200)))]);
}

#[test]
fn previous_shows_the_restart_of_a_track_not_loaded() {
    // A restored session, paused at 61.5 s with nothing loaded: Previous restarts it, and
    // the bar must hear about it (the tick after the input doesn't push the state).
    let mut p = Player::new(prefs(true, true));
    run(
        &mut p,
        Input::Restore(Session {
            queue: tracks(&["a", "b"]),
            index: 1,
            position_ms: 61_500,
            radio: false,
            shuffle: false,
            repeat: RepeatMode::Off,
        }),
    );
    let fx = p.handle(Input::Previous);
    assert_eq!(p.position(), Duration::ZERO);
    assert!(fx.contains(&Effect::StateChanged), "{fx:?}");

    // The same while the current track is still resolving.
    let mut p = playing(&["a", "b"]);
    run(&mut p, Input::Next);
    run(&mut p, Input::SeekTo(secs(10)));
    let fx = p.handle(Input::Previous);
    assert_eq!(p.position(), Duration::ZERO);
    assert!(fx.contains(&Effect::StateChanged), "{fx:?}");
}

// "Skip songs I've disliked".

fn disliked(s: &str) -> Track {
    Track {
        like: LikeStatus::Dislike,
        ..track(s)
    }
}

/// A player with `queue` (a list, not radio) and its `start` track audible, and the
/// effects of that start (the next track's preload).
fn audible_from(prefs: PlaybackPrefs, queue: Vec<Track>, start: usize) -> (Player, Vec<Effect>) {
    let mut p = Player::new(prefs);
    let first = queue[start].id.0.clone();
    run(
        &mut p,
        Input::PlayTracks {
            tracks: queue,
            start,
            radio: false,
        },
    );
    run(&mut p, ok(&first, t0() + 6 * HOUR));
    let fx = run(&mut p, Input::AudioStarted);
    (p, fx)
}

fn current(p: &Player) -> Option<String> {
    p.queue().current().map(|t| t.id.0.clone())
}

#[test]
fn the_end_of_a_track_passes_over_a_disliked_next_one() {
    let queue = vec![track("a"), disliked("b"), track("c")];
    let (mut p, started) = audible_from(prefs(true, true), queue, 0);
    // The track after the disliked one is the one buffered ahead.
    assert_eq!(resolves(&started), ["c"]);

    let fx = run(&mut p, Input::AudioEnded);
    assert_eq!(current(&p).as_deref(), Some("c"));
    assert!(stop_of(&fx).is_none());
}

#[test]
fn next_passes_over_disliked_tracks_even_from_repeat_one() {
    let queue = vec![track("a"), disliked("b"), disliked("c"), track("d")];
    let (mut p, _) = audible_from(prefs(true, true), queue, 0);
    run(&mut p, Input::SetRepeat(RepeatMode::One));
    run(&mut p, Input::Next);
    assert_eq!(current(&p).as_deref(), Some("d"));
}

#[test]
fn a_disliked_track_started_directly_still_plays() {
    let queue = vec![track("a"), disliked("b"), track("c")];
    let mut p = Player::new(prefs(true, true));
    let fx = run(
        &mut p,
        Input::PlayTracks {
            tracks: queue.clone(),
            start: 1,
            radio: false,
        },
    );
    assert_eq!(resolves(&fx), ["b"]);

    // Picked from Up next.
    let (mut p, _) = audible_from(prefs(true, true), queue, 0);
    run(&mut p, Input::SkipTo(0));
    assert_eq!(current(&p).as_deref(), Some("b"));
}

#[test]
fn repeat_one_keeps_playing_a_disliked_track() {
    let (mut p, _) = audible_from(prefs(true, true), vec![disliked("a"), track("b")], 0);
    run(&mut p, Input::SetRepeat(RepeatMode::One));
    let fx = run(&mut p, Input::AudioEnded);
    assert_eq!(current(&p).as_deref(), Some("a"));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Load { id, .. } if id.0 == "a"))
    );
}

#[test]
fn only_disliked_tracks_left_stops_as_at_the_end_of_the_queue() {
    let queue = vec![track("a"), disliked("b"), disliked("c")];
    let (mut p, started) = audible_from(prefs(true, true), queue, 0);
    assert_eq!(resolves(&started), Vec::<String>::new());

    // Next does nothing, as on the last track.
    let fx = run(&mut p, Input::Next);
    assert_eq!(fx, Vec::<Effect>::new());
    assert_eq!(p.status(), &Status::Playing);

    // The end stops on the track that played, not as a failure.
    let fx = run(&mut p, Input::AudioEnded);
    assert_eq!(current(&p).as_deref(), Some("a"));
    assert_eq!(p.status(), &Status::Idle);
    assert!(stop_of(&fx).is_none());
}

#[test]
fn repeat_all_over_disliked_tracks_does_not_spin() {
    // Every track disliked, the first started directly: its end stops.
    let (mut p, _) = audible_from(prefs(true, true), vec![disliked("a"), disliked("b")], 0);
    run(&mut p, Input::SetRepeat(RepeatMode::All));
    run(&mut p, Input::AudioEnded);
    assert_eq!(p.status(), &Status::Idle);

    // One track left that isn't: it plays again.
    let (mut p, _) = audible_from(prefs(true, true), vec![track("a"), disliked("b")], 0);
    run(&mut p, Input::SetRepeat(RepeatMode::All));
    let fx = run(&mut p, Input::AudioEnded);
    assert_eq!(current(&p).as_deref(), Some("a"));
    assert!(
        fx.iter()
            .any(|e| matches!(e, Effect::Load { id, .. } if id.0 == "a"))
    );
}

#[test]
fn a_continuation_waited_for_passes_over_its_disliked_tracks() {
    let mut p = Player::new(prefs(true, true));
    run(
        &mut p,
        Input::PlayTracks {
            tracks: vec![track("a")],
            start: 0,
            radio: true,
        },
    );
    run(&mut p, ok("a", t0() + 6 * HOUR));
    run(&mut p, Input::AudioStarted);
    run(&mut p, Input::AudioEnded);
    assert_eq!(p.status(), &Status::Loading);

    run(
        &mut p,
        Input::ContinuationLoaded(id("a"), vec![disliked("b"), track("c")]),
    );
    assert_eq!(current(&p).as_deref(), Some("c"));
}

#[test]
fn with_the_setting_off_a_disliked_track_plays() {
    let off = PlaybackPrefs {
        skip_disliked: false,
        ..prefs(true, true)
    };
    let (mut p, _) = audible_from(off, vec![track("a"), disliked("b"), track("c")], 0);
    run(&mut p, Input::AudioEnded);
    assert_eq!(current(&p).as_deref(), Some("b"));
}

#[test]
fn turning_the_setting_on_moves_the_preload_past_a_disliked_next_track() {
    let off = PlaybackPrefs {
        skip_disliked: false,
        ..prefs(true, true)
    };
    let (mut p, started) = audible_from(off, vec![track("a"), disliked("b"), track("c")], 0);
    assert_eq!(resolves(&started), ["b"]);

    let fx = run(&mut p, Input::SetPrefs(prefs(true, true)));
    assert_eq!(resolves(&fx), ["c"]);
}

#[test]
fn a_track_disliked_while_queued_is_passed_over_when_it_comes_round() {
    let (mut p, _) = audible_from(prefs(true, true), tracks(&["a", "b", "c"]), 0);
    let fx = run(&mut p, Input::SetLike(id("b"), LikeStatus::Dislike));
    // The preload moves on to the track that will play, and the queue is saved.
    assert_eq!(resolves(&fx), ["c"]);
    assert!(fx.contains(&Effect::PersistSession));

    run(&mut p, Input::AudioEnded);
    assert_eq!(current(&p).as_deref(), Some("c"));
    assert_eq!(
        p.session().queue[1].like,
        LikeStatus::Dislike,
        "the rating is saved with the session"
    );
}
