//! How long each resolver step takes, and whether its URL plays (its first 1 MiB fetched,
//! as the engine's first request asks for it). A measurement, not a check: it prints one
//! line per step and never fails on a refusal. Without sign-in, five resolves by default;
//! `RESOLVE_STEPS=retry` asks one song natively three times instead (is a refused URL a
//! one-off?), `RESOLVE_STEPS=keys` asks one song three times and prints the URLs' query
//! keys, and `RESOLVE_STEPS=chain` runs only the app's chain after a refused URL:
//! `FEATHERPLAY_HOME=<dir> cargo nextest run -p featherplay-audio --run-ignored only -E 'test(resolve_steps)' --no-capture`

use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

use featherplay_core::model::TrackId;
use featherplay_streams::{Probe, ResolvedStream, Sidecars, StreamResolver, YtDlpResolver};

/// Public Creative Commons songs (see `play_probe.rs`).
const SONGS: [&str; 3] = ["Yy-NyPTRYU8", "9WQBzXr4gFc", "XgBPZ68L6TE"];
const FETCH: u64 = 1 << 20;
/// The engine tries a 403 on the first request again for 1.75 s in all.
const GATE_RETRY_SPAN: Duration = Duration::from_millis(1750);

#[tokio::test(flavor = "multi_thread")]
#[ignore = "live: a few resolves and their first 1 MiB"]
async fn resolve_steps_time_each_step_and_its_first_chunk() {
    let home = PathBuf::from(std::env::var("FEATHERPLAY_HOME").expect("set FEATHERPLAY_HOME"))
        .join("resolve-steps");
    let _ = std::fs::remove_dir_all(home.join("resolver"));
    let sidecars = || Sidecars {
        dir: home.join("sidecars"),
    };
    sidecars()
        .ensure(|_, _, _| {})
        .await
        .expect("download the sidecars");
    let http = reqwest::Client::new();
    let r = YtDlpResolver::new(sidecars(), None);
    let mode = std::env::var("RESOLVE_STEPS").unwrap_or_default();
    let steps: Vec<(&str, &str, Probe<'_>)> = if mode == "chain" {
        Vec::new()
    } else if mode == "keys" {
        // Which query keys a refused URL has that a playing one hasn't.
        vec![
            ("native, cold", SONGS[1], Probe::Native),
            ("native, again", SONGS[1], Probe::Native),
            ("native, a third time", SONGS[1], Probe::Native),
        ]
    } else if mode == "retry" {
        vec![
            ("native, cold", SONGS[1], Probe::Native),
            ("native, again", SONGS[1], Probe::Native),
            ("native, another song", SONGS[2], Probe::Native),
            ("native, a third time", SONGS[1], Probe::Native),
        ]
    } else {
        vec![
            ("native, cold (embed page, player)", SONGS[0], Probe::Native),
            ("native, warm", SONGS[1], Probe::Native),
            ("native, warm again", SONGS[0], Probe::Native),
            ("yt-dlp embedded", SONGS[1], Probe::Embedded),
            ("yt-dlp default (TV)", SONGS[0], Probe::Default),
        ]
    };
    for (name, song, step) in steps {
        let t = Instant::now();
        let resolved = r.probe(&TrackId(song.into()), step).await;
        let resolve = t.elapsed();
        let line = match resolved {
            Err(e) => format!("failed: {}", e.lines().last().unwrap_or("")),
            Ok(s) => {
                let fetched = first_chunk(&http, &s).await;
                format!(
                    "c={}, first chunk {fetched} after {} ms | keys {}",
                    param(&s.url, "c").unwrap_or("?"),
                    t.elapsed().as_millis(),
                    keys(&s.url)
                )
            }
        };
        println!(
            "STEP {name} | {song} | resolve {} ms | {line}",
            resolve.as_millis()
        );
    }
    if mode == "chain" {
        chain_after_a_refusal(&r, &http).await;
    }
}

/// The app's whole chain after one refused URL: the refused song again, as the engine asks
/// for it, then another song while URLs are in doubt.
async fn chain_after_a_refusal(r: &YtDlpResolver, http: &reqwest::Client) {
    let refused = TrackId(SONGS[1].into());
    let Ok(s) = r.probe(&refused, Probe::Native).await else {
        return;
    };
    r.refused(&refused, &s.url);
    for (name, song) in [("the refused song", SONGS[1]), ("another song", SONGS[2])] {
        let id = TrackId(song.into());
        let t = Instant::now();
        let resolved = r.resolve(&id).await;
        let resolve = t.elapsed();
        let line = match resolved {
            Ok(s) => format!(
                "c={}, first chunk {} after {} ms",
                param(&s.url, "c").unwrap_or("?"),
                first_chunk(http, &s).await,
                t.elapsed().as_millis()
            ),
            Err(e) => format!("failed: {e}"),
        };
        println!(
            "CHAIN after a refusal, {name} | {song} | resolve {} ms ({}) | {line}",
            resolve.as_millis(),
            r.take_fallback_reason(&id)
                .map_or("native".into(), |why| format!("not native: {why}"))
        );
    }
}

/// "206", "403 then 206" or "403 then 403": the first range, as the engine asks for it.
async fn first_chunk(http: &reqwest::Client, s: &ResolvedStream) -> String {
    if let Some(at) = s.available_at
        && let Ok(left) = at.duration_since(SystemTime::now())
    {
        tokio::time::sleep(left + Duration::from_millis(200)).await;
    }
    let want = s.content_length.map_or(FETCH, |len| len.min(FETCH));
    let get = || async {
        http.get(&s.url)
            .header("Range", format!("bytes=0-{}", want - 1))
            .send()
            .await
            .map(|r| r.status().as_u16())
            .unwrap_or(0)
    };
    let first = get().await;
    if first != 403 {
        return first.to_string();
    }
    tokio::time::sleep(GATE_RETRY_SPAN).await;
    format!("403 then {}", get().await)
}

/// The URL's query keys, sorted, without their values.
fn keys(url: &str) -> String {
    let mut k: Vec<&str> = url
        .split_once('?')
        .map_or("", |(_, q)| q)
        .split('&')
        .filter_map(|kv| kv.split('=').next())
        .collect();
    k.sort_unstable();
    k.join(",")
}

fn param<'a>(url: &'a str, key: &str) -> Option<&'a str> {
    url.split_once('?')?
        .1
        .split('&')
        .find_map(|kv| kv.strip_prefix(key)?.strip_prefix('='))
}
