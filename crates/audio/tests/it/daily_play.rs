//! The daily CI play check (`.github/workflows/daily-play.yml`): download the
//! sidecars, resolve a fixed public Creative Commons track without cookies, fetch it in
//! 1 MiB `Range` chunks and decode 10 s of it into a null sink. No output device is
//! needed, so it runs on a GitHub runner. Run with:
//! `FEATHERPLAY_HOME=<dir> cargo test -p featherplay-audio --test it -- daily_play:: --ignored --nocapture`
//! Sidecars go to `{FEATHERPLAY_HOME}/sidecars`; the audio stays in memory.

use std::io::Cursor;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use featherplay_core::model::TrackId;
use featherplay_streams::{Sidecars, StreamResolver, YtDlpResolver};
use rodio::Source;
use rodio::decoder::DecoderBuilder;

/// Kevin MacLeod, "Exit the Premises" (CC BY).
const CC_TRACK: &str = "Yy-NyPTRYU8";
const CHUNK: u64 = 1 << 20;
const DECODE: Duration = Duration::from_secs(10);

#[tokio::test(flavor = "multi_thread")]
#[ignore = "live: downloads the sidecars, one yt-dlp resolve and one full-track fetch"]
async fn daily_play_decodes_ten_seconds() {
    let home = PathBuf::from(std::env::var("FEATHERPLAY_HOME").expect("set FEATHERPLAY_HOME"));
    let sidecars = Sidecars {
        dir: home.join("sidecars"),
    };
    let t = Instant::now();
    sidecars
        .ensure(|_, _, _| {})
        .await
        .expect("download the sidecars");
    println!("sidecars ready in {:.1} s", t.elapsed().as_secs_f64());

    let resolver = YtDlpResolver::new(sidecars, None);
    let t = Instant::now();
    let s = match resolver.resolve(&TrackId(CC_TRACK.into())).await {
        Ok(s) => s,
        // YouTube asks requests from datacenter addresses (GitHub's runners among them) to
        // prove they aren't bots. That says nothing about playback on a home connection, so
        // the check can't tell from here: it says so (the workflow turns the line into a
        // warning) and passes, rather than reporting a breakage that isn't there.
        Err(e) if is_bot_check(&format!("{e:?}")) => {
            println!("INCONCLUSIVE: YouTube asked this machine to confirm it isn't a bot: {e:?}");
            return;
        }
        Err(e) => panic!("resolve {CC_TRACK}: {e:?}"),
    };
    println!(
        "resolved in {:.1} s: itag {}, {}, {:?} bytes",
        t.elapsed().as_secs_f64(),
        s.itag,
        s.mime,
        s.content_length
    );
    assert_eq!(s.itag, 140, "expected AAC in MP4 (itag 140)");
    let len = s.content_length.expect("content length");

    // The whole file: symphonia's MP4 reader scans every fragment before the first sample.
    let http = reqwest::Client::new();
    let mut data = Vec::with_capacity(len as usize);
    while (data.len() as u64) < len {
        let from = data.len() as u64;
        let to = (from + CHUNK).min(len) - 1;
        let resp = http
            .get(&s.url)
            .header("Range", format!("bytes={from}-{to}"))
            .send()
            .await
            .unwrap_or_else(|e| panic!("fetch bytes {from}-{to}: {e}"));
        assert_eq!(resp.status().as_u16(), 206, "bytes {from}-{to}");
        let body = resp.bytes().await.expect("chunk body");
        assert!(!body.is_empty(), "empty chunk at {from}");
        data.extend_from_slice(&body);
    }
    println!("fetched {} bytes", data.len());

    let dec = DecoderBuilder::new()
        .with_data(Cursor::new(data))
        .with_byte_len(len)
        .with_seekable(true)
        .with_hint("m4a")
        .with_mime_type("audio/mp4")
        .build()
        .expect("decoder");
    let (channels, rate) = (
        dec.channels().get() as usize,
        dec.sample_rate().get() as usize,
    );
    let want = channels * rate * DECODE.as_secs() as usize;
    // The null sink: count the samples and keep the loudest.
    let (mut n, mut peak) = (0usize, 0f32);
    for sample in dec.take(want) {
        n += 1;
        peak = peak.max(sample.abs());
    }
    println!("decoded {n} samples ({channels} ch, {rate} Hz), peak {peak:.3}");
    assert_eq!(n, want, "decoded less than {DECODE:?}");
    assert!(peak > 0.001, "10 s of silence");
}

/// YouTube's challenge for suspected bots, as yt-dlp reports it.
pub(super) fn is_bot_check(error: &str) -> bool {
    error.contains("confirm you") && error.contains("not a bot")
}

#[test]
fn a_bot_challenge_is_told_apart_from_a_breakage() {
    assert!(is_bot_check(
        "ToolFailed { code: 1, stderr_tail: \"ERROR: [youtube] Yy-NyPTRYU8: Sign in to confirm you\u{2019}re not a bot.\" }"
    ));
    assert!(!is_bot_check(
        "ToolFailed { code: 1, stderr_tail: \"ERROR: [youtube] Yy-NyPTRYU8: Requested format is not available\" }"
    ));
}
