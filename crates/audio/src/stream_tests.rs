//! Progressive playback against a throttled local HTTP server: no output device and no
//! network. `tone-10s-frag.m4a` is `tone-10s.m4a` laid out like the DASH m4a streams
//! (`ftyp`, `moov`, `sidx`, then a `moof` + `mdat` per second), made with
//! `ffmpeg -i tone-10s.m4a -c:a copy -f mp4 -movflags +dash+global_sidx -frag_duration 1000000`.

use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
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
            duration: None,
            loudness_db: None,
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
    head::prefetch(
        &env.heads,
        env.rt.handle(),
        &env.client,
        server.url.clone(),
        |_| {},
    );
    let deadline = Instant::now() + Duration::from_secs(10);
    while env.heads.get(&server.url).is_none() {
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

/// The embedded player's URLs have no ad gate: a 403 on their first request is final, so
/// the app resolves again at once. Waiting 1.75 s first never helped (3 of 3, 2026-11-05).
#[test]
fn a_refused_url_without_an_ad_gate_fails_at_once() {
    let server = serve(fixture("tone-10s-frag.m4a"), 1 << 20, Duration::ZERO);
    server.log.refuse.store(2, Ordering::SeqCst);
    let env = env();
    let url = format!("{}?c=WEB_EMBEDDED_PLAYER&n=x", server.url);
    let opened = env.rt.block_on(crate::http::open(&env.client, &url, None));
    let err = opened.expect_err("refused");
    assert!(err.starts_with("HTTP 403"), "{err}");
    assert_eq!(
        server.log.refuse.load(Ordering::SeqCst),
        1,
        "one request, not tried again"
    );
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

// Opus in WebM (itag 251). `tone-10s.webm` is a stereo 440 Hz sine, 10 s, with its `Cues`
// before the first `Cluster` like YouTube's (a `Cluster` per second), made with
// `ffmpeg -f lavfi -i sine=frequency=440:sample_rate=48000:duration=10 -ac 2 -c:a libopus
// -b:a 96k -f webm -cluster_time_limit 1000 -cues_to_front 1 tone-10s.webm`.

const WEBM: &str = "audio/webm; codecs=\"opus\"";
/// The frames ffmpeg decodes from `tone-10s.webm` (`ffmpeg -i tone-10s.webm -f f32le -`), with
/// the pre-skip (312) and the end padding dropped.
const WEBM_FRAMES: i64 = 480_000;
/// One Opus frame, 20 ms at 48 kHz.
const OPUS_FRAME: i64 = 960;
/// 10 ms at 48 kHz.
const TEN_MS: i64 = 480;

fn open_webm(env: &Env, url: &str) -> Decoder {
    let (media, n, _) = env.open(url);
    Decoder::open(media, n, WEBM).expect("decoder")
}

/// Decodes to the end; returns the interleaved samples.
fn samples(dec: &mut Decoder) -> Vec<f32> {
    dec.by_ref().collect()
}

/// Decodes to the end; returns the frames (samples per channel).
fn frames(dec: &mut Decoder) -> i64 {
    let channels = dec.channels().get() as i64;
    dec.by_ref().count() as i64 / channels
}

#[test]
fn webm_first_audio_long_before_the_download_ends() {
    let data = fixture("tone-10s.webm");
    let len = data.len() as u64;
    // ~150 KB at 16 KB/s: the whole file takes ~9 s.
    let server = serve(data, 16 << 10, Duration::ZERO);
    let env = env();
    let t = Instant::now();
    let (media, n, _) = env.open(&server.url);
    let mut dec = Decoder::open(media, n, WEBM).expect("decoder");
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
    assert_eq!((dec.channels().get(), dec.sample_rate().get()), (2, 48_000));
    // Half a second plays from the first `Cluster`.
    assert_eq!(dec.by_ref().take(48_000).count(), 48_000);
}

#[test]
fn webm_decodes_every_frame_without_the_pre_skip() {
    let server = serve(fixture("tone-10s.webm"), 1 << 20, Duration::ZERO);
    let env = env();
    let mut dec = open_webm(&env, &server.url);
    let n = frames(&mut dec);
    // With the pre-skip left in, it's one Opus frame too many.
    assert!(
        (n - WEBM_FRAMES).abs() < OPUS_FRAME,
        "{n} frames decoded, ffmpeg gives {WEBM_FRAMES}"
    );
}

#[test]
fn webm_tone_has_the_right_pitch() {
    let server = serve(fixture("tone-10s.webm"), 1 << 20, Duration::ZERO);
    let env = env();
    let mut dec = open_webm(&env, &server.url);
    let all = samples(&mut dec);
    let left: Vec<f32> = all.iter().step_by(2).copied().collect();
    let rising = left
        .windows(2)
        .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
        .count();
    let hz = rising as f64 / (left.len() as f64 / 48_000.0);
    assert!((hz - 440.0).abs() < 2.0, "{hz} Hz");
}

#[test]
fn webm_seek_lands_on_the_target_and_back_to_the_start() {
    let server = serve(fixture("tone-10s.webm"), 1 << 20, Duration::ZERO);
    let env = env();
    let mut dec = open_webm(&env, &server.url);
    let whole = samples(&mut dec);
    let total = whole.len() as i64 / 2;
    dec.try_seek(Duration::from_secs(5)).expect("seek");
    let left = frames(&mut dec);
    let landed = total - left;
    assert!(
        (landed - 240_000).abs() <= TEN_MS,
        "landed at frame {landed} for 5 s (240000)"
    );
    dec.try_seek(Duration::ZERO).expect("seek to the start");
    let again = samples(&mut dec);
    assert_eq!(
        again.len(),
        whole.len(),
        "a different length from the start"
    );
    assert!(
        again[..9600] == whole[..9600],
        "the start plays differently"
    );
}

#[test]
fn webm_seeks_twice_before_reading_and_past_the_end() {
    let server = serve(fixture("tone-10s.webm"), 1 << 20, Duration::ZERO);
    let env = env();
    let mut dec = open_webm(&env, &server.url);
    let total = frames(&mut dec);
    // A drag across the seek bar: the second seek wins.
    dec.try_seek(Duration::from_secs(8)).expect("seek");
    dec.try_seek(Duration::from_secs(2)).expect("seek back");
    let landed = total - frames(&mut dec);
    assert!(
        (landed - 96_000).abs() <= TEN_MS,
        "landed at frame {landed} for 2 s (96000)"
    );
    dec.try_seek(Duration::from_secs(30))
        .expect("seek past the end");
    assert_eq!(dec.next(), None);
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
        duration: None,
        loudness_db: None,
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

/// A click on a song resolved just before it: its head is on its way. The load waits for
/// that response instead of sending a second request, which would start later and come
/// later (on a second connection, over the network).
#[test]
fn engine_load_takes_the_head_that_is_on_its_way() {
    let data = fixture("tone-10s-frag.m4a");
    // Every request takes 600 ms to answer.
    let server = serve(data, 1 << 20, Duration::from_millis(600));
    let engine = crate::AudioEngine::new_null(engine_tmp());
    let events = engine.events();
    engine.prefetch(&src(&server.url));
    std::thread::sleep(Duration::from_millis(250));
    let t = Instant::now();
    engine.load(src(&server.url), Duration::ZERO);
    engine.play();
    wait_event(&events, Duration::from_secs(3), crate::AudioEvent::Started);
    let started = t.elapsed();
    let starts = server.log.starts.lock().unwrap().clone();
    assert_eq!(starts, [0], "one request for the start, the prefetch's");
    // The head comes ~350 ms after the load; a request of its own would take 600 ms.
    assert!(
        started < Duration::from_millis(550),
        "Started after {started:?}"
    );
}

/// On a slow link the head takes seconds to come in whole: the load takes its request over
/// and plays from its first bytes, as a request of its own would.
#[test]
fn engine_load_on_a_slow_link_takes_the_head_request_over() {
    let data = fixture("tone-10s-frag.m4a");
    // ~5 s for the whole file.
    let server = serve(data, 16 << 10, Duration::ZERO);
    let engine = crate::AudioEngine::new_null(engine_tmp());
    let events = engine.events();
    engine.prefetch(&src(&server.url));
    std::thread::sleep(Duration::from_millis(250));
    let t = Instant::now();
    engine.load(src(&server.url), Duration::ZERO);
    engine.play();
    wait_event(&events, Duration::from_secs(3), crate::AudioEvent::Started);
    let started = t.elapsed();
    let starts = server.log.starts.lock().unwrap().clone();
    assert_eq!(starts, [0], "one request for the start, the prefetch's");
    assert!(
        started < Duration::from_millis(500),
        "Started after {started:?}"
    );
}

/// A head's answer that comes late still wins if it beats the request the load then sends
/// of its own: the load never comes later than it would have without the head.
#[test]
fn engine_load_takes_a_late_head_if_it_answers_first() {
    let data = fixture("tone-10s-frag.m4a");
    // Every request takes 1.3 s to answer: longer than a load waits for the head's.
    let server = serve(data, 1 << 20, Duration::from_millis(1300));
    let engine = crate::AudioEngine::new_null(engine_tmp());
    let events = engine.events();
    engine.prefetch(&src(&server.url));
    std::thread::sleep(Duration::from_millis(100));
    let t = Instant::now();
    engine.load(src(&server.url), Duration::ZERO);
    engine.play();
    wait_event(&events, Duration::from_secs(4), crate::AudioEvent::Started);
    let started = t.elapsed();
    // The head answers ~1.2 s after the load; a request of its own would answer later.
    assert!(
        started < Duration::from_millis(1600),
        "Started after {started:?}"
    );
}

/// A server that keeps its connections open between requests, as the stream servers do,
/// and counts them.
struct KeepAlive {
    url: String,
    connections: Arc<AtomicU32>,
    /// The paths asked for, in the order they were answered.
    answered: Arc<Mutex<Vec<String>>>,
}

impl KeepAlive {
    /// Waits until `n` requests are answered, and a moment more for the client to take
    /// the last answer in.
    fn wait_answered(&self, n: usize) {
        let t = Instant::now();
        while self.answered.lock().unwrap().len() < n {
            assert!(
                t.elapsed() < Duration::from_secs(5),
                "answer {n} never came"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Serves `data` to `Range` requests and an empty 204 to any other, as the stream servers'
/// `/generate_204`, each after `delay`.
fn serve_keep_alive(data: Vec<u8>, delay: Duration) -> KeepAlive {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/track.m4a", listener.local_addr().unwrap());
    let connections = Arc::new(AtomicU32::new(0));
    let answered = Arc::new(Mutex::new(Vec::new()));
    let data = Arc::new(data);
    let (count, log) = (connections.clone(), answered.clone());
    std::thread::spawn(move || {
        for conn in listener.incoming() {
            let Ok(conn) = conn else { return };
            count.fetch_add(1, Ordering::SeqCst);
            let (data, log) = (data.clone(), log.clone());
            std::thread::spawn(move || keep_answering(conn, &data, &log, delay));
        }
    });
    KeepAlive {
        url,
        connections,
        answered,
    }
}

fn keep_answering(mut conn: TcpStream, data: &[u8], log: &Mutex<Vec<String>>, delay: Duration) {
    let mut reader = BufReader::new(conn.try_clone().unwrap());
    loop {
        let mut path = None;
        let mut range = None;
        loop {
            let mut line = String::new();
            if reader.read_line(&mut line).unwrap_or(0) == 0 {
                // The client closed the connection.
                return;
            }
            let line = line.trim_end();
            if line.is_empty() {
                break;
            }
            if path.is_none() {
                // `GET /track.m4a HTTP/1.1`
                path = line.split(' ').nth(1).map(str::to_owned);
            } else if let Some(v) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                let (a, b) = v.split_once('-').unwrap();
                range = Some((a.parse::<usize>().unwrap(), b.parse::<usize>().unwrap()));
            }
        }
        std::thread::sleep(delay);
        log.lock().unwrap().push(path.unwrap_or_default());
        let sent = match range {
            Some((a, b)) => {
                let b = b.min(data.len() - 1);
                let head = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Type: audio/mp4\r\n\
                     Content-Length: {}\r\nContent-Range: bytes {a}-{b}/{}\r\n\r\n",
                    b - a + 1,
                    data.len()
                );
                conn.write_all(head.as_bytes())
                    .and_then(|()| conn.write_all(&data[a..=b]))
            }
            None => conn.write_all(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n"),
        };
        if sent.is_err() {
            return;
        }
    }
}

/// A click on a song that isn't resolved yet: while its resolve runs, the app has the
/// engine open a connection to the server the stream will likely come from. The stream's
/// first request then goes over it instead of opening its own (~100 ms over TLS).
#[test]
fn engine_load_uses_the_connection_opened_ahead() {
    let server = serve_keep_alive(fixture("tone-10s-frag.m4a"), Duration::ZERO);
    let engine = crate::AudioEngine::new_null(engine_tmp());
    let events = engine.events();
    // The server alone, as the app names it.
    engine.connect(server.url.trim_end_matches("/track.m4a"));
    server.wait_answered(1);
    engine.load(src(&server.url), Duration::ZERO);
    engine.play();
    wait_event(&events, Duration::from_secs(3), crate::AudioEvent::Started);
    assert_eq!(
        *server.answered.lock().unwrap(),
        ["/generate_204", "/track.m4a"]
    );
    assert_eq!(
        server.connections.load(Ordering::SeqCst),
        1,
        "the stream's request opened no connection"
    );
}

/// Clicks in a row each ask for a connection: the one on its way or waiting in the pool
/// serves them all, and no second one opens.
#[test]
fn engine_opens_no_second_connection_ahead() {
    // Each answer takes 300 ms, so the second call comes while the first is on its way.
    let server = serve_keep_alive(Vec::new(), Duration::from_millis(300));
    let engine = crate::AudioEngine::new_null(engine_tmp());
    engine.connect(&server.url);
    engine.connect(&server.url);
    server.wait_answered(1);
    // The connection waits in the pool now: the next call's request goes over it.
    engine.connect(&server.url);
    server.wait_answered(2);
    assert_eq!(
        server.answered.lock().unwrap().len(),
        2,
        "the second call sent nothing"
    );
    assert_eq!(server.connections.load(Ordering::SeqCst), 1);
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

/// A preload whose URL is refused says so, so the app can resolve it again before Play
/// instead of after it (2026-11-05: the restored track's URL, refused once Play was pressed).
#[test]
fn engine_reports_a_refused_preload() {
    let server = serve(fixture("tone-10s-frag.m4a"), 1 << 20, Duration::ZERO);
    server.log.refuse.store(u32::MAX, Ordering::SeqCst);
    let engine = crate::AudioEngine::new_null(engine_tmp());
    let events = engine.events();
    let url = format!("{}?c=WEB_EMBEDDED_PLAYER", server.url);
    engine.preload(src(&url));
    let event = events.recv_timeout(Duration::from_secs(3));
    assert_eq!(event, Ok(crate::AudioEvent::PreloadRefused(url)));
}

/// A track resolved ahead gets its head fetched: that fetch is the probe that finds a
/// refused URL before the track is played.
#[test]
fn engine_reports_a_refused_prefetch() {
    let server = serve(fixture("tone-10s-frag.m4a"), 1 << 20, Duration::ZERO);
    server.log.refuse.store(u32::MAX, Ordering::SeqCst);
    let engine = crate::AudioEngine::new_null(engine_tmp());
    let events = engine.events();
    let url = format!("{}?c=WEB_EMBEDDED_PLAYER", server.url);
    engine.prefetch(&src(&url));
    let event = events.recv_timeout(Duration::from_secs(3));
    assert_eq!(event, Ok(crate::AudioEvent::PreloadRefused(url)));
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

// With no output device (the speakers switched off): the engine starts anyway, and `play()`
// opens a device once there is one.

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

// Crossfade: two voices at once for the fade, on the null output.

/// A local fixture as a `file://` stream.
fn local(name: &str) -> StreamSource {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(name);
    let url = format!(
        "file:///{}",
        path.to_string_lossy()
            .replace('\\', "/")
            .trim_start_matches('/')
    );
    src(&url)
}

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

/// Within a 10 ms block, an AAC frame (23 ms) and the encoder's padding.
#[track_caller]
fn near(got: Option<Duration>, want: f64) {
    let got = got.expect("an end").as_secs_f64();
    assert!((got - want).abs() < 0.05, "{got} s, want {want} s");
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

/// This process's CPU time so far, from its cycle count: `GetProcessTimes` moves in 15.6 ms
/// steps, too coarse for a few ms per second. The cycles are converted at the rate a busy
/// thread counts them, measured once.
#[cfg(windows)]
fn process_cpu() -> Duration {
    use std::sync::OnceLock;

    use windows::Win32::System::Threading::{GetCurrentProcess, GetCurrentThread};
    use windows::Win32::System::WindowsProgramming::{QueryProcessCycleTime, QueryThreadCycleTime};

    let thread = || {
        let mut c = 0u64;
        // SAFETY: the pseudo-handle of the current thread is always valid, and the pointer
        // is to a live local.
        let _ = unsafe { QueryThreadCycleTime(GetCurrentThread(), &mut c) };
        c
    };
    static PER_SECOND: OnceLock<f64> = OnceLock::new();
    let per_second = *PER_SECOND.get_or_init(|| {
        let (c0, t) = (thread(), Instant::now());
        while t.elapsed() < Duration::from_millis(300) {
            std::hint::spin_loop();
        }
        (thread() - c0) as f64 / t.elapsed().as_secs_f64()
    });
    let mut cycles = 0u64;
    // SAFETY: the pseudo-handle of the current process is always valid, and the pointer is
    // to a live local.
    let _ = unsafe { QueryProcessCycleTime(GetCurrentProcess(), &mut cycles) };
    Duration::from_secs_f64(cycles as f64 / per_second)
}

#[cfg(not(windows))]
fn process_cpu() -> Duration {
    Duration::ZERO
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
#[cfg(windows)]
fn commit() -> (usize, usize) {
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::GetCurrentProcess;

    let mut c = PROCESS_MEMORY_COUNTERS::default();
    // SAFETY: the pseudo-handle of the current process is always valid, and `c` is a live
    // local of the size passed.
    let _ = unsafe {
        GetProcessMemoryInfo(
            GetCurrentProcess(),
            &mut c,
            std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
        )
    };
    (c.PagefileUsage, c.PeakPagefileUsage)
}

#[cfg(not(windows))]
fn commit() -> (usize, usize) {
    (0, 0)
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
        crate::priority::lower_scan();
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
