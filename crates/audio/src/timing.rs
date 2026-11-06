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
    let deno = sidecars.join("deno.exe");
    let mut cmd = Command::new(sidecars.join("yt-dlp.exe"));
    cmd.env("DENO_DIR", home.join("deno-dir"))
        .args(["--ignore-config", "--no-playlist", "-v", "-J", "-f", "140"])
        .arg("--cache-dir")
        .arg(home.join("yt-dlp-cache"))
        .arg("--js-runtimes")
        .arg(format!("deno:{}", deno.display()));
    if let Some(x) = xargs {
        cmd.arg("--extractor-args").arg(x);
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
    let client = reqwest::Client::new();
    let t = Instant::now();
    let Opened { media, len, cancel } = Media::open(
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
    let client = reqwest::Client::new();
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
    reqwest::Client::new()
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
            &reqwest::Client::new(),
            r.url.clone(),
            |_| {},
        );
        while heads.lock().unwrap().get(&r.url).is_none() {
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
