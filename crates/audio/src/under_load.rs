//! Playback with every core busy, on the null output (consumed in real time, no device):
//! how often the output callback came after its 46 ms buffer had run out, and how often
//! the ring ran dry. A measurement, run by hand on an otherwise quiet machine:
//!
//! ```text
//! RUSTFLAGS="--cfg ci" cargo nextest run -p featherplay-audio --lib \
//!     -E 'test(under_load::)' --run-ignored only --no-capture
//! ```
//!
//! Each round plays the 10 s tone for 8 s, first on a quiet machine, then beside another
//! process that busy-loops one thread per logical core at normal priority (8.5 s of burn
//! per round). `UNDER_LOAD_ROUNDS` sets the rounds (default 1); keep it at 3 or fewer, so
//! the machine is never full for more than ~30 s in all. The master bus runs at its most
//! work: the equalizer on Bass and the compressor on Strong.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use crate::glitches::Counts;
use crate::{AudioEngine, AudioEvent, Compressor, StreamSource};

const PLAY: Duration = Duration::from_secs(8);

fn tone() -> StreamSource {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tone-10s.m4a");
    let path = path
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/");
    StreamSource {
        url: format!("file:///{}", path.trim_start_matches('/')),
        mime: "audio/mp4".into(),
        content_length: None,
    }
}

fn tmp() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../target/tmp/featherplay-audio-load"
    ))
}

/// Spins one thread per logical core at normal priority until `stop` is set.
fn spin(stop: &AtomicBool) {
    let cores = std::thread::available_parallelism().map_or(4, |n| n.get());
    std::thread::scope(|s| {
        for _ in 0..cores {
            s.spawn(|| {
                let mut x = 0u64;
                while !stop.load(Ordering::Relaxed) {
                    for i in 0..10_000u64 {
                        x = std::hint::black_box(x.wrapping_mul(6_364_136_223_846_793_005) ^ i);
                    }
                }
            });
        }
    });
}

/// The CPU load, in another process as a real one would be: this test binary running
/// `burner` for `for_`.
fn burn(for_: Duration) -> std::process::Child {
    std::process::Command::new(std::env::current_exe().expect("test binary"))
        .args(["--exact", "under_load::burner", "--ignored", "--nocapture"])
        .env("UNDER_LOAD_BURN_MS", for_.as_millis().to_string())
        .stdout(std::process::Stdio::null())
        .spawn()
        .expect("burner")
}

/// The other process of `burn`; does nothing when run on its own.
#[test]
#[ignore = "the CPU load for `playback_beside_a_full_cpu`, started by it"]
fn burner() {
    let Some(ms) = std::env::var("UNDER_LOAD_BURN_MS")
        .ok()
        .and_then(|v| v.parse().ok())
    else {
        return;
    };
    let stop = Arc::new(AtomicBool::new(false));
    let timer = {
        let stop = stop.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(ms));
            stop.store(true, Ordering::Relaxed);
        })
    };
    spin(&stop);
    let _ = timer.join();
}

/// Plays the tone for `PLAY` from the start, polling the position every 250 ms as the app's
/// controller does, and returns the counters it added.
fn play(engine: &AudioEngine, events: &std::sync::mpsc::Receiver<AudioEvent>) -> Counts {
    engine.load(tone(), Duration::ZERO);
    engine.play();
    let t = Instant::now();
    loop {
        match events.recv_timeout(Duration::from_secs(5)) {
            Ok(AudioEvent::Started) => break,
            Ok(AudioEvent::Error(e)) => panic!("engine error: {e}"),
            Ok(_) => {}
            Err(_) => panic!("no Started within 5 s"),
        }
    }
    let before = engine.glitches();
    engine.reset_max_gap();
    while t.elapsed() < PLAY {
        std::thread::sleep(Duration::from_millis(250));
        let _ = engine.position();
    }
    let after = engine.glitches();
    engine.pause();
    Counts {
        late: after.late - before.late,
        max_gap: after.max_gap,
        starved: after.starved - before.starved,
    }
}

#[test]
#[ignore = "a measurement: busy-loops every core, see the module docs"]
fn playback_beside_a_full_cpu() {
    let rounds: usize = std::env::var("UNDER_LOAD_ROUNDS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1);
    let engine = AudioEngine::new_null(tmp());
    engine.set_equalizer(Some([8.0, 0.0, 0.0, 0.0, 0.0]));
    engine.set_compressor(Compressor::Strong);
    let events = engine.events();
    let show = |what: &str, c: Counts| {
        println!(
            "{what:<15} late callbacks {:>3}, longest gap {:>6.1} ms, ring ran dry {:>3}",
            c.late,
            c.max_gap.as_secs_f64() * 1000.0,
            c.starved
        );
    };
    for _ in 0..rounds {
        show("quiet", play(&engine, &events));
        let mut burner = burn(PLAY + Duration::from_millis(500));
        // Let every burning thread start.
        std::thread::sleep(Duration::from_millis(300));
        let loaded = play(&engine, &events);
        let _ = burner.wait();
        show("all cores busy", loaded);
    }
}
