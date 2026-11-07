//! Offline engine tests against `fixtures/tone-10s.m4a` (440 Hz sine, AAC in MP4).
//! They open the real output device, so they play briefly at a low volume.
//! Run with `RUSTFLAGS="--cfg ci"` to skip them on machines without one.

use std::path::PathBuf;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use featherplay_audio::{AudioEngine, AudioEvent, StreamSource};

/// Slider position for tests that make sound (gain 0.2³ = 0.008).
const QUIET: f32 = 0.2;

fn tone() -> StreamSource {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tone-10s.m4a");
    let path = path
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/");
    StreamSource {
        url: format!("file:///{}", path.trim_start_matches('/')),
        mime: "audio/mp4".into(),
        content_length: None,
        duration: None,
    }
}

fn tmp_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("featherplay-audio")
}

/// Waits for the first event matching `want`, failing on `Error` or after `timeout`.
fn wait_for(
    rx: &Receiver<AudioEvent>,
    timeout: Duration,
    want: impl Fn(&AudioEvent) -> bool,
) -> AudioEvent {
    let deadline = Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match rx.recv_timeout(left) {
            Ok(AudioEvent::Error(e)) => panic!("engine error: {e}"),
            Ok(ev) if want(&ev) => return ev,
            Ok(_) => {}
            Err(e) => panic!("no matching event within {timeout:?}: {e}"),
        }
    }
}

#[test]
#[cfg_attr(ci, ignore = "needs an output device")]
fn local_file_plays_and_ends() {
    let engine = AudioEngine::new(None, tmp_dir()).expect("engine");
    let events = engine.events();
    engine.set_volume(QUIET);
    let t = Instant::now();
    engine.load(tone(), Duration::from_secs(9));
    engine.play();
    wait_for(&events, Duration::from_secs(3), |e| {
        matches!(e, AudioEvent::Started)
    });
    let started = t.elapsed();
    let left = Duration::from_secs(3).saturating_sub(started);
    wait_for(&events, left, |e| matches!(e, AudioEvent::Ended));
    println!("Started after {started:?}, Ended after {:?}", t.elapsed());
}

#[test]
#[cfg_attr(ci, ignore = "needs an output device")]
fn seek_moves_position() {
    let engine = AudioEngine::new(None, tmp_dir()).expect("engine");
    let events = engine.events();
    engine.set_volume(QUIET);
    engine.load(tone(), Duration::ZERO);
    engine.play();
    wait_for(&events, Duration::from_secs(3), |e| {
        matches!(e, AudioEvent::Started)
    });
    engine.seek(Duration::from_secs(5));
    let now = engine.position();
    assert!(
        now >= Duration::from_secs(5) && now <= Duration::from_millis(5300),
        "right after seek: {now:?}"
    );
    std::thread::sleep(Duration::from_millis(150));
    let later = engine.position();
    // Above 5.0 means it's the playing voice reporting, not the seek target echoed back.
    assert!(
        later > Duration::from_secs(5) && later <= Duration::from_millis(5300),
        "150 ms after seek: {later:?}"
    );
    engine.pause();
}

#[test]
#[cfg_attr(ci, ignore = "needs an output device")]
fn unknown_device_falls_back_to_default() {
    let engine = AudioEngine::new(Some("featherplay test: no such device"), tmp_dir())
        .expect("falls back to default");
    let events = engine.events();
    // Silent: the check is that samples flow through the fallback device.
    engine.set_volume(0.0);
    engine.load(tone(), Duration::from_secs(9));
    engine.play();
    wait_for(&events, Duration::from_secs(3), |e| {
        matches!(e, AudioEvent::Started)
    });
    // Switching to an unknown name at runtime also lands on the default device and keeps playing.
    engine.set_device(Some("featherplay test: still no such device"));
    wait_for(&events, Duration::from_secs(3), |e| {
        matches!(e, AudioEvent::Ended)
    });
}

/// Longer than the engine's 1 s before a paused output stream stops.
const STREAM_STOPPED: Duration = Duration::from_millis(1500);

/// Resumes and returns how long until `position()` moves past `from`, i.e. until the
/// output takes audio from the track again.
fn resume_latency(engine: &AudioEngine, from: Duration) -> Duration {
    let t = Instant::now();
    engine.play();
    while engine.position() <= from {
        assert!(
            t.elapsed() < Duration::from_secs(2),
            "no audio after resume"
        );
        std::thread::sleep(Duration::from_millis(1));
    }
    t.elapsed()
}

#[test]
#[cfg_attr(ci, ignore = "needs an output device")]
fn pause_then_resume_plays_to_the_end() {
    let engine = AudioEngine::new(None, tmp_dir()).expect("engine");
    let events = engine.events();
    engine.set_volume(0.0);
    engine.load(tone(), Duration::from_secs(6));
    engine.play();
    wait_for(&events, Duration::from_secs(3), |e| {
        matches!(e, AudioEvent::Started)
    });
    std::thread::sleep(Duration::from_millis(300));
    // A short pause: the output stream keeps running.
    engine.pause();
    std::thread::sleep(Duration::from_millis(300));
    let at = engine.position();
    let short = resume_latency(&engine, at);
    std::thread::sleep(Duration::from_millis(300));
    // A long one: the stream stops, and the track doesn't move meanwhile.
    engine.pause();
    std::thread::sleep(Duration::from_millis(100));
    let at = engine.position();
    std::thread::sleep(STREAM_STOPPED);
    assert_eq!(engine.position(), at, "moved while paused");
    let long = resume_latency(&engine, at);
    println!("resume latency: {short:?} with the stream running, {long:?} with it stopped");
    assert!(long < Duration::from_millis(500), "slow resume: {long:?}");
    wait_for(&events, Duration::from_secs(4), |e| {
        matches!(e, AudioEvent::Ended)
    });
}

#[test]
#[cfg_attr(ci, ignore = "needs an output device")]
fn seek_and_device_change_while_paused_then_resume() {
    let engine = AudioEngine::new(None, tmp_dir()).expect("engine");
    let events = engine.events();
    engine.set_volume(0.0);
    engine.load(tone(), Duration::ZERO);
    engine.play();
    wait_for(&events, Duration::from_secs(3), |e| {
        matches!(e, AudioEvent::Started)
    });
    engine.pause();
    std::thread::sleep(STREAM_STOPPED);
    engine.seek(Duration::from_secs(8));
    // Reopens the (default) device; the new stream stays stopped until `play()`.
    engine.set_device(Some("featherplay test: no such device"));
    std::thread::sleep(Duration::from_millis(300));
    let at = engine.position();
    assert!(
        at >= Duration::from_secs(8) && at < Duration::from_millis(8100),
        "paused after the seek: {at:?}"
    );
    resume_latency(&engine, at);
    let later = engine.position();
    assert!(
        later < Duration::from_millis(8500),
        "after resuming: {later:?}"
    );
    wait_for(&events, Duration::from_secs(3), |e| {
        matches!(e, AudioEvent::Ended)
    });
}
