//! With no output device (the speakers switched off): the engine starts anyway, and `play()`
//! opens a device once there is one.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::{engine_tmp, fixture, serve, src, wait_event};

fn unplugged() -> (crate::AudioEngine, Arc<AtomicBool>) {
    let plugged = Arc::new(AtomicBool::new(false));
    let engine = crate::AudioEngine::new_fake_device(engine_tmp(), plugged.clone())
        .expect("an engine with no output device");
    (engine, plugged)
}

#[test]
fn engine_starts_with_no_output_device() {
    let plugged = Arc::new(AtomicBool::new(false));
    let engine = crate::AudioEngine::new_fake_device(engine_tmp(), plugged);
    assert!(engine.is_ok(), "start failed: {:?}", engine.err());
}

#[test]
fn engine_play_with_no_output_device_says_so() {
    let server = serve(fixture("tone-10s-frag.m4a"), 1 << 20, Duration::ZERO);
    let (engine, _plugged) = unplugged();
    let events = engine.events();
    engine.load(src(&server.url), Duration::ZERO);
    engine.play();
    wait_event(&events, Duration::from_secs(3), crate::AudioEvent::NoDevice);
}

#[test]
fn engine_plays_on_a_device_that_appears_after_start() {
    let server = serve(fixture("tone-10s-frag.m4a"), 1 << 20, Duration::ZERO);
    let (engine, plugged) = unplugged();
    let events = engine.events();
    engine.load(src(&server.url), Duration::ZERO);
    engine.play();
    wait_event(&events, Duration::from_secs(3), crate::AudioEvent::NoDevice);
    plugged.store(true, Ordering::SeqCst);
    engine.play();
    wait_event(&events, Duration::from_secs(3), crate::AudioEvent::Started);
}

#[test]
#[cfg_attr(
    target_os = "linux",
    ignore = "Linux opens a new output instead (`REOPEN_IDLE`), which it doesn't start"
)]
fn engine_wake_starts_an_output_stopped_a_while_and_it_stops_again() {
    let engine = crate::AudioEngine::new_null(engine_tmp());
    let starts = |n: u32| {
        let until = Instant::now() + Duration::from_secs(2);
        while engine.output_starts() < n && Instant::now() < until {
            std::thread::sleep(Duration::from_millis(5));
        }
        engine.output_starts()
    };
    // Opened, never started: woken once it's been stopped `WAKE_AFTER` (300 ms in tests).
    engine.wake();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(engine.output_starts(), 0, "woken while just stopped");
    std::thread::sleep(Duration::from_millis(300));
    engine.wake();
    assert_eq!(starts(1), 1, "not woken");
    // It stops `STOP_AFTER` (1 s) later, as in a pause; then idle again, it wakes again.
    std::thread::sleep(Duration::from_millis(1800));
    engine.wake();
    assert_eq!(starts(2), 2, "it didn't stop, or didn't wake again");
}

#[test]
fn engine_wake_does_nothing_while_playing() {
    let engine = crate::AudioEngine::new_null(engine_tmp());
    std::thread::sleep(Duration::from_millis(350));
    engine.play();
    let until = Instant::now() + Duration::from_secs(2);
    while engine.output_starts() == 0 && Instant::now() < until {
        std::thread::sleep(Duration::from_millis(5));
    }
    let started = engine.output_starts();
    engine.wake();
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(engine.output_starts(), started);
}
