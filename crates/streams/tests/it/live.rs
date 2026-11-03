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
        assert_eq!(s.itag, 140);
        assert_eq!(s.mime, "audio/mp4");
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
            .take_fallback_reason()
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
        assert_eq!(s.itag, 140);
    }
}
