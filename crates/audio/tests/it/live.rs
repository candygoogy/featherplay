//! Live check against YouTube: one resolve, about 3 s of quiet playback. Run with:
//! `FEATHERPLAY_HOME=<your dev home> cargo test -p featherplay-audio --test it -- live:: --ignored --nocapture`
//! Uses the sidecars in `{FEATHERPLAY_HOME}/sidecars` and buffers into `{FEATHERPLAY_HOME}/tmp`.

use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use featherplay_audio::{AudioEngine, AudioEvent, StreamSource};
use featherplay_core::model::TrackId;
use featherplay_streams::{Sidecars, StreamResolver, YtDlpResolver};

const ID: &str = "Jb6gcoR266U";

/// Bytes downloaded so far, from the size of this engine's temp files.
fn downloaded(tmp: &Path) -> u64 {
    std::fs::read_dir(tmp)
        .map(|d| {
            d.flatten()
                .filter(|e| {
                    e.file_name()
                        .to_string_lossy()
                        .starts_with("featherplay-audio-")
                })
                // Not `DirEntry::metadata`: on Windows that is the directory's cached size.
                .filter_map(|e| std::fs::metadata(e.path()).ok())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}

fn drain(rx: &Receiver<AudioEvent>, t0: Instant) {
    for ev in rx.try_iter() {
        println!("[{:6.3}] event {ev:?}", t0.elapsed().as_secs_f64());
        if let AudioEvent::Error(e) = ev {
            panic!("engine error: {e}");
        }
    }
}

#[test]
#[ignore = "live: one yt-dlp resolve and ~3 s of streaming at volume 0.1"]
fn live_stream_starts_fast_and_seeks_cold() {
    let home = PathBuf::from(std::env::var("FEATHERPLAY_HOME").expect("set FEATHERPLAY_HOME"));
    let tmp = home.join("tmp");
    let resolver = YtDlpResolver::new(
        Sidecars {
            dir: home.join("sidecars"),
        },
        None,
    );
    let t = Instant::now();
    let s = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(resolver.resolve(&TrackId(ID.into())))
        .expect("resolve");
    println!(
        "resolved in {:.2} s: itag {}, {:?} bytes, {:?}",
        t.elapsed().as_secs_f64(),
        s.itag,
        s.content_length,
        s.duration
    );
    let len = s.content_length.expect("content length");
    let dur = s.duration.expect("duration");

    let engine = AudioEngine::new(None, &tmp).expect("engine");
    let events = engine.events();
    engine.set_volume(0.1);
    let t0 = Instant::now();
    engine.load(
        StreamSource {
            url: s.url,
            mime: s.mime,
            content_length: s.content_length,
        },
        Duration::ZERO,
    );
    engine.play();
    loop {
        match events.recv_timeout(Duration::from_secs(3).saturating_sub(t0.elapsed())) {
            Ok(AudioEvent::Started) => break,
            Ok(AudioEvent::Error(e)) => panic!("engine error: {e}"),
            Ok(ev) => println!("[{:6.3}] event {ev:?}", t0.elapsed().as_secs_f64()),
            Err(_) => panic!("no Started within 3 s"),
        }
    }
    let started = t0.elapsed();
    println!(
        "[{:6.3}] Started (time to Started: {} ms)",
        started.as_secs_f64(),
        started.as_millis()
    );
    assert!(started <= Duration::from_secs(3));

    // Seek at once, while only the first chunks are on disk.
    let target = Duration::from_secs(120);
    let target_byte = (len as f64 * target.as_secs_f64() / dur.as_secs_f64()) as u64;
    let have = downloaded(&tmp);
    println!(
        "[{:6.3}] seek to 120 s (~byte {target_byte}); downloaded so far {have} bytes => {}",
        t0.elapsed().as_secs_f64(),
        if have < target_byte {
            "cold seek"
        } else {
            "WARM seek (already downloaded)"
        }
    );
    let ts = Instant::now();
    engine.seek(target);
    // `position()` echoes the target until the decoder lands, then plays on from it.
    let landed = loop {
        let p = engine.position();
        if p > target + Duration::from_millis(50) {
            break ts.elapsed();
        }
        assert!(
            ts.elapsed() < Duration::from_secs(5),
            "position stuck at {p:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    println!(
        "[{:6.3}] seek landed: playing past 120.05 s {} ms after seek()",
        t0.elapsed().as_secs_f64(),
        landed.as_millis()
    );
    drain(&events, t0);
    std::thread::sleep(Duration::from_secs(1));
    let p = engine.position();
    println!(
        "[{:6.3}] position {:.3} s, downloaded {} of {len} bytes",
        t0.elapsed().as_secs_f64(),
        p.as_secs_f64(),
        downloaded(&tmp)
    );
    drain(&events, t0);
    assert!(
        p >= Duration::from_millis(120_800) && p <= Duration::from_millis(121_500),
        "position after 1 s: {p:?}"
    );
    engine.pause();
    drop(engine);
    std::thread::sleep(Duration::from_millis(300));
    println!("temp files left after drop: {} bytes", downloaded(&tmp));
}
