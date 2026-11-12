//! A stream opened and decoded over the throttled server: first audio, seeks, the head in
//! memory, a server that ignores `Range`, the ad gate and stalls.

use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use super::{env, env_with_read_timeout, fixture, serve};
use crate::decoder::Decoder;
use crate::head;

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
pub(super) const SHORT_READ_TIMEOUT: Duration = Duration::from_secs(1);

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
