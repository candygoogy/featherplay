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
//! work: the equalizer on Bass and the compressor on Strong, and skip silence looks at every
//! block (Everywhere). Crossfade is on: each track's tail is scanned, and from 2 s to 7 s
//! the tone crossfades into a second one, so two voices decode and sound at once.
//! `UNDER_LOAD_SPEED` sets the playback speed (default 1); above 1 the round's times shrink
//! by it, so the 10 s tones last the round. `UNDER_LOAD_SCAN_FILE` (an Opus WebM file) runs
//! skip silence's whole-track scan of it over and over beside playback, at its below-normal
//! priority, as a long song's scan would run.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::glitches::Counts;
use crate::head::HeadCache;
use crate::http::ErrorSlot;
use crate::media::Media;
use crate::{AudioEngine, AudioEvent, Compressor, SkipSilence, StreamSource};

/// With `UNDER_LOAD_SCAN_FILE`, scans the file until `stop`; the thread returns the scans
/// it finished.
fn scan_beside(stop: Arc<AtomicBool>) -> Option<JoinHandle<u32>> {
    let path = std::env::var("UNDER_LOAD_SCAN_FILE").ok()?;
    let src = StreamSource {
        url: format!(
            "file:///{}",
            path.replace('\\', "/").trim_start_matches('/')
        ),
        mime: "audio/webm".into(),
        content_length: None,
        duration: None,
        loudness_db: None,
    };
    Some(std::thread::spawn(move || {
        crate::priority::lower_scan();
        let rt = tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("runtime");
        let client = crate::http::stream_client(crate::http::READ_TIMEOUT).expect("client");
        let mut done = 0;
        while !stop.load(Ordering::Relaxed) {
            let opened = Media::open(
                &src,
                rt.handle(),
                &client,
                &tmp(),
                &HeadCache::default(),
                ErrorSlot::default(),
            )
            .expect("open");
            if crate::scan::silences(opened.media, opened.len, &src.mime, &stop).is_some() {
                done += 1;
            }
        }
        done
    }))
}

const PLAY: Duration = Duration::from_secs(8);
/// When in each round the crossfade starts, and how long it takes.
const FADE_AT: Duration = Duration::from_secs(2);
const FADE: Duration = Duration::from_secs(5);

fn tone() -> StreamSource {
    fixture("tone-10s.m4a")
}

fn fixture(name: &str) -> StreamSource {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let path = path
        .to_string_lossy()
        .replace(std::path::MAIN_SEPARATOR, "/");
    StreamSource {
        url: format!("file:///{}", path.trim_start_matches('/')),
        mime: "audio/mp4".into(),
        content_length: None,
        duration: Some(Duration::from_secs(10)),
        loudness_db: None,
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

/// Plays the tone for `PLAY` (divided by `faster`) from the start, polling the position
/// every 250 ms as the app's controller does, and returns the counters it added.
fn play(
    engine: &AudioEngine,
    events: &std::sync::mpsc::Receiver<AudioEvent>,
    faster: f64,
) -> Counts {
    let (play_for, fade_at, fade) = (
        PLAY.div_f64(faster),
        FADE_AT.div_f64(faster),
        FADE.div_f64(faster),
    );
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
    let next = fixture("tone-10s-frag.m4a");
    engine.preload(next.clone());
    let mut faded = false;
    while t.elapsed() < play_for {
        std::thread::sleep(Duration::from_millis(250));
        let _ = engine.playhead();
        if !faded && t.elapsed() >= fade_at {
            faded = true;
            engine.crossfade_to(next.clone(), fade);
        }
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
    engine.set_skip_silences(SkipSilence::Everywhere);
    engine.set_crossfade(true);
    let speed: f32 = std::env::var("UNDER_LOAD_SPEED")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(1.0);
    engine.set_speed(speed);
    println!("at {speed}x");
    let faster = f64::from(speed).max(1.0);
    let events = engine.events();
    let show = |what: &str, c: Counts| {
        println!(
            "{what:<15} late callbacks {:>3}, longest gap {:>6.1} ms, ring ran dry {:>3}",
            c.late,
            c.max_gap.as_secs_f64() * 1000.0,
            c.starved
        );
    };
    let stop_scans = Arc::new(AtomicBool::new(false));
    let scans = scan_beside(stop_scans.clone());
    for _ in 0..rounds {
        show("quiet", play(&engine, &events, faster));
        let mut burner = burn(PLAY.div_f64(faster) + Duration::from_millis(500));
        // Let every burning thread start.
        std::thread::sleep(Duration::from_millis(300));
        let loaded = play(&engine, &events, faster);
        let _ = burner.wait();
        show("all cores busy", loaded);
    }
    stop_scans.store(true, Ordering::Relaxed);
    if let Some(scans) = scans {
        println!(
            "whole-track scans beside it: {}",
            scans.join().expect("scans")
        );
    }
}
