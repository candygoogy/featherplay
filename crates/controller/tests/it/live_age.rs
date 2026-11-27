//! Live check of an age-restricted song's resolve, signed out and as the test account. Run
//! with `FEATHERPLAY_HOME=<dev home> FEATHERPLAY_COOKIES_FILE=<path> cargo nextest run -p
//! featherplay-controller --run-ignored only live_age`. Reads only: a few `player`
//! requests, and 64 KB of each stream at its start and 2 MiB in.

#![allow(
    clippy::print_stdout,
    reason = "the live test reports what it measured"
)]

use std::path::PathBuf;
use std::time::Instant;

use featherplay_core::model::TrackId;
use featherplay_streams::{ResolveError, Sidecars, StreamResolver, Via, YtDlpResolver};
use featherplay_ytm::YtMusic;

/// Age-restricted music videos: the one of issue #18 unless `LIVE_AGE_IDS` (comma-separated)
/// names others.
const SONG: &str = "TxEHAQ7-bms";

fn resolver(home: &std::path::Path) -> YtDlpResolver {
    YtDlpResolver::new(Sidecars {
        dir: home.join("sidecars"),
    })
}

#[tokio::test]
#[ignore = "live: needs FEATHERPLAY_HOME and FEATHERPLAY_COOKIES_FILE"]
async fn live_age_restricted_songs_play_signed_in() {
    let home = PathBuf::from(std::env::var("FEATHERPLAY_HOME").expect("set FEATHERPLAY_HOME"));
    let ids = std::env::var("LIVE_AGE_IDS").unwrap_or_else(|_| SONG.into());
    let path = std::env::var("FEATHERPLAY_COOKIES_FILE").expect("set FEATHERPLAY_COOKIES_FILE");
    let cookies = std::fs::read_to_string(path).expect("the cookie file");
    let api = YtMusic::from_cookie_header(cookies.trim())
        .await
        .expect("sign in");
    let signed = resolver(&home).with_signed_player(Box::new(move |video, sts| {
        let (api, video) = (api.clone(), video.to_string());
        Box::pin(async move {
            Some(
                api.stream_player(&video, sts)
                    .await
                    .map_err(|e| e.to_string()),
            )
        })
    }));
    let anon = resolver(&home);
    let http = reqwest::Client::new();
    for id in ids.split(',').map(|id| TrackId(id.trim().into())) {
        // Signed out, the resolve ends at once with the age check, no yt-dlp run.
        let t = Instant::now();
        let e = match anon.resolve(&id).await {
            Ok(s) => {
                println!(
                    "{}: plays signed out ({}), not age-gated here",
                    id.0,
                    s.via.label()
                );
                continue;
            }
            Err(e) => e,
        };
        println!(
            "{}: signed out, {e} in {:.2} s",
            id.0,
            t.elapsed().as_secs_f64()
        );
        if !matches!(e, ResolveError::AgeRestricted { .. }) {
            continue;
        }
        assert!(
            matches!(
                e,
                ResolveError::AgeRestricted {
                    signed_in: false,
                    ..
                }
            ),
            "{e:?}"
        );
        let t = Instant::now();
        let s = signed
            .resolve(&id)
            .await
            .unwrap_or_else(|e| panic!("{}: {e}", id.0));
        let took = t.elapsed().as_secs_f64();
        assert_eq!(s.via, Via::NativeSigned, "{}", id.0);
        for range in ["bytes=0-65535", "bytes=2097152-2162687"] {
            let r = http
                .get(&s.url)
                .header("Range", range)
                .send()
                .await
                .unwrap();
            let status = r.status().as_u16();
            let n = r.bytes().await.unwrap().len();
            println!(
                "{}: signed in, resolved in {took:.2} s, itag {}; {range}: {status}, {n} bytes",
                id.0, s.itag
            );
            assert_eq!((status, n), (206, 65536), "{} {range}", id.0);
        }
    }
}
