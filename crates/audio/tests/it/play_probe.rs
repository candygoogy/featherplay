//! The play probe (`.github/workflows/play-probe.yml`, manual only): which resolver step
//! gets a playable stream from this machine, without sign-in. For each song it tries every
//! step on its own (`YtDlpResolver::probe`: the native request, yt-dlp as the app runs it,
//! yt-dlp's defaults, and single yt-dlp clients), then fetches the first 1 MiB of the URL.
//! A measurement, so it never fails on a step that's blocked or broken. Run with:
//! `FEATHERPLAY_HOME=<dir> cargo test -p featherplay-audio --test it -- play_probe:: --ignored --nocapture`
//! One line per (song, step) on stdout; the markdown table is appended to
//! `$GITHUB_STEP_SUMMARY` and to `$PLAY_PROBE_OUT`, when they're set.

use std::fmt::Write as _;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use featherplay_core::model::TrackId;
use featherplay_streams::{Probe, ResolvedStream, Sidecars, YtDlpResolver};

use super::daily_play::is_bot_check;

/// Public Creative Commons songs, all Kevin MacLeod uploads on his own channel
/// ("Kevin MacLeod [Official] - … - incompetech.com"). He offers all his music under CC BY
/// 4.0 (incompetech.com's FAQ), and the channel says its uploads are CC BY 4.0.
/// `Yy-NyPTRYU8`'s YouTube licence field read "Creative Commons Attribution license (reuse
/// allowed)" on 2026-10-25.
const SONGS: &[(&str, &str)] = &[
    ("Yy-NyPTRYU8", "Exit the Premises"),
    ("9WQBzXr4gFc", "Fresh Air"),
    ("XgBPZ68L6TE", "Crypto"),
];

/// yt-dlp 2026.10.26's clients worth a try without sign-in (`INNERTUBE_CLIENTS` in its
/// `extractor/youtube/_base.py`). Its defaults are `tv_samsung,visionos,web`. `tv`,
/// `tv_samsung` and `visionos` need no PO token for format 140; `web_safari`, `mweb`, `ios`
/// and `android_vr` do, so they can't give it ("Requested format is not available") but
/// still show whether their player request gets past the bot check.
const CLIENTS: &[&str] = &[
    "tv",
    "tv_samsung",
    "visionos",
    "web_safari",
    "mweb",
    "ios",
    "android_vr",
];

const FETCH: u64 = 1 << 20;
const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
/// Between two steps, so the sweep isn't a burst.
const PAUSE: Duration = Duration::from_secs(2);
/// The TV clients' URLs answer 403 until `available_at` (a pre-roll ad, ~6 s).
const MAX_GATE_WAIT: Duration = Duration::from_secs(20);
const DETAIL_CHARS: usize = 160;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Outcome {
    /// A stream URL, and its first 1 MiB fetched.
    Works,
    /// YouTube refused this machine: the bot check, a sign-in to confirm, HTTP 429.
    Blocked,
    /// Anything else.
    Broken,
}

impl Outcome {
    fn as_str(self) -> &'static str {
        match self {
            Outcome::Works => "works",
            Outcome::Blocked => "blocked",
            Outcome::Broken => "broken",
        }
    }
}

struct Row {
    song: &'static str,
    step: String,
    outcome: Outcome,
    resolve_ms: u128,
    total_ms: u128,
    detail: String,
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "live: one sweep of resolves and 1 MiB fetches, per song and step"]
async fn play_probe_sweeps_every_step() {
    let home = PathBuf::from(std::env::var("FEATHERPLAY_HOME").expect("set FEATHERPLAY_HOME"));
    let sidecars = || Sidecars {
        dir: home.join("sidecars"),
    };
    sidecars()
        .ensure(|_, _, _| {})
        .await
        .expect("download the sidecars");
    let (ytdlp, deno) = sidecars().versions().await;
    println!(
        "yt-dlp {}, deno {}",
        ytdlp.as_deref().unwrap_or("?"),
        deno.as_deref().unwrap_or("?")
    );

    let client_args: Vec<(String, String)> = CLIENTS
        .iter()
        .map(|c| (format!("yt-dlp {c}"), format!("youtube:player_client={c}")))
        .collect();
    let mut steps: Vec<(String, Probe<'_>)> = vec![
        ("native".into(), Probe::Native),
        ("yt-dlp embedded (app)".into(), Probe::Embedded),
        ("yt-dlp default".into(), Probe::Default),
    ];
    steps.extend(
        client_args
            .iter()
            .map(|(name, args)| (name.clone(), Probe::Args(args))),
    );

    let http = reqwest::Client::new();
    let mut rows = Vec::new();
    for &(song, _) in SONGS {
        for (name, step) in &steps {
            if !rows.is_empty() {
                tokio::time::sleep(PAUSE).await;
            }
            // A fresh resolver each time: no step inherits another's health or state.
            let resolver = YtDlpResolver::new(sidecars(), None);
            let row = probe_one(&resolver, &http, song, name, *step).await;
            println!(
                "PROBE {} | {} | {} | resolve {} ms | total {} ms | {}",
                row.song,
                row.step,
                row.outcome.as_str(),
                row.resolve_ms,
                row.total_ms,
                row.detail
            );
            rows.push(row);
        }
    }

    let table = markdown(&rows, ytdlp.as_deref(), deno.as_deref());
    println!("\n{table}");
    for var in ["GITHUB_STEP_SUMMARY", "PLAY_PROBE_OUT"] {
        if let Ok(path) = std::env::var(var) {
            append(&path, &table).unwrap_or_else(|e| panic!("write {var}: {e}"));
        }
    }
}

fn append(path: &str, text: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(text.as_bytes())
}

async fn probe_one(
    resolver: &YtDlpResolver,
    http: &reqwest::Client,
    song: &'static str,
    step_name: &str,
    step: Probe<'_>,
) -> Row {
    let t = Instant::now();
    let resolved = resolver.probe(&TrackId(song.into()), step).await;
    let resolve_ms = t.elapsed().as_millis();
    let (outcome, detail) = match resolved {
        Err(e) => (classify(&e), one_line(&e)),
        Ok(s) => match fetch_head(http, &s).await {
            Ok(bytes) => (
                Outcome::Works,
                format!(
                    "itag {}, c={}, {bytes} bytes",
                    s.itag,
                    url_param(&s.url, "c").unwrap_or("?")
                ),
            ),
            Err(e) => (classify(&e), one_line(&e)),
        },
    };
    Row {
        song,
        step: step_name.to_string(),
        outcome,
        resolve_ms,
        total_ms: t.elapsed().as_millis(),
        detail,
    }
}

/// The first 1 MiB (or the whole stream, if shorter), after the URL's ad gate if it has one.
async fn fetch_head(http: &reqwest::Client, s: &ResolvedStream) -> Result<usize, String> {
    if let Some(at) = s.available_at
        && let Ok(left) = at.duration_since(SystemTime::now())
    {
        tokio::time::sleep((left + Duration::from_millis(200)).min(MAX_GATE_WAIT)).await;
    }
    let want = s.content_length.map_or(FETCH, |len| len.min(FETCH));
    if want == 0 {
        return Err("the stream is empty".into());
    }
    let resp = http
        .get(&s.url)
        .header("Range", format!("bytes=0-{}", want - 1))
        .timeout(FETCH_TIMEOUT)
        .send()
        .await
        .map_err(|e| format!("fetch: {e}"))?;
    let status = resp.status();
    if status.as_u16() != 206 {
        return Err(format!("fetch answered HTTP {status}"));
    }
    let body = resp.bytes().await.map_err(|e| format!("fetch body: {e}"))?;
    if body.len() as u64 != want {
        return Err(format!("fetch gave {} of {want} bytes", body.len()));
    }
    Ok(body.len())
}

/// `blocked` when YouTube refused this machine, as yt-dlp words it ("Sign in to confirm
/// you're not a bot"), as the native request gets it (a `LOGIN_REQUIRED` answer with the
/// same reason, or an HTTP 429 to its GETs), or as Google's "unusual traffic" page says it.
fn classify(error: &str) -> Outcome {
    let low = error.to_ascii_lowercase();
    let blocked = is_bot_check(error)
        || low.contains("not a robot")
        || low.contains("too many requests")
        || low.contains("http error 429")
        || low.contains("answered 429")
        || low.contains("http 429")
        || low.contains("unusual traffic");
    if blocked {
        Outcome::Blocked
    } else {
        Outcome::Broken
    }
}

/// The line that says why: yt-dlp's last `ERROR:` line, else the last line, cut to
/// `DETAIL_CHARS` and safe in a table cell.
fn one_line(error: &str) -> String {
    let lines = || error.lines().map(str::trim).filter(|l| !l.is_empty());
    let line = lines()
        .rfind(|l| l.contains("ERROR:"))
        .or_else(|| lines().next_back())
        .unwrap_or("");
    let line = line.split_once("ERROR: ").map_or(line, |(_, rest)| rest);
    let mut out: String = line.chars().take(DETAIL_CHARS).collect();
    if line.chars().count() > DETAIL_CHARS {
        out.push('…');
    }
    out.replace('|', "/")
}

fn url_param<'a>(url: &'a str, key: &str) -> Option<&'a str> {
    let query = url.split_once('?')?.1;
    query
        .split('&')
        .find_map(|kv| kv.strip_prefix(key)?.strip_prefix('='))
}

fn markdown(rows: &[Row], ytdlp: Option<&str>, deno: Option<&str>) -> String {
    let mut t = format!(
        "### Play probe\n\nyt-dlp {}, deno {}. `works`: a stream URL and its first 1 MiB; \
         `blocked`: bot check, sign-in to confirm or HTTP 429; `broken`: anything else.\n\n\
         | song | step | outcome | resolve ms | total ms | detail |\n|---|---|---|---:|---:|---|\n",
        ytdlp.unwrap_or("?"),
        deno.unwrap_or("?")
    );
    for r in rows {
        let _ = writeln!(
            t,
            "| {} | {} | {} | {} | {} | {} |",
            r.song,
            r.step,
            r.outcome.as_str(),
            r.resolve_ms,
            r.total_ms,
            r.detail
        );
    }
    t.push('\n');
    t
}

#[test]
fn refusals_of_this_machine_are_told_apart_from_breakages() {
    let cases = [
        (
            "yt-dlp exited with code 1: WARNING: [youtube] x\nERROR: [youtube] Yy-NyPTRYU8: Sign in to confirm you\u{2019}re not a bot. Use --cookies-from-browser or --cookies for the authentication.",
            Outcome::Blocked,
        ),
        (
            "not with the embedded client: LOGIN_REQUIRED: Sign in to confirm you're not a bot",
            Outcome::Blocked,
        ),
        (
            "native resolve failed: GET answered 429 Too Many Requests",
            Outcome::Blocked,
        ),
        (
            "yt-dlp exited with code 1: ERROR: [youtube] x: Unable to download API page: HTTP Error 429: Too Many Requests",
            Outcome::Blocked,
        ),
        (
            "yt-dlp exited with code 1: ERROR: [youtube] Yy-NyPTRYU8: Requested format is not available",
            Outcome::Broken,
        ),
        ("fetch answered HTTP 403 Forbidden", Outcome::Broken),
    ];
    for (error, want) in cases {
        assert_eq!(classify(error), want, "{error}");
    }
}

#[test]
fn the_detail_is_the_error_line_on_one_line() {
    assert_eq!(
        one_line(
            "yt-dlp exited with code 1: WARNING: a | b\nERROR: [youtube] x: Requested format is not available\nWARNING: later"
        ),
        "[youtube] x: Requested format is not available"
    );
    assert_eq!(
        one_line("native resolve failed: player API answered 500"),
        "native resolve failed: player API answered 500"
    );
}
