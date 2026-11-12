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
use crate::head::HeadCache;
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

mod crossfade;
mod devices;
mod engine;
mod media;
mod silence;
mod webm;

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

/// Within a 10 ms block, an AAC frame (23 ms) and the encoder's padding.
#[track_caller]
fn near(got: Option<Duration>, want: f64) {
    let got = got.expect("an end").as_secs_f64();
    assert!((got - want).abs() < 0.05, "{got} s, want {want} s");
}

/// This process's CPU time so far, from its cycle count: `GetProcessTimes` moves in 15.6 ms
/// steps, too coarse for a few ms per second. The cycles are converted at the rate a busy
/// thread counts them, measured once.
fn process_cpu() -> Duration {
    use std::sync::OnceLock;

    use featherplay_platform::thread::{cycles, process_cycles};

    // Linux counts no cycles: the cost reads as zero there.
    if process_cycles().is_none() {
        return Duration::ZERO;
    }
    let thread = || cycles().unwrap_or(0);
    static PER_SECOND: OnceLock<f64> = OnceLock::new();
    let per_second = *PER_SECOND.get_or_init(|| {
        let (c0, t) = (thread(), Instant::now());
        while t.elapsed() < Duration::from_millis(300) {
            std::hint::spin_loop();
        }
        (thread() - c0) as f64 / t.elapsed().as_secs_f64()
    });
    let process = process_cycles().unwrap_or(0);
    Duration::from_secs_f64(process as f64 / per_second)
}
