//! Crossfade: two voices at once for the fade, on the null output.

use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use super::webm::WEBM;
use super::{engine_tmp, env, fixture, local, near, process_cpu, serve, src, wait_event};
use crate::StreamSource;

/// An engine playing `tone-10s.m4a` from `at`, with `tone-10s-frag.m4a` preloaded and its
/// ring filled.
fn playing_with_next(
    at: Duration,
) -> (
    crate::AudioEngine,
    std::sync::mpsc::Receiver<crate::AudioEvent>,
) {
    let engine = crate::AudioEngine::new_null(engine_tmp());
    let events = engine.events();
    engine.load(local("tone-10s.m4a"), at);
    engine.play();
    wait_event(&events, Duration::from_secs(3), crate::AudioEvent::Started);
    engine.preload(local("tone-10s-frag.m4a"));
    std::thread::sleep(Duration::from_millis(400));
    (engine, events)
}

#[test]
fn engine_crossfade_sounds_both_tracks_then_only_the_new_one() {
    let (engine, events) = playing_with_next(Duration::from_secs(8));
    assert_eq!(engine.sources(), 1);
    // Longer than what's left of the old track, so its stream ends inside the fade.
    engine.crossfade_to(local("tone-10s-frag.m4a"), Duration::from_secs(3));
    wait_event(&events, Duration::from_secs(1), crate::AudioEvent::Started);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(engine.sources(), 2, "both tracks sound during the fade");
    let at = engine.position();
    assert!(at < Duration::from_secs(1), "{at:?}");
    std::thread::sleep(Duration::from_millis(2500));
    assert_eq!(engine.sources(), 1, "the old track is gone");
    // The old track's end, inside the fade, wasn't told: the first `Ended` is the new one's.
    engine.seek(Duration::from_millis(9500));
    let t = Instant::now();
    wait_event(&events, Duration::from_secs(3), crate::AudioEvent::Ended);
    let after = t.elapsed();
    assert!(
        after > Duration::from_millis(300),
        "Ended {after:?} after the seek"
    );
}

#[test]
fn engine_crossfade_holds_no_third_ring() {
    let (engine, _events) = playing_with_next(Duration::ZERO);
    engine.crossfade_to(local("tone-10s-frag.m4a"), Duration::from_secs(1));
    // The track after next, asked for as soon as the new one starts, waits for the fade.
    engine.preload(local("silence-ends.m4a"));
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(crate::voice::rings(), 2);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(engine.sources(), 1);
    assert_eq!(
        crate::voice::rings(),
        2,
        "the old ring went, the next one came"
    );
}

#[test]
fn engine_crossfade_to_the_waiting_preload_downloads_it_once() {
    let (engine, _events) = playing_with_next(Duration::ZERO);
    engine.crossfade_to(local("tone-10s-frag.m4a"), Duration::from_secs(1));
    // The track after next waits for the fade; then Fade on skip goes straight to it.
    engine.preload(local("silence-ends.m4a"));
    engine.fade_out(Duration::from_millis(500));
    engine.crossfade_to(local("silence-ends.m4a"), Duration::from_millis(500));
    std::thread::sleep(Duration::from_millis(1800));
    assert_eq!(engine.sources(), 1);
    assert_eq!(crate::voice::rings(), 1, "a second ring for the same track");
}

#[test]
fn engine_device_change_during_a_crossfade_drops_the_old_track() {
    let (engine, events) = playing_with_next(Duration::ZERO);
    engine.crossfade_to(local("tone-10s-frag.m4a"), Duration::from_secs(5));
    wait_event(&events, Duration::from_secs(1), crate::AudioEvent::Started);
    engine.set_device(None);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(engine.sources(), 1);
    assert_eq!(crate::voice::rings(), 1);
}

#[test]
fn engine_rapid_faded_skips_keep_two_fading_tracks_at_most() {
    let (engine, _events) = playing_with_next(Duration::ZERO);
    for name in [
        "tone-10s-frag.m4a",
        "silence-ends.m4a",
        "tone-10s.webm",
        "tone-10s.m4a",
    ] {
        engine.fade_out(Duration::from_secs(2));
        let mut next = local(name);
        if name.ends_with(".webm") {
            next.mime = WEBM.into();
        }
        engine.crossfade_to(next, Duration::from_secs(2));
        std::thread::sleep(Duration::from_millis(250));
    }
    assert!(engine.sources() <= 3, "{} sources", engine.sources());
    std::thread::sleep(Duration::from_millis(2500));
    assert_eq!(engine.sources(), 1);
}

#[test]
fn engine_crossfade_without_a_preload_fades_in_once_loaded() {
    let engine = crate::AudioEngine::new_null(engine_tmp());
    let events = engine.events();
    engine.load(local("tone-10s.m4a"), Duration::ZERO);
    engine.play();
    wait_event(&events, Duration::from_secs(3), crate::AudioEvent::Started);
    engine.crossfade_to(local("tone-10s-frag.m4a"), Duration::from_secs(1));
    wait_event(&events, Duration::from_secs(3), crate::AudioEvent::Started);
    std::thread::sleep(Duration::from_millis(1500));
    assert_eq!(engine.sources(), 1);
}

#[test]
fn engine_load_during_a_crossfade_stops_both_tracks() {
    let (engine, events) = playing_with_next(Duration::ZERO);
    engine.crossfade_to(local("tone-10s-frag.m4a"), Duration::from_secs(5));
    wait_event(&events, Duration::from_secs(1), crate::AudioEvent::Started);
    engine.load(local("silence-ends.m4a"), Duration::ZERO);
    engine.play();
    wait_event(&events, Duration::from_secs(3), crate::AudioEvent::Started);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(engine.sources(), 1);
}

#[test]
fn engine_seek_during_a_crossfade_ends_the_fade_on_the_new_track() {
    let (engine, events) = playing_with_next(Duration::ZERO);
    engine.crossfade_to(local("tone-10s-frag.m4a"), Duration::from_secs(5));
    wait_event(&events, Duration::from_secs(1), crate::AudioEvent::Started);
    engine.seek(Duration::from_secs(5));
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(engine.sources(), 1);
    let at = engine.position();
    assert!(
        at >= Duration::from_secs(5) && at < Duration::from_secs(6),
        "{at:?}"
    );
}

// Where the music ends: the tail scan.

/// `name`'s music end, scanned from a stream `length` long.
fn scanned(name: &str, mime: &str, length: Duration) -> Option<Duration> {
    let env = env();
    let (media, n, _) = env.open(&local(name).url);
    crate::tail::music_end(media, n, mime, length, &AtomicBool::new(false))
}

#[test]
fn the_tail_scan_finds_where_a_silent_outro_starts() {
    near(
        scanned("silence-ends.m4a", "audio/mp4", Duration::from_secs(6)),
        4.0,
    );
}

#[test]
fn the_tail_scan_of_a_song_that_sounds_to_its_end() {
    near(
        scanned("tone-10s.m4a", "audio/mp4", Duration::from_secs(10)),
        10.0,
    );
    near(
        scanned("tone-10s.webm", WEBM, Duration::from_secs(10)),
        10.0,
    );
}

/// `silence-ends.m4a` as a 6 s stream.
fn silent_outro() -> StreamSource {
    StreamSource {
        duration: Some(Duration::from_secs(6)),
        loudness_db: None,
        ..local("silence-ends.m4a")
    }
}

#[test]
fn engine_finds_where_the_music_ends_while_crossfade_is_on() {
    let engine = crate::AudioEngine::new_null(engine_tmp());
    engine.set_crossfade(true);
    let events = engine.events();
    engine.load(silent_outro(), Duration::ZERO);
    engine.play();
    wait_event(&events, Duration::from_secs(3), crate::AudioEvent::Started);
    std::thread::sleep(Duration::from_millis(500));
    near(engine.playhead().music_end, 4.0);
}

#[test]
fn engine_scans_nothing_with_crossfade_off() {
    let engine = crate::AudioEngine::new_null(engine_tmp());
    let events = engine.events();
    engine.load(silent_outro(), Duration::ZERO);
    engine.play();
    wait_event(&events, Duration::from_secs(3), crate::AudioEvent::Started);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(engine.playhead().music_end, None);
}

#[test]
fn engine_scans_a_download_once_it_is_complete() {
    // ~2.6 s for the whole file.
    let server = serve(fixture("tone-10s-frag.m4a"), 32 << 10, Duration::ZERO);
    let engine = crate::AudioEngine::new_null(engine_tmp());
    engine.set_crossfade(true);
    let events = engine.events();
    let stream = StreamSource {
        duration: Some(Duration::from_secs(10)),
        loudness_db: None,
        ..src(&server.url)
    };
    engine.load(stream, Duration::ZERO);
    engine.play();
    wait_event(&events, Duration::from_secs(3), crate::AudioEvent::Started);
    assert_eq!(
        engine.playhead().music_end,
        None,
        "scanned before the download ended"
    );
    let t = Instant::now();
    while engine.playhead().music_end.is_none() && t.elapsed() < Duration::from_secs(6) {
        std::thread::sleep(Duration::from_millis(100));
    }
    near(engine.playhead().music_end, 10.0);
}

/// What a crossfade costs, on the null output with crossfade on: the process's CPU per
/// second with the AAC tone playing alone, during a 5 s crossfade into the Opus tone (two
/// decoders and the ramps), then with the Opus tone alone; and one tail scan of each. A
/// measurement, run by hand in release on an otherwise quiet machine:
/// `RUSTFLAGS="--cfg ci" cargo nextest run -p featherplay-audio --lib --release -E 'test(crossfade_cpu)' --run-ignored only --no-capture`
#[test]
#[ignore = "a measurement; run it in release"]
fn crossfade_cpu() {
    let opus = StreamSource {
        mime: WEBM.into(),
        duration: Some(Duration::from_secs(10)),
        loudness_db: None,
        ..local("tone-10s.webm")
    };
    let aac = StreamSource {
        duration: Some(Duration::from_secs(10)),
        loudness_db: None,
        ..local("tone-10s.m4a")
    };
    let engine = crate::AudioEngine::new_null(engine_tmp());
    engine.set_crossfade(true);
    let events = engine.events();
    engine.load(aac.clone(), Duration::ZERO);
    engine.play();
    wait_event(&events, Duration::from_secs(3), crate::AudioEvent::Started);
    engine.preload(opus.clone());
    // Both rings full and both tails scanned.
    std::thread::sleep(Duration::from_millis(1000));
    let per_s = |span: Duration| {
        let cpu = process_cpu();
        std::thread::sleep(span);
        (process_cpu() - cpu).as_secs_f64() * 1000.0 / span.as_secs_f64()
    };
    let alone = per_s(Duration::from_secs(3));
    engine.crossfade_to(opus, Duration::from_secs(5));
    let fading = per_s(Duration::from_secs(5));
    std::thread::sleep(Duration::from_millis(200));
    let after = per_s(Duration::from_secs(3));
    println!(
        "process CPU, ms per second: AAC alone {alone:.2}, crossfading {fading:.2}, Opus alone {after:.2}"
    );
    for (name, mime) in [("tone-10s.m4a", "audio/mp4"), ("tone-10s.webm", WEBM)] {
        let env = env();
        let (media, n, _) = env.open(&local(name).url);
        let cpu = process_cpu();
        let wall = Instant::now();
        let end = crate::tail::music_end(
            media,
            n,
            mime,
            Duration::from_secs(10),
            &AtomicBool::new(false),
        );
        println!(
            "tail scan of 10 s of {name}: {:.1} ms CPU, {:.1} ms wall, music ends at {end:?}",
            (process_cpu() - cpu).as_secs_f64() * 1000.0,
            wall.elapsed().as_secs_f64() * 1000.0
        );
    }
}

#[test]
fn engine_fade_out_then_crossfade_brings_the_next_track_in() {
    let (engine, events) = playing_with_next(Duration::ZERO);
    // Fade on skip: the old track goes, and the next one comes.
    engine.fade_out(Duration::from_secs(1));
    engine.crossfade_to(local("tone-10s-frag.m4a"), Duration::from_secs(1));
    wait_event(&events, Duration::from_secs(1), crate::AudioEvent::Started);
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(engine.sources(), 2);
    std::thread::sleep(Duration::from_millis(1200));
    assert_eq!(engine.sources(), 1);
}
