//! Progressive playback against a throttled local HTTP server: no output device and no
//! network. `tone-10s-frag.m4a` is `tone-10s.m4a` laid out like the DASH m4a streams
//! (`ftyp`, `moov`, `sidx`, then a `moof` + `mdat` per second), made with
//! `ffmpeg -i tone-10s.m4a -c:a copy -f mp4 -movflags +dash+global_sidx -frag_duration 1000000`.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::StreamSource;
use crate::decoder::Decoder;
use crate::head::{self, HeadCache};
use crate::http::ErrorSlot;
use crate::media::{Media, Opened};

fn fixture(name: &str) -> Vec<u8> {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    std::fs::read(path).unwrap()
}

#[derive(Default)]
struct Log {
    /// Start offsets of the `Range` requests, in order.
    starts: Mutex<Vec<u64>>,
    sent: AtomicU64,
    /// Requests left to refuse with a 403 (the ad gate).
    refuse: AtomicU32,
    /// Answer with the whole file (200), as a server that ignores `Range` does.
    whole: std::sync::atomic::AtomicBool,
    /// Requests left that stall: they send the body up to byte `stall_at`, then nothing
    /// more, with the connection left open (as a half-open connection does).
    stall: AtomicU32,
    stall_at: AtomicU64,
}

struct Server {
    url: String,
    log: Arc<Log>,
}

/// Serves `data` with `Range` support at `rate` bytes/s, after `delay` per request.
fn serve(data: Vec<u8>, rate: u64, delay: Duration) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/track.m4a", listener.local_addr().unwrap());
    let log = Arc::new(Log::default());
    let data = Arc::new(data);
    let l = log.clone();
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(conn) = conn else { return };
            let (data, log) = (data.clone(), l.clone());
            std::thread::spawn(move || answer(conn, &data, &log, rate, delay));
        }
    });
    Server { url, log }
}

fn answer(conn: TcpStream, data: &[u8], log: &Log, rate: u64, delay: Duration) {
    let mut reader = BufReader::new(conn.try_clone().unwrap());
    let mut range = None;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 {
            return;
        }
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some(v) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
            let (a, b) = v.split_once('-').unwrap();
            let a: u64 = a.parse().unwrap();
            let b = b.parse::<u64>().unwrap_or(data.len() as u64 - 1);
            range = Some((a, b.min(data.len() as u64 - 1)));
        }
    }
    let mut conn = conn;
    std::thread::sleep(delay);
    if log
        .refuse
        .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
        .is_ok()
    {
        let _ = conn
            .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
        return;
    }
    let whole = log.whole.load(Ordering::SeqCst);
    let (a, b) = match range {
        Some(r) if !whole => r,
        _ => (0, data.len() as u64 - 1),
    };
    log.starts.lock().unwrap().push(a);
    let head = if whole {
        format!(
            "HTTP/1.1 200 OK\r\nContent-Type: audio/mp4\r\nContent-Length: {}\r\n\
             Connection: close\r\n\r\n",
            data.len()
        )
    } else {
        format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Type: audio/mp4\r\nContent-Length: {}\r\n\
             Content-Range: bytes {a}-{b}/{}\r\nConnection: close\r\n\r\n",
            b - a + 1,
            data.len()
        )
    };
    if conn.write_all(head.as_bytes()).is_err() {
        return;
    }
    let stall = log
        .stall
        .try_update(Ordering::SeqCst, Ordering::SeqCst, |n| n.checked_sub(1))
        .is_ok();
    // The end of the body sent: the range's, or the stall's.
    let stop = if stall {
        (b + 1).min(log.stall_at.load(Ordering::SeqCst)).max(a)
    } else {
        b + 1
    };
    // 50 sends a second.
    let piece = (rate / 50).max(1) as usize;
    for part in data[a as usize..stop as usize].chunks(piece) {
        if conn.write_all(part).is_err() {
            return;
        }
        log.sent.fetch_add(part.len() as u64, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(20));
    }
    if stall {
        // The connection stays open and silent for longer than any test runs.
        std::thread::sleep(Duration::from_secs(120));
    }
}

struct Env {
    rt: tokio::runtime::Runtime,
    client: reqwest::Client,
    tmp: PathBuf,
    heads: HeadCache,
}

fn env() -> Env {
    env_with_read_timeout(crate::http::READ_TIMEOUT)
}

fn env_with_read_timeout(read_timeout: Duration) -> Env {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    Env {
        rt,
        client: crate::http::stream_client(read_timeout).unwrap(),
        tmp: PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../target/tmp/featherplay-audio-unit"
        )),
        heads: HeadCache::default(),
    }
}

impl Env {
    fn open(&self, url: &str) -> (Media, u64, ErrorSlot) {
        let src = StreamSource {
            url: url.into(),
            mime: "audio/mp4".into(),
            content_length: None,
        };
        let error = ErrorSlot::default();
        let Opened { media, len, .. } = Media::open(
            &src,
            self.rt.handle(),
            &self.client,
            &self.tmp,
            &self.heads,
            error.clone(),
        )
        .expect("open");
        (media, len, error)
    }
}

/// Decodes to the end; returns the seconds of audio.
fn drain(dec: &mut Decoder) -> f64 {
    let per_s = dec.sample_rate().get() as f64 * dec.channels().get() as f64;
    let mut n = 0u64;
    while dec.next().is_some() {
        n += 1;
    }
    n as f64 / per_s
}

#[test]
fn first_audio_long_before_the_download_ends() {
    let data = fixture("tone-10s-frag.m4a");
    let len = data.len() as u64;
    // ~84 KB at 16 KB/s: the whole file takes ~5 s.
    let server = serve(data, 16 << 10, Duration::ZERO);
    let env = env();
    let t = Instant::now();
    let (media, n, _) = env.open(&server.url);
    assert_eq!(n, len);
    let mut dec = Decoder::open(media, n, "audio/mp4").expect("decoder");
    dec.next().expect("a first sample");
    let first = t.elapsed();
    let sent = server.log.sent.load(Ordering::SeqCst);
    assert!(
        first < Duration::from_millis(1500),
        "first sample after {first:?}"
    );
    assert!(
        sent < len / 3,
        "{sent} of {len} bytes sent before the first sample"
    );
    let secs = drain(&mut dec);
    assert!((secs - 10.0).abs() < 0.1, "{secs} s decoded");
}

#[test]
fn seek_ahead_fetches_the_target_instead_of_waiting() {
    let data = fixture("tone-10s-frag.m4a");
    let server = serve(data, 16 << 10, Duration::ZERO);
    let env = env();
    let (mut media, n, _) = env.open(&server.url);
    // As if the file were large: a jump this far ahead is a new `Range` request.
    media.set_jump_ahead(0);
    let mut dec = Decoder::open(media, n, "audio/mp4").expect("decoder");
    let t = Instant::now();
    dec.try_seek(Duration::from_millis(8500)).expect("seek");
    dec.next().expect("a sample after the seek");
    let took = t.elapsed();
    // Reading through to 8.5 s at 16 KB/s would take ~4 s.
    assert!(took < Duration::from_millis(1500), "seek took {took:?}");
    let starts = server.log.starts.lock().unwrap().clone();
    assert!(
        starts.iter().any(|&s| s > n / 2),
        "no request near the target: {starts:?}"
    );
    let left = drain(&mut dec);
    assert!(
        (left - 1.5).abs() < 0.1,
        "{left} s left after seeking to 8.5 s"
    );
    // Back before the jump: the hole it left is fetched, never read as garbage.
    dec.try_seek(Duration::from_secs(1)).expect("seek back");
    let left = drain(&mut dec);
    assert!(
        (left - 9.0).abs() < 0.1,
        "{left} s left after seeking back to 1 s"
    );
}

#[test]
fn seek_back_and_past_the_end() {
    let data = fixture("tone-10s-frag.m4a");
    let server = serve(data, 1 << 20, Duration::ZERO);
    let env = env();
    let (media, n, _) = env.open(&server.url);
    let mut dec = Decoder::open(media, n, "audio/mp4").expect("decoder");
    dec.try_seek(Duration::from_secs(6)).expect("seek");
    dec.next().unwrap();
    dec.try_seek(Duration::from_millis(2250))
        .expect("seek back");
    let left = drain(&mut dec);
    assert!(
        (left - 7.75).abs() < 0.1,
        "{left} s left after seeking to 2.25 s"
    );
    dec.try_seek(Duration::from_secs(30))
        .expect("seek past the end");
    assert_eq!(dec.next(), None);
    dec.try_seek(Duration::ZERO).expect("seek to the start");
    let all = drain(&mut dec);
    assert!((all - 10.0).abs() < 0.1, "{all} s after seeking to 0");
}

#[test]
fn unfragmented_local_file_plays_and_seeks() {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tone-10s.m4a");
    let url = format!(
        "file:///{}",
        path.to_string_lossy()
            .replace('\\', "/")
            .trim_start_matches('/')
    );
    let env = env();
    let (media, n, _) = env.open(&url);
    let mut dec = Decoder::open(media, n, "audio/mp4").expect("decoder");
    dec.try_seek(Duration::from_secs(9)).expect("seek");
    let left = drain(&mut dec);
    assert!(
        (left - 1.0).abs() < 0.1,
        "{left} s left after seeking to 9 s"
    );
}

#[test]
fn a_fetched_head_starts_without_the_network() {
    let data = fixture("tone-10s-frag.m4a");
    // Every request takes 2 s to answer.
    let server = serve(data, 1 << 20, Duration::from_secs(2));
    let env = env();
    head::prefetch(&env.heads, env.rt.handle(), &env.client, server.url.clone());
    let deadline = Instant::now() + Duration::from_secs(10);
    while env.heads.lock().unwrap().get(&server.url).is_none() {
        assert!(Instant::now() < deadline, "the head never arrived");
        std::thread::sleep(Duration::from_millis(20));
    }
    let t = Instant::now();
    let (media, n, _) = env.open(&server.url);
    let mut dec = Decoder::open(media, n, "audio/mp4").expect("decoder");
    dec.next().expect("a first sample");
    let first = t.elapsed();
    assert!(
        first < Duration::from_millis(300),
        "first sample after {first:?}"
    );
    // The whole fixture fits in the head; it plays to the end.
    let secs = drain(&mut dec);
    assert!((secs - 10.0).abs() < 0.1, "{secs} s decoded");
}

#[test]
fn a_server_that_ignores_range_still_plays() {
    let data = fixture("tone-10s-frag.m4a");
    let server = serve(data, 1 << 20, Duration::ZERO);
    server.log.whole.store(true, Ordering::SeqCst);
    let env = env();
    let (media, n, _) = env.open(&server.url);
    let mut dec = Decoder::open(media, n, "audio/mp4").expect("decoder");
    let secs = drain(&mut dec);
    assert!((secs - 10.0).abs() < 0.1, "{secs} s decoded");
}

#[test]
fn a_short_gate_is_waited_out() {
    let data = fixture("tone-10s-frag.m4a");
    let server = serve(data, 1 << 20, Duration::ZERO);
    // Refused twice, as a URL is just before its `available_at`.
    server.log.refuse.store(2, Ordering::SeqCst);
    let env = env();
    let (media, n, _) = env.open(&server.url);
    let mut dec = Decoder::open(media, n, "audio/mp4").expect("decoder");
    assert!(dec.next().is_some());
}

/// Read timeout for the stall tests: short, so they take seconds.
/// It must stay well above how long the local server takes to answer on a loaded machine:
/// with 200 ms, 32 busy loops on 16 cores made requests time out before their first byte,
/// so the retries (and even the open) depended on scheduling.
const SHORT_READ_TIMEOUT: Duration = Duration::from_secs(1);

/// Opens `url` with `SHORT_READ_TIMEOUT` and decodes it to the end on another thread.
/// Gives the seconds decoded and the recorded failure; `Timeout` if it took longer than
/// `limit` (the stream hung), `Disconnected` if the thread panicked (the open failed).
fn decode_with_short_timeout(
    url: String,
    limit: Duration,
) -> Result<(f64, Option<String>), std::sync::mpsc::RecvTimeoutError> {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let env = env_with_read_timeout(SHORT_READ_TIMEOUT);
        let (media, n, error) = env.open(&url);
        let mut dec = Decoder::open(media, n, "audio/mp4").expect("decoder");
        let secs = drain(&mut dec);
        let _ = tx.send((secs, error.lock().unwrap().take()));
    });
    rx.recv_timeout(limit)
}

#[test]
fn a_stream_that_stays_stalled_fails() {
    let data = fixture("tone-10s-frag.m4a");
    let half = data.len() as u64 / 2;
    let server = serve(data, 1 << 20, Duration::ZERO);
    // Every request stops at the middle of the file and hangs there.
    server.log.stall.store(u32::MAX, Ordering::SeqCst);
    server.log.stall_at.store(half, Ordering::SeqCst);
    // 4 reads time out (4 s) between the 3 retry delays (3.25 s).
    let (secs, error) = decode_with_short_timeout(server.url, Duration::from_secs(20))
        .expect("the stalled stream didn't end with a failure");
    assert!(secs < 9.0, "{secs} s decoded from half a file");
    assert!(error.is_some(), "the stall was not reported as a failure");
}

#[test]
fn a_stream_that_stalls_once_resumes_where_it_stopped() {
    let data = fixture("tone-10s-frag.m4a");
    let half = data.len() as u64 / 2;
    let server = serve(data, 1 << 20, Duration::ZERO);
    server.log.stall.store(1, Ordering::SeqCst);
    server.log.stall_at.store(half, Ordering::SeqCst);
    let (secs, error) = decode_with_short_timeout(server.url.clone(), Duration::from_secs(20))
        .expect("the stalled stream didn't play to the end");
    assert_eq!(error, None);
    assert!((secs - 10.0).abs() < 0.1, "{secs} s decoded");
    // How many requests it takes depends on scheduling (a retry that also times out asks
    // again from the same byte); the first one past the start must begin at the stall.
    let starts = server.log.starts.lock().unwrap().clone();
    let resumed = starts.iter().copied().find(|&s| s != 0);
    assert_eq!(resumed, Some(half), "requests: {starts:?}");
}

// The engine end to end, on a null output: load, play, and the `Started` event.

fn engine_tmp() -> PathBuf {
    PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../target/tmp/featherplay-audio-engine"
    ))
}

fn src(url: &str) -> StreamSource {
    StreamSource {
        url: url.into(),
        mime: "audio/mp4".into(),
        content_length: None,
    }
}

/// Waits for `want`; panics on an error event or after `timeout`.
fn wait_event(
    rx: &std::sync::mpsc::Receiver<crate::AudioEvent>,
    timeout: Duration,
    want: crate::AudioEvent,
) -> Duration {
    let t = Instant::now();
    loop {
        let left = timeout.saturating_sub(t.elapsed());
        match rx.recv_timeout(left) {
            Ok(crate::AudioEvent::Error(e)) => panic!("engine error: {e}"),
            Ok(ev) if ev == want => return t.elapsed(),
            Ok(_) => {}
            Err(_) => panic!("no {want:?} within {timeout:?}"),
        }
    }
}

#[test]
fn engine_sounds_long_before_the_download_ends() {
    let data = fixture("tone-10s-frag.m4a");
    let server = serve(data, 16 << 10, Duration::ZERO);
    let engine = crate::AudioEngine::new_null(engine_tmp());
    let events = engine.events();
    let t = Instant::now();
    engine.load(src(&server.url), Duration::ZERO);
    engine.play();
    wait_event(&events, Duration::from_secs(3), crate::AudioEvent::Started);
    let started = t.elapsed();
    assert!(
        started < Duration::from_millis(1500),
        "Started after {started:?}"
    );
    std::thread::sleep(Duration::from_millis(500));
    let at = engine.position();
    assert!(at >= Duration::from_millis(300), "position {at:?}");
}

#[test]
fn engine_starts_a_prefetched_stream_without_the_network() {
    let data = fixture("tone-10s-frag.m4a");
    // Every request takes 1.5 s to answer.
    let server = serve(data, 1 << 20, Duration::from_millis(1500));
    let engine = crate::AudioEngine::new_null(engine_tmp());
    let events = engine.events();
    engine.prefetch(&src(&server.url));
    std::thread::sleep(Duration::from_millis(2500));
    let t = Instant::now();
    engine.load(src(&server.url), Duration::ZERO);
    engine.play();
    wait_event(&events, Duration::from_secs(3), crate::AudioEvent::Started);
    let started = t.elapsed();
    assert!(
        started < Duration::from_millis(300),
        "Started after {started:?}"
    );
    assert_eq!(
        server.log.starts.lock().unwrap().len(),
        1,
        "one request, the prefetch"
    );
}

#[test]
fn engine_starts_a_preloaded_stream_at_once() {
    let data = fixture("tone-10s-frag.m4a");
    let server = serve(data, 1 << 20, Duration::from_millis(1000));
    let engine = crate::AudioEngine::new_null(engine_tmp());
    let events = engine.events();
    engine.preload(src(&server.url));
    std::thread::sleep(Duration::from_millis(2000));
    let t = Instant::now();
    engine.load(src(&server.url), Duration::ZERO);
    engine.play();
    wait_event(&events, Duration::from_secs(3), crate::AudioEvent::Started);
    let started = t.elapsed();
    assert!(
        started < Duration::from_millis(300),
        "Started after {started:?}"
    );
}

#[test]
fn engine_seek_into_the_undownloaded_part_plays_on() {
    let data = fixture("tone-10s-frag.m4a");
    // ~2.6 s for the whole file. A target this close is waited for (a jump costs more).
    let server = serve(data, 32 << 10, Duration::ZERO);
    let engine = crate::AudioEngine::new_null(engine_tmp());
    let events = engine.events();
    engine.load(src(&server.url), Duration::ZERO);
    engine.play();
    wait_event(&events, Duration::from_secs(3), crate::AudioEvent::Started);
    engine.seek(Duration::from_secs(8));
    assert_eq!(engine.position(), Duration::from_secs(8));
    // The stall shows as buffering, then the last 2 s play to the end.
    wait_event(
        &events,
        Duration::from_secs(3),
        crate::AudioEvent::Buffering(true),
    );
    let ended = wait_event(&events, Duration::from_secs(7), crate::AudioEvent::Ended);
    assert!(
        ended > Duration::from_millis(1500),
        "Ended {ended:?} after the seek"
    );
}

#[test]
fn engine_reports_a_stream_that_stays_stalled() {
    let data = fixture("tone-10s-frag.m4a");
    let half = data.len() as u64 / 2;
    let server = serve(data, 1 << 20, Duration::ZERO);
    server.log.stall.store(u32::MAX, Ordering::SeqCst);
    server.log.stall_at.store(half, Ordering::SeqCst);
    let engine = crate::AudioEngine::new_null_with_read_timeout(engine_tmp(), SHORT_READ_TIMEOUT);
    let events = engine.events();
    engine.load(src(&server.url), Duration::ZERO);
    engine.play();
    let t = Instant::now();
    let limit = Duration::from_secs(20);
    loop {
        match events.recv_timeout(limit.saturating_sub(t.elapsed())) {
            Ok(crate::AudioEvent::Error(_)) => break,
            Ok(crate::AudioEvent::Ended) => panic!("half a file ended as if whole"),
            Ok(_) => {}
            Err(_) => panic!("no error within {limit:?}: the stalled stream hung"),
        }
    }
}
