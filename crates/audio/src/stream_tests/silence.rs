//! Skip silence on the engine: the silence it skips, and its scan of the whole track.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

use super::webm::WEBM;
use super::{engine_tmp, env, local, near, process_cpu, src, wait_event};

/// `silence-ends.m4a`: 2 s of silence, 2 s of a 440 Hz tone, then 2 s of silence, made with
/// `ffmpeg -f lavfi -i "sine=frequency=440:sample_rate=44100:duration=2,adelay=2s:all=1,
/// apad=pad_dur=2" -ac 2 -c:a aac -b:a 48k -movflags +faststart silence-ends.m4a`.
#[test]
fn engine_skips_the_silence_a_track_starts_and_ends_with() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/silence-ends.m4a");
    let url = format!(
        "file:///{}",
        path.to_string_lossy()
            .replace('\\', "/")
            .trim_start_matches('/')
    );
    let engine = crate::AudioEngine::new_null(engine_tmp());
    engine.set_skip_silences(crate::SkipSilence::StartAndEnd);
    let events = engine.events();
    engine.load(src(&url), Duration::ZERO);
    engine.play();
    wait_event(&events, Duration::from_secs(3), crate::AudioEvent::Skipped);
    let at = engine.position();
    assert!(at >= Duration::from_millis(1900), "position {at:?}");
    let t = Instant::now();
    wait_event(&events, Duration::from_secs(5), crate::AudioEvent::Ended);
    // The tone and half a second of the silence after it, instead of 4 s.
    let tail = t.elapsed();
    assert!(
        (Duration::from_millis(2000)..Duration::from_millis(3300)).contains(&tail),
        "ended {tail:?} after the skip"
    );
}

// Skip silence's scan of the whole track, for the seek bar's marks.

/// The next `Silences` event within `timeout`.
fn next_silences(
    rx: &std::sync::mpsc::Receiver<crate::AudioEvent>,
    timeout: Duration,
) -> Option<(String, crate::Silences)> {
    let t = Instant::now();
    loop {
        match rx.recv_timeout(timeout.saturating_sub(t.elapsed())) {
            Ok(crate::AudioEvent::Silences { url, silences }) => return Some((url, silences)),
            Ok(crate::AudioEvent::Error(e)) => panic!("engine error: {e}"),
            Ok(_) => {}
            Err(_) => return None,
        }
    }
}

#[test]
fn engine_finds_every_silence_of_a_track_while_skip_silence_is_on() {
    let engine = crate::AudioEngine::new_null(engine_tmp());
    engine.set_skip_silences(crate::SkipSilence::StartAndEnd);
    let events = engine.events();
    let track = local("silence-ends.m4a");
    // Paused: the scan needs no playback, only all of the bytes.
    engine.load(track.clone(), Duration::ZERO);
    let (url, found) = next_silences(&events, Duration::from_secs(5)).expect("silences");
    assert_eq!(url, track.url);
    let spans = found.spans(crate::SkipSilence::StartAndEnd);
    // The 2 s intro whole, and the last 1.5 s of the 2 s outro.
    assert_eq!(spans.len(), 2, "{spans:?}");
    for (got, want) in [
        (spans[0].start, 0.0),
        (spans[0].end, 2.0),
        (spans[1].start, 4.5),
        (spans[1].end, 6.0),
    ] {
        near(Some(got), want);
    }
}

#[test]
fn engine_does_not_scan_a_track_whose_silences_the_app_has() {
    let engine = crate::AudioEngine::new_null(engine_tmp());
    engine.set_skip_silences(crate::SkipSilence::StartAndEnd);
    let events = engine.events();
    let track = local("silence-ends.m4a");
    engine.silences_known(&track.url);
    engine.load(track, Duration::ZERO);
    engine.play();
    assert_eq!(next_silences(&events, Duration::from_secs(1)), None);
}

/// The process's commit charge now and at its peak, in bytes.
fn commit() -> (usize, usize) {
    featherplay_platform::memory::commit_charge().unwrap_or((0, 0))
}

/// What skip silence's scan of a whole track costs: CPU, wall time at below-normal
/// priority, and the most memory it added. `SCAN_FILE` names an Opus WebM file (default:
/// the 10 s tone). A measurement, run by hand in release on an otherwise quiet machine:
/// `RUSTFLAGS="--cfg ci" SCAN_FILE=<path> cargo nextest run -p featherplay-audio --lib --release -E 'test(silence_scan_cpu)' --run-ignored only --no-capture`
#[test]
#[ignore = "a measurement; run it in release"]
fn silence_scan_cpu() {
    let url = match std::env::var("SCAN_FILE") {
        Ok(path) => format!(
            "file:///{}",
            path.replace('\\', "/").trim_start_matches('/')
        ),
        Err(_) => local("tone-10s.webm").url,
    };
    let env = env();
    let (media, n, _) = env.open(&url);
    let (before, _) = commit();
    let cpu = process_cpu();
    let wall = Instant::now();
    let found = std::thread::spawn(move || {
        featherplay_platform::thread::lower_priority();
        crate::scan::silences(media, n, WEBM, &AtomicBool::new(false))
    })
    .join()
    .unwrap()
    .expect("scanned");
    let (cpu, wall) = (process_cpu() - cpu, wall.elapsed());
    let (_, peak) = commit();
    let secs = found.frames as f64 / f64::from(found.rate);
    println!(
        "scan of {secs:.1} s: {:.0} ms CPU ({:.0}x realtime), {:.0} ms wall, peak +{} KB, {} runs",
        cpu.as_secs_f64() * 1000.0,
        secs / cpu.as_secs_f64(),
        wall.as_secs_f64() * 1000.0,
        peak.saturating_sub(before) / 1024,
        found.runs.len()
    );
    for mode in [
        crate::SkipSilence::StartAndEnd,
        crate::SkipSilence::Everywhere,
    ] {
        println!("{mode:?}: {:?}", found.spans(mode));
    }
}
