//! Live check against YouTube. Run with:
//! `FEATHERPLAY_HOME=<your dev home> cargo test -p featherplay-streams --test it -- live:: --ignored --nocapture`
//! Uses the sidecars already in `{FEATHERPLAY_HOME}/sidecars`, resolves without cookies.

use std::path::PathBuf;
use std::time::Instant;

use featherplay_core::model::TrackId;
use featherplay_streams::{Sidecars, StreamResolver, YtDlpResolver};

#[tokio::test]
#[ignore = "live: two yt-dlp resolves and two 1 MiB fetches"]
async fn live_resolve_and_range_fetch() {
    let home = PathBuf::from(std::env::var("FEATHERPLAY_HOME").expect("set FEATHERPLAY_HOME"));
    let r = YtDlpResolver::new(
        Sidecars {
            dir: home.join("sidecars"),
        },
        None,
    );
    let http = reqwest::Client::new();
    for id in ["swJOIjjW69U", "juRFjpB5Ppg"] {
        let t = Instant::now();
        let s = r
            .resolve(&TrackId(id.into()))
            .await
            .unwrap_or_else(|e| panic!("{id}: {e:?}"));
        println!(
            "{id}: resolved in {:.2} s (includes the wait for available_at), itag {}",
            t.elapsed().as_secs_f64(),
            s.itag
        );
        // High quality: Opus when the video has it, else AAC.
        assert!(
            matches!(
                (s.itag, s.mime.as_str()),
                (251, "audio/webm") | (140, "audio/mp4")
            ),
            "{id}: itag {} {}",
            s.itag,
            s.mime
        );
        let resp = http
            .get(&s.url)
            .header("Range", "bytes=0-1048575")
            .send()
            .await
            .unwrap();
        let status = resp.status().as_u16();
        let n = resp.bytes().await.unwrap().len();
        println!("{id}: Range fetch {status}, {n} bytes");
        assert_eq!(status, 206, "{id}");
        assert_eq!(n, 1 << 20, "{id}");
    }
}

/// The resolve a cold click waits for, track by track (`TIMING_IDS`, comma-separated), with
/// which path answered and one 64 KB `Range` fetch each to check the URL plays. The first
/// resolve after a new player also downloads and preprocesses it (once per player version).
/// `FEATHERPLAY_HOME=… TIMING_IDS=a,b cargo test -p featherplay-streams --test it
/// live_resolve_timing -- --ignored --nocapture`
#[tokio::test]
#[ignore = "live: one player API request and one 64 KB fetch per TIMING_IDS entry"]
async fn live_resolve_timing() {
    let home = PathBuf::from(std::env::var("FEATHERPLAY_HOME").expect("set FEATHERPLAY_HOME"));
    let sidecars = std::env::var("FEATHERPLAY_SIDECARS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| home.join("sidecars"));
    let r = YtDlpResolver::new(Sidecars { dir: sidecars }, None);
    let http = reqwest::Client::new();
    let ids = std::env::var("TIMING_IDS").expect("set TIMING_IDS");
    for (i, id) in ids.split(',').map(str::trim).enumerate() {
        if i > 0 {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        }
        let t = Instant::now();
        let s = r
            .resolve(&TrackId(id.into()))
            .await
            .unwrap_or_else(|e| panic!("{id}: {e:?}"));
        let took = t.elapsed().as_secs_f64();
        let via = r
            .take_fallback_reason(&TrackId(id.into()))
            .map_or("native".into(), |e| format!("yt-dlp ({e})"));
        let t = Instant::now();
        let resp = http
            .get(&s.url)
            .header("Range", "bytes=0-65535")
            .send()
            .await
            .unwrap();
        let status = resp.status().as_u16();
        let n = resp.bytes().await.unwrap().len();
        println!(
            "{id}: resolve {took:.3} s via {via}; 64 KB fetch {status} in {:.3} s ({n} bytes); title {:?}",
            t.elapsed().as_secs_f64(),
            s.video_title
        );
        assert_eq!(status, 206, "{id}");
        assert!(matches!(s.itag, 251 | 140), "{id}: itag {}", s.itag);
    }
}

/// A first run, on an empty home (`FEATHERPLAY_HOME`, with no `resolver` folder), as the
/// app starts one: the helpers download in the background, and how long the start takes to
/// have the native step ready (an embed page, the player's JS and its preprocessing; no
/// helper is waited for), then the first song (`TIMING_IDS`).
/// `MEASURE_PREPROCESSOR`: featherplay.exe, to preprocess in a child as the app does.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "live: the helpers' download, an embed page, a player's JS, one player API request \
            and one 64 KB fetch"]
async fn live_first_run() {
    let home = PathBuf::from(std::env::var("FEATHERPLAY_HOME").expect("set FEATHERPLAY_HOME"));
    assert!(!home.join("resolver").exists(), "not a first run");
    let t = Instant::now();
    let mut r = YtDlpResolver::new(
        Sidecars {
            dir: home.join("sidecars"),
        },
        None,
    )
    .with_downloads(featherplay_streams::Downloads::pinned());
    if let Ok(exe) = std::env::var("MEASURE_PREPROCESSOR") {
        r = r.with_preprocessor(featherplay_streams::Preprocessor::Child(exe.into()));
    }
    let r = std::sync::Arc::new(r);
    let helpers = r.clone();
    let downloads = tokio::spawn(async move {
        let done = helpers.install_helpers().await;
        println!(
            "helpers downloaded after {:.2} s: {done:?}",
            t.elapsed().as_secs_f64()
        );
    });
    r.prepare().await;
    println!(
        "the native step is ready after {:.2} s",
        t.elapsed().as_secs_f64()
    );
    let id = std::env::var("TIMING_IDS").expect("set TIMING_IDS");
    let t1 = Instant::now();
    let s = r.resolve(&TrackId(id.clone())).await.unwrap();
    let via = r
        .take_fallback_reason(&TrackId(id.clone()))
        .map_or("native".into(), |e| format!("yt-dlp ({e})"));
    println!(
        "first resolve {:.2} s via {via}; {:.2} s since the start",
        t1.elapsed().as_secs_f64(),
        t.elapsed().as_secs_f64()
    );
    let t2 = Instant::now();
    let resp = reqwest::Client::new()
        .get(&s.url)
        .header("Range", "bytes=0-65535")
        .send()
        .await
        .unwrap();
    println!(
        "64 KB fetch {} in {:.3} s",
        resp.status().as_u16(),
        t2.elapsed().as_secs_f64()
    );
    downloads.await.unwrap();
}
