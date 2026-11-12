//! Headless load-time harness: splits a track's time to first sound into stages. It resolves
//! with the yt-dlp sidecar, then opens and decodes the stream into nothing (no output
//! device). Run it by hand; every track costs one live resolve:
//!
//! ```text
//! FEATHERPLAY_HOME=<your dev home> \
//! TIMING_IDS=FZ_vOw0Q1gY,K0HSD_i2DvA TIMING_OUT=<a scratch folder> \
//! cargo test --release -p featherplay-audio --lib timing -- --ignored --nocapture
//! ```
//!
//! - `TIMING_XARGS`: yt-dlp `--extractor-args` for the resolve (default: the app's own).
//! - `TIMING_LABEL`: a label for the CSV rows.
//! - `TIMING_REUSE`: a `streams.tsv` written by an earlier run; skips resolving and only
//!   measures the HTTP and decoder stages on those (still valid) URLs.
//!
//! Rows are appended to `{TIMING_OUT}/stages.csv`; resolved streams to `streams.tsv` and
//! yt-dlp's stderr to `ytdlp-{id}-{label}.log` there. Stream URLs stay in `TIMING_OUT`.

use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::StreamSource;
use crate::decoder::Decoder;
use crate::head::{self, HeadCache};
use crate::http::ErrorSlot;
use crate::media::{Media, Opened};

struct Resolved {
    id: String,
    url: String,
    mime: String,
    clen: Option<u64>,
    available_at: Option<u64>,
    /// Seconds from spawning yt-dlp to: its first stderr line, its first `[youtube]` line,
    /// its first player request, exit.
    startup: f64,
    extract_start: f64,
    player_request: f64,
    total: f64,
    ads: String,
    /// Whether the video title and a first tag came back (the scrobbler's alternates).
    meta: String,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.trim().is_empty())
}

fn unix_now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

fn resolve(home: &Path, out: &Path, id: &str, xargs: Option<&str>, label: &str) -> Resolved {
    let sidecars = env("FEATHERPLAY_SIDECARS_DIR")
        .map(Into::into)
        .unwrap_or_else(|| home.join("sidecars"));
    let deno = sidecars.join(format!("deno{}", std::env::consts::EXE_SUFFIX));
    let mut cmd = Command::new(sidecars.join(format!("yt-dlp{}", std::env::consts::EXE_SUFFIX)));
    cmd.env("DENO_DIR", home.join("deno-dir"))
        .args(["--ignore-config", "--no-playlist", "-v", "-J", "-f", "140"])
        .arg("--cache-dir")
        .arg(home.join("yt-dlp-cache"))
        .arg("--js-runtimes")
        .arg(format!("deno:{}", deno.display()));
    if let Some(x) = xargs {
        cmd.arg("--extractor-args").arg(x);
    }
    // The app's resolves go through the same proxy (`YtDlpResolver::command`).
    if let Some(p) = featherplay_proxy::for_url(featherplay_proxy::YOUTUBE)
        && let Some(arg) = p.argument()
    {
        cmd.arg("--proxy").arg(arg);
    }
    cmd.arg(format!("https://music.youtube.com/watch?v={id}"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let t0 = Instant::now();
    let mut child = cmd.spawn().expect("spawn yt-dlp");
    let stderr = child.stderr.take().unwrap();
    let lines = std::thread::spawn(move || {
        BufReader::new(stderr)
            .lines()
            .map_while(Result::ok)
            .map(|l| (t0.elapsed().as_secs_f64(), l))
            .collect::<Vec<_>>()
    });
    let mut stdout = String::new();
    std::io::Read::read_to_string(child.stdout.as_mut().unwrap(), &mut stdout).unwrap();
    let status = child.wait().unwrap();
    let total = t0.elapsed().as_secs_f64();
    let lines = lines.join().unwrap();
    let log: String = lines
        .iter()
        .map(|(t, l)| format!("[{t:7.3}] {l}\n"))
        .collect();
    std::fs::write(out.join(format!("ytdlp-{id}-{label}.log")), log).unwrap();
    assert!(status.success(), "{id}: yt-dlp failed, see the log");
    let at = |p: &dyn Fn(&str) -> bool| lines.iter().find(|(_, l)| p(l)).map_or(total, |l| l.0);
    let ads = lines
        .iter()
        .filter(|(_, l)| l.contains("Detected a"))
        .map(|(_, l)| l.rsplit(": ").next().unwrap_or_default().to_string())
        .collect::<Vec<_>>()
        .join("; ");
    let s = featherplay_streams::parse_ytdlp_json(&stdout, SystemTime::now()).expect("parse");
    Resolved {
        id: id.into(),
        url: s.url,
        mime: s.mime,
        clen: s.content_length,
        available_at: s
            .available_at
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_secs()),
        startup: lines.first().map_or(total, |l| l.0),
        extract_start: at(&|l| l.starts_with("[youtube]")),
        player_request: at(&|l| l.contains("player API JSON")),
        total,
        ads,
        meta: format!(
            "title={} tag={}",
            s.video_title.is_some(),
            s.first_tag.is_some()
        ),
    }
}

fn query(url: &str, key: &str) -> String {
    url.split_once('?')
        .map(|(_, q)| q)
        .unwrap_or_default()
        .split('&')
        .find_map(|p| p.strip_prefix(&format!("{key}=")))
        .unwrap_or_default()
        .to_string()
}

fn source(s: &Resolved) -> StreamSource {
    StreamSource {
        url: s.url.clone(),
        mime: s.mime.clone(),
        content_length: s.clen,
        duration: None,
        loudness_db: None,
    }
}

/// Seconds to: the first chunk's headers (open), the decoder, the first sample, 5 s of
/// audio decoded. With `heads` holding the URL, open needs no request.
fn decode(
    rt: &tokio::runtime::Runtime,
    tmp: &Path,
    s: &Resolved,
    heads: &HeadCache,
) -> (f64, f64, f64, f64) {
    let client = featherplay_proxy::client().build().unwrap();
    let t = Instant::now();
    let Opened {
        media, len, cancel, ..
    } = Media::open(
        &source(s),
        rt.handle(),
        &client,
        tmp,
        heads,
        ErrorSlot::default(),
    )
    .expect("open");
    let open = t.elapsed().as_secs_f64();
    let mut dec = Decoder::open(media, len, &s.mime).expect("decoder");
    let probe = t.elapsed().as_secs_f64();
    dec.next().expect("a first sample");
    let first = t.elapsed().as_secs_f64();
    let want = 5 * dec.sample_rate().get() as usize * dec.channels().get() as usize;
    for _ in 0..want {
        if dec.next().is_none() {
            break;
        }
    }
    let five = t.elapsed().as_secs_f64();
    drop(dec);
    if let Some(c) = cancel {
        c();
    }
    (open, probe, first, five)
}

/// Seconds for the whole file in sequential 1 MiB `Range` requests, as the engine fetches it.
async fn full_download(url: &str) -> (f64, u64) {
    let client = featherplay_proxy::client().build().unwrap();
    let t = Instant::now();
    let mut pos = 0u64;
    let mut total = None;
    loop {
        let r = client
            .get(url)
            .header(
                "Range",
                format!("bytes={pos}-{}", pos + crate::http::CHUNK - 1),
            )
            .send()
            .await
            .expect("range request");
        assert!(r.status().is_success(), "HTTP {}", r.status());
        if total.is_none() {
            total = r
                .headers()
                .get("content-range")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.rsplit('/').next()?.parse::<u64>().ok());
        }
        pos += r.bytes().await.expect("body").len() as u64;
        if total.is_none_or(|t| pos >= t) {
            break;
        }
    }
    (t.elapsed().as_secs_f64(), pos)
}

async fn status_now(url: &str) -> u16 {
    featherplay_proxy::client()
        .build()
        .unwrap()
        .get(url)
        .header("Range", "bytes=0-1023")
        .send()
        .await
        .map(|r| r.status().as_u16())
        .unwrap_or(0)
}

/// Waits until `available_at` + `margin` and returns the status then; on a 403, tries again
/// 0.5 s and 1 s later. Returns (seconds waited, margin that worked, statuses).
fn gate(rt: &tokio::runtime::Runtime, r: &Resolved, margin: f64) -> (f64, f64, String) {
    let t = Instant::now();
    let Some(at) = r.available_at else {
        return (0.0, 0.0, String::new());
    };
    let mut statuses = Vec::new();
    for m in [margin, margin + 0.5, margin + 1.0] {
        let left = at as f64 + m - unix_now();
        if left > 0.0 {
            std::thread::sleep(Duration::from_secs_f64(left));
        }
        let st = rt.block_on(status_now(&r.url));
        statuses.push(format!("{st}@{m:+.1}"));
        if st == 206 {
            return (t.elapsed().as_secs_f64(), m, statuses.join(" "));
        }
    }
    (t.elapsed().as_secs_f64(), f64::NAN, statuses.join(" "))
}

fn read_reuse(path: &Path) -> Vec<Resolved> {
    std::fs::read_to_string(path)
        .expect("TIMING_REUSE")
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (f.len() >= 5).then(|| Resolved {
                id: f[0].into(),
                url: f[1].into(),
                mime: f[2].into(),
                clen: f[3].parse().ok(),
                available_at: f[4].parse().ok(),
                startup: 0.0,
                extract_start: 0.0,
                player_request: 0.0,
                total: 0.0,
                ads: String::new(),
                meta: String::new(),
            })
        })
        .collect()
}

#[test]
#[ignore = "live: one yt-dlp resolve per TIMING_IDS entry (none with TIMING_REUSE)"]
fn timing() {
    let home = std::path::PathBuf::from(env("FEATHERPLAY_HOME").expect("set FEATHERPLAY_HOME"));
    let out = std::path::PathBuf::from(env("TIMING_OUT").expect("set TIMING_OUT"));
    std::fs::create_dir_all(&out).unwrap();
    let tmp = home.join("tmp");
    let label = env("TIMING_LABEL").unwrap_or_else(|| "run".into());
    let xargs = env("TIMING_XARGS");
    // How long past `available_at` the first try goes (the app's margin).
    let margin: f64 = env("TIMING_MARGIN").map_or(0.2, |m| m.parse().unwrap());
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let ids: Vec<String> = env("TIMING_IDS")
        .map(|v| v.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();
    let mut csv = OpenOptions::new()
        .create(true)
        .append(true)
        .open(out.join("stages2.csv"))
        .unwrap();
    if csv.metadata().unwrap().len() == 0 {
        writeln!(
            csv,
            "label,id,xargs,c,startup_s,extract_start_s,player_request_s,ytdlp_total_s,ads,\
             available_in_s,status_at_resolve,gate_wait_s,gate_margin_s,gate_statuses,\
             open_s,probe_s,first_sample_s,five_s_decoded_s,head_fetch_s,head_first_sample_s,\
             full_download_s,bytes,meta"
        )
        .unwrap();
    }
    let reused: Option<Vec<Resolved>> = env("TIMING_REUSE").map(|p| read_reuse(Path::new(&p)));
    let fresh = reused.is_none();
    let tracks: Box<dyn Iterator<Item = Resolved>> = match reused {
        Some(v) => Box::new(v.into_iter()),
        None => Box::new(ids.iter().enumerate().map(|(i, id)| {
            if i > 0 {
                // Spread the live resolves out a little.
                std::thread::sleep(Duration::from_secs(5));
            }
            let r = resolve(&home, &out, id, xargs.as_deref(), &label);
            let mut tsv = OpenOptions::new()
                .create(true)
                .append(true)
                .open(out.join("streams.tsv"))
                .unwrap();
            writeln!(
                tsv,
                "{}\t{}\t{}\t{}\t{}\t{label}",
                r.id,
                r.url,
                r.mime,
                r.clen.map(|c| c.to_string()).unwrap_or_default(),
                r.available_at.map(|c| c.to_string()).unwrap_or_default()
            )
            .unwrap();
            r
        })),
    };
    for r in tracks {
        let available_in = r.available_at.map_or(0.0, |a| a as f64 - unix_now());
        let early = if fresh {
            rt.block_on(status_now(&r.url))
        } else {
            0
        };
        let (gate_wait, gate_margin, statuses) = if fresh {
            gate(&rt, &r, margin)
        } else {
            (0.0, 0.0, String::new())
        };
        let cold = HeadCache::default();
        let (open, probe, first, five) = decode(&rt, &tmp, &r, &cold);
        // The same stream with its head fetched ahead, as after a pre-resolve.
        let heads = HeadCache::default();
        let t = Instant::now();
        head::prefetch(
            &heads,
            rt.handle(),
            &featherplay_proxy::client().build().unwrap(),
            r.url.clone(),
            |_| {},
        );
        while heads.get(&r.url).is_none() {
            assert!(t.elapsed() < Duration::from_secs(10), "no head");
            std::thread::sleep(Duration::from_millis(5));
        }
        let head_fetch = t.elapsed().as_secs_f64();
        let (_, _, head_first, _) = decode(&rt, &tmp, &r, &heads);
        let (full, bytes) = rt.block_on(full_download(&r.url));
        let row = format!(
            "{label},{},{},{},{:.3},{:.3},{:.3},{:.3},{},{available_in:.1},{early},\
             {gate_wait:.3},{gate_margin:.1},{statuses},{open:.3},{probe:.3},{first:.3},\
             {five:.3},{head_fetch:.3},{head_first:.3},{full:.3},{bytes},{}",
            r.id,
            xargs.as_deref().unwrap_or("default").replace(',', "+"),
            query(&r.url, "c"),
            r.startup,
            r.extract_start,
            r.player_request,
            r.total,
            r.ads.replace(',', " "),
            r.meta,
        );
        println!("{row}");
        writeln!(csv, "{row}").unwrap();
    }
}

/// The app's path end to end, on a null output: `YtDlpResolver::resolve` (the native
/// request, then yt-dlp's embedded client, then the TV fallback and its gate wait), then
/// `AudioEngine` load + play until `Started`.
/// Per track: a cold click; a click after a resolve ahead (`prefetch`); Next (`preload`).
/// `TIMING_COLD_ONLY=1` does the cold click alone (fewer requests; the others show 0).
/// One live resolve per `TIMING_IDS` entry; rows go to `{TIMING_OUT}/engine.csv`.
#[test]
#[ignore = "live: one yt-dlp resolve per TIMING_IDS entry"]
fn timing_engine() {
    use featherplay_core::model::TrackId;
    use featherplay_streams::{Sidecars, StreamResolver, YtDlpResolver};

    // The resolver's and the engine's log lines (ads before a song, the first sound).
    let _ = log::set_logger(&NEW_CONNECTIONS);
    log::set_max_level(log::LevelFilter::Debug);
    let home = std::path::PathBuf::from(env("FEATHERPLAY_HOME").expect("set FEATHERPLAY_HOME"));
    let out = std::path::PathBuf::from(env("TIMING_OUT").expect("set TIMING_OUT"));
    let label = env("TIMING_LABEL").unwrap_or_else(|| "engine".into());
    let sidecars = env("FEATHERPLAY_SIDECARS_DIR")
        .map(Into::into)
        .unwrap_or_else(|| home.join("sidecars"));
    let resolver = YtDlpResolver::new(Sidecars { dir: sidecars }, None);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let tmp = home.join("tmp");
    // Load and play `src` on a fresh engine after `prep` ran on it; seconds to `Started`.
    let play = |src: &StreamSource, prep: &dyn Fn(&crate::AudioEngine)| -> f64 {
        let engine = crate::AudioEngine::new_null(tmp.clone());
        let events = engine.events();
        prep(&engine);
        let t = Instant::now();
        engine.load(src.clone(), Duration::ZERO);
        engine.play();
        loop {
            match events.recv_timeout(Duration::from_secs(20)) {
                Ok(crate::AudioEvent::Started) => return t.elapsed().as_secs_f64(),
                Ok(crate::AudioEvent::Error(e)) => panic!("{e}"),
                Ok(_) => {}
                Err(_) => panic!("no Started"),
            }
        }
    };
    let mut csv = OpenOptions::new()
        .create(true)
        .append(true)
        .open(out.join("engine.csv"))
        .unwrap();
    if csv.metadata().unwrap().len() == 0 {
        writeln!(
            csv,
            "label,id,client,resolve_s,cold_started_s,cold_total_s,ahead_started_s,next_started_s"
        )
        .unwrap();
    }
    let ids: Vec<String> = env("TIMING_IDS")
        .map(|v| v.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();
    for (i, id) in ids.iter().enumerate() {
        if i > 0 {
            std::thread::sleep(Duration::from_secs(5));
        }
        // A cold click: resolve, load, play.
        let t = Instant::now();
        let s = rt
            .block_on(resolver.resolve(&TrackId(id.clone())))
            .expect("resolve");
        let resolve_s = t.elapsed().as_secs_f64();
        let via = resolver
            .take_fallback_reason(&TrackId(id.clone()))
            .map_or("native".into(), |e| format!("yt-dlp after: {e}"));
        let src = StreamSource {
            url: s.url.clone(),
            mime: s.mime.clone(),
            content_length: s.content_length,
            duration: None,
            loudness_db: None,
        };
        let cold = play(&src, &|_| {});
        let cold_total = t.elapsed().as_secs_f64();
        if env("TIMING_COLD_ONLY").is_some() {
            let row = format!(
                "{label},{id},{},{resolve_s:.3},{cold:.3},{cold_total:.3},0,0",
                query(&s.url, "c")
            );
            println!("{row} ({via})");
            writeln!(csv, "{row}").unwrap();
            continue;
        }
        // Clicked after a resolve ahead: the head is in memory.
        let ahead = play(&src, &|e| {
            e.prefetch(&src);
            std::thread::sleep(Duration::from_secs(2));
        });
        // Next: the player preloaded it while the one before played.
        let next = play(&src, &|e| {
            e.preload(src.clone());
            std::thread::sleep(Duration::from_secs(2));
        });
        let row = format!(
            "{label},{id},{},{resolve_s:.3},{cold:.3},{cold_total:.3},{ahead:.3},{next:.3}",
            query(&s.url, "c")
        );
        println!("{row} ({via})");
        writeln!(csv, "{row}").unwrap();
    }
}

/// Prints each new connection reqwest opens (without its host) and the engine's own log
/// lines, so a request on a kept connection shows as one without a line before it.
struct NewConnections;

static NEW_CONNECTIONS: NewConnections = NewConnections;

/// The connections opened to stream servers so far.
static STREAM_CONNECTIONS: AtomicU32 = AtomicU32::new(0);

impl log::Log for NewConnections {
    fn enabled(&self, m: &log::Metadata) -> bool {
        m.target().starts_with("reqwest") || m.target().starts_with("featherplay")
    }
    fn log(&self, r: &log::Record) {
        let msg = r.args().to_string();
        if msg.starts_with("starting new connection") {
            if msg.contains(".googlevideo.com") {
                STREAM_CONNECTIONS.fetch_add(1, Ordering::Relaxed);
            }
            println!("  new connection");
        } else if r.target().starts_with("featherplay") {
            println!("  {msg}");
        }
    }
    fn flush(&self) {}
}

/// The rows of a `TIMING_STREAMS` file: `url, mime, length` and, for `timing_click`, the
/// track's id.
fn read_streams(path: &str) -> Vec<(StreamSource, String)> {
    std::fs::read_to_string(path)
        .expect("TIMING_STREAMS")
        .lines()
        .filter_map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            (f.len() >= 3).then(|| {
                let src = StreamSource {
                    url: f[0].into(),
                    mime: f[1].into(),
                    content_length: f[2].parse().ok(),
                    duration: None,
                    loudness_db: None,
                };
                (src, f.get(3).copied().unwrap_or_default().to_string())
            })
        })
        .collect()
}

/// `scheme://host` of a URL.
fn server(url: &str) -> &str {
    let host = url.find("://").map_or(0, |i| i + 3);
    let end = url[host..].find(['/', '?']).map_or(url.len(), |i| host + i);
    &url[..end]
}

/// Cold clicks as the app makes them, on a null output: an engine that's there already,
/// the stream's resolve, then load + play until `Started`, with the engine's own steps
/// (`FirstSound`) and the connections opened to stream servers on the way. The engine goes
/// at `Started`, which stops the download: one `Range` request per click.
///
/// - `TIMING_STREAMS` (as for `timing_start`, with the track's id as a fourth column):
///   streams resolved earlier. The first row is loaded once before the clicks and not
///   counted, as a process's first request also pays for DNS and the TLS library's start.
///   Each click says whether its stream is on the server most of these rows are on.
/// - `TIMING_PREPARED=n`: the next `n` rows are clicked `TIMING_GAP` ms (default 100) and
///   2 s after a prefetch, a hovered and a prepared start (two requests each).
/// - `TIMING_SAVED=n`: the next `n` rows are clicked cold without a resolve: a wait of
///   `TIMING_RESOLVE_MS` (default 400) stands in for it.
/// - `TIMING_IDS`: cold clicks with the player's resolve (`YtDlpResolver::resolve`, signed
///   out), one live resolve each.
/// - `TIMING_CONNECT=1`: at each cold click the engine opens a connection to that server
///   while the resolve runs (`AudioEngine::connect`), as the app does.
///
/// Rows go to `{TIMING_OUT}/click.csv`; a click that fails says why in its last column.
#[test]
#[ignore = "live: one Range request per click, and one resolve per TIMING_IDS entry"]
fn timing_click() {
    use featherplay_core::model::TrackId;
    use featherplay_streams::{Sidecars, StreamResolver, YtDlpResolver};

    let _ = log::set_logger(&NEW_CONNECTIONS);
    log::set_max_level(log::LevelFilter::Debug);
    let home = std::path::PathBuf::from(env("FEATHERPLAY_HOME").expect("set FEATHERPLAY_HOME"));
    let out = std::path::PathBuf::from(env("TIMING_OUT").expect("set TIMING_OUT"));
    std::fs::create_dir_all(&out).unwrap();
    let label = env("TIMING_LABEL").unwrap_or_else(|| "click".into());
    let count = |name: &str| env(name).map_or(0, |n| n.parse::<usize>().unwrap());
    let millis =
        |name: &str, or: u64| Duration::from_millis(env(name).map_or(or, |ms| ms.parse().unwrap()));
    let gap = millis("TIMING_GAP", 100);
    let stand_in = millis("TIMING_RESOLVE_MS", 400);
    let connect = env("TIMING_CONNECT").is_some();
    let streams = read_streams(&env("TIMING_STREAMS").expect("set TIMING_STREAMS"));
    let servers: Vec<&str> = streams.iter().map(|(s, _)| server(&s.url)).collect();
    let likely = servers
        .iter()
        .copied()
        .max_by_key(|s| servers.iter().filter(|o| *o == s).count())
        .expect("a row in TIMING_STREAMS");
    let sidecars = Sidecars {
        dir: home.join("sidecars"),
    };
    let resolver = YtDlpResolver::new(sidecars, None);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let tmp = home.join("tmp");
    let mut csv = OpenOptions::new()
        .create(true)
        .append(true)
        .open(out.join("click.csv"))
        .unwrap();
    if csv.metadata().unwrap().len() == 0 {
        writeln!(
            csv,
            "label,kind,id,likely_server,resolve_ms,click_to_sound_ms,\
             total,open,at_hand,decoder,fill,output,stream_connections,failed"
        )
        .unwrap();
    }
    // One click on a new engine, after `prep` ran on it: `stream` gives the stream (the
    // resolve, or one at hand), then load and play until `Started`.
    let mut click = |kind: &str,
                     id: &str,
                     prep: &dyn Fn(&crate::AudioEngine),
                     stream: &dyn Fn() -> Result<StreamSource, String>| {
        let engine = crate::AudioEngine::new_null(tmp.clone());
        let events = engine.events();
        prep(&engine);
        let before = STREAM_CONNECTIONS.load(Ordering::Relaxed);
        let t = Instant::now();
        if connect && matches!(kind, "saved" | "cold") {
            engine.connect(likely);
        }
        let started = stream().and_then(|src| {
            let resolved = t.elapsed();
            engine.load(src.clone(), Duration::ZERO);
            engine.play();
            loop {
                match events.recv_timeout(Duration::from_secs(20)) {
                    Ok(crate::AudioEvent::Started) => break,
                    Ok(crate::AudioEvent::Error(e)) => return Err(e),
                    Ok(_) => {}
                    Err(_) => return Err("no Started".into()),
                }
            }
            Ok((src, resolved, t.elapsed()))
        });
        let connections = STREAM_CONNECTIONS.load(Ordering::Relaxed) - before;
        let row = match started {
            Ok((src, resolved, sound)) => {
                let f = engine.first_sound().expect("first sound");
                format!(
                    "{label},{kind},{id},{},{},{},{},{},{},{},{},{},{connections},",
                    server(&src.url) == likely,
                    resolved.as_millis(),
                    sound.as_millis(),
                    f.total.as_millis(),
                    f.open.as_millis(),
                    f.at_hand,
                    f.decoder.as_millis(),
                    f.fill.as_millis(),
                    f.output.as_millis(),
                )
            }
            Err(e) => {
                // Without the URL an error may name.
                let why = e.split("http").next().unwrap_or_default().replace(',', ";");
                format!("{label},{kind},{id},,,,,,,,,,{connections},{}", why.trim())
            }
        };
        drop(engine);
        println!("{row}");
        writeln!(csv, "{row}").unwrap();
    };
    let mut rows = streams.iter();
    if let Some((src, id)) = rows.next() {
        click("first", id, &|_| {}, &|| Ok(src.clone()));
    }
    for (src, id) in rows.by_ref().take(count("TIMING_PREPARED")) {
        let ahead = |wait: Duration| {
            move |e: &crate::AudioEngine| {
                e.prefetch(src);
                std::thread::sleep(wait);
            }
        };
        click("hovered", id, &ahead(gap), &|| Ok(src.clone()));
        click("prepared", id, &ahead(Duration::from_secs(2)), &|| {
            Ok(src.clone())
        });
    }
    for (src, id) in rows.by_ref().take(count("TIMING_SAVED")) {
        click("saved", id, &|_| {}, &|| {
            std::thread::sleep(stand_in);
            Ok(src.clone())
        });
    }
    let ids: Vec<String> = env("TIMING_IDS")
        .map(|v| v.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();
    for (i, id) in ids.iter().enumerate() {
        if i > 0 {
            // Spread the live resolves out a little.
            std::thread::sleep(Duration::from_secs(5));
        }
        click("cold", id, &|_| {}, &|| {
            let s = rt
                .block_on(resolver.resolve(&TrackId(id.clone())))
                .map_err(|e| format!("resolve: {e:?}"))?;
            Ok(StreamSource {
                url: s.url,
                mime: s.mime,
                content_length: s.content_length,
                duration: None,
                loudness_db: None,
            })
        });
    }
}

/// The engine's own time from `load` to `Started`, step by step (`FirstSound`), on streams
/// already resolved: no resolve. `TIMING_STREAMS` is a TSV of `url, mime, length` rows
/// (e.g. from the app's `streams.json`). Per stream, a few `Range` requests: the first
/// response's headers on a new connection and again on the same one, then the engine's
/// cold load, the same stream again on that engine, a load after a prefetch, and one
/// `TIMING_GAP` ms (default 100) after a prefetch, while its head is on its way.
/// `TIMING_DEVICE=1` plays on the default device at volume 0 instead of the null output.
/// `TIMING_IDLE=30,90` also times a request on a connection left idle that many seconds
/// (first stream only). Rows go to `{TIMING_OUT}/start.csv`.
#[test]
#[ignore = "live: a few Range requests per TIMING_STREAMS row"]
fn timing_start() {
    use crate::engine::FirstSound;

    let _ = log::set_logger(&NEW_CONNECTIONS);
    log::set_max_level(log::LevelFilter::Debug);
    let out = std::path::PathBuf::from(env("TIMING_OUT").expect("set TIMING_OUT"));
    let tmp = out.join("tmp");
    let label = env("TIMING_LABEL").unwrap_or_else(|| "start".into());
    let device = env("TIMING_DEVICE").is_some();
    let streams: Vec<StreamSource> =
        read_streams(&env("TIMING_STREAMS").expect("set TIMING_STREAMS"))
            .into_iter()
            .map(|(src, _)| src)
            .collect();
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    // Seconds to the headers of a first-chunk request on `client`. The body is read to its
    // end, so the connection goes back to the pool (a response dropped early closes it).
    let headers = |client: &reqwest::Client, url: &str| -> f64 {
        let t = Instant::now();
        let r = rt
            .block_on(crate::http::get_range(client, url, 0, crate::http::CHUNK))
            .expect("range request");
        let s = t.elapsed().as_secs_f64();
        rt.block_on(r.bytes()).expect("body");
        s
    };
    let engine = || {
        if device {
            let e = crate::AudioEngine::new(None, tmp.clone()).expect("engine");
            e.set_volume(0.0);
            e
        } else {
            crate::AudioEngine::new_null(tmp.clone())
        }
    };
    let play = |e: &crate::AudioEngine, src: &StreamSource| -> FirstSound {
        let events = e.events();
        e.load(src.clone(), Duration::ZERO);
        e.play();
        loop {
            match events.recv_timeout(Duration::from_secs(20)) {
                Ok(crate::AudioEvent::Started) => break,
                Ok(crate::AudioEvent::Error(err)) => panic!("{err}"),
                Ok(other) => println!("  event {other:?}"),
                Err(_) => panic!("no Started"),
            }
        }
        e.first_sound().expect("first sound")
    };
    let ms = |s: f64| (s * 1000.0).round();
    let fs = |f: &FirstSound| {
        format!(
            "{},{},{},{},{},{}",
            f.total.as_millis(),
            f.open.as_millis(),
            f.at_hand,
            f.decoder.as_millis(),
            f.fill.as_millis(),
            f.output.as_millis()
        )
    };
    let mut csv = OpenOptions::new()
        .create(true)
        .append(true)
        .open(out.join("start.csv"))
        .unwrap();
    if csv.metadata().unwrap().len() == 0 {
        writeln!(
            csv,
            "label,stream,mime,device,cold_headers_ms,warm_headers_ms,\
             cold_total,cold_open,cold_at_hand,cold_decoder,cold_fill,cold_output,\
             again_total,again_open,again_at_hand,again_decoder,again_fill,again_output,\
             ahead_total,ahead_open,ahead_at_hand,ahead_decoder,ahead_fill,ahead_output,\
             way_total,way_open,way_at_hand,way_decoder,way_fill,way_output"
        )
        .unwrap();
    }
    let gap = Duration::from_millis(env("TIMING_GAP").map_or(100, |g| g.parse().unwrap()));
    for (i, src) in streams.iter().enumerate() {
        let client = crate::http::stream_client(crate::http::READ_TIMEOUT).unwrap();
        println!(" stream {i}: a request on a new client");
        let cold = headers(&client, &src.url);
        println!(" the same again");
        let warm = headers(&client, &src.url);
        // A cold click: a new engine, so a new connection.
        println!(" a new engine's load");
        let e = engine();
        let c = play(&e, src);
        // Time for the whole download, whose last connection then waits in the pool.
        std::thread::sleep(Duration::from_secs(3));
        // The same stream again on that engine, its connection warm (a jump back). The
        // stopped voice's head isn't kept, so this opens with a request.
        println!(" the same load again");
        let again = play(&e, src);
        drop(e);
        // Resolved ahead: its head was fetched before the click.
        println!(" a new engine's prefetch, then its load");
        let e = engine();
        e.prefetch(src);
        std::thread::sleep(Duration::from_secs(2));
        let a = play(&e, src);
        drop(e);
        // Clicked just after its resolve ahead: its head is on its way.
        println!(" a new engine's prefetch, then its load {gap:?} later");
        let e = engine();
        e.prefetch(src);
        std::thread::sleep(gap);
        let on_its_way = play(&e, src);
        drop(e);
        let row = format!(
            "{label},{i},{},{device},{},{},{},{},{},{}",
            src.mime.split(';').next().unwrap_or_default(),
            ms(cold),
            ms(warm),
            fs(&c),
            fs(&again),
            fs(&a),
            fs(&on_its_way),
        );
        println!("{row}");
        writeln!(csv, "{row}").unwrap();
    }
    if let (Some(idle), Some(src)) = (env("TIMING_IDLE"), streams.first()) {
        // A pool that never drops idle connections itself: what the server keeps.
        let client = featherplay_proxy::client()
            .pool_idle_timeout(None)
            .build()
            .unwrap();
        for secs in idle.split(',').filter_map(|s| s.trim().parse::<u64>().ok()) {
            println!("  request, then idle {secs} s");
            let first = headers(&client, &src.url);
            std::thread::sleep(Duration::from_secs(secs));
            println!("  request after idle {secs} s");
            let after = headers(&client, &src.url);
            let row = format!(
                "idle {secs} s: first {} ms, after {} ms",
                ms(first),
                ms(after)
            );
            println!("{row}");
            writeln!(csv, "# {label} {row}").unwrap();
        }
    }
}
