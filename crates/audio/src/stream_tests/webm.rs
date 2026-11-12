//! Opus in WebM (itag 251). `tone-10s.webm` is a stereo 440 Hz sine, 10 s, with its `Cues`
//! before the first `Cluster` like YouTube's (a `Cluster` per second), made with
//! `ffmpeg -f lavfi -i sine=frequency=440:sample_rate=48000:duration=10 -ac 2 -c:a libopus
//! -b:a 96k -f webm -cluster_time_limit 1000 -cues_to_front 1 tone-10s.webm`.

use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use super::{Env, env, fixture, serve};
use crate::decoder::Decoder;

pub(super) const WEBM: &str = "audio/webm; codecs=\"opus\"";
/// The frames ffmpeg decodes from `tone-10s.webm` (`ffmpeg -i tone-10s.webm -f f32le -`), with
/// the pre-skip (312) and the end padding dropped.
const WEBM_FRAMES: i64 = 480_000;
/// One Opus frame, 20 ms at 48 kHz.
const OPUS_FRAME: i64 = 960;
/// 10 ms at 48 kHz.
const TEN_MS: i64 = 480;

fn open_webm(env: &Env, url: &str) -> Decoder {
    let (media, n, _) = env.open(url);
    Decoder::open(media, n, WEBM).expect("decoder")
}

/// Decodes to the end; returns the interleaved samples.
fn samples(dec: &mut Decoder) -> Vec<f32> {
    dec.by_ref().collect()
}

/// Decodes to the end; returns the frames (samples per channel).
fn frames(dec: &mut Decoder) -> i64 {
    let channels = dec.channels().get() as i64;
    dec.by_ref().count() as i64 / channels
}

#[test]
fn webm_first_audio_long_before_the_download_ends() {
    let data = fixture("tone-10s.webm");
    let len = data.len() as u64;
    // ~150 KB at 16 KB/s: the whole file takes ~9 s.
    let server = serve(data, 16 << 10, Duration::ZERO);
    let env = env();
    let t = Instant::now();
    let (media, n, _) = env.open(&server.url);
    let mut dec = Decoder::open(media, n, WEBM).expect("decoder");
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
    assert_eq!((dec.channels().get(), dec.sample_rate().get()), (2, 48_000));
    // Half a second plays from the first `Cluster`.
    assert_eq!(dec.by_ref().take(48_000).count(), 48_000);
}

#[test]
fn webm_decodes_every_frame_without_the_pre_skip() {
    let server = serve(fixture("tone-10s.webm"), 1 << 20, Duration::ZERO);
    let env = env();
    let mut dec = open_webm(&env, &server.url);
    let n = frames(&mut dec);
    // With the pre-skip left in, it's one Opus frame too many.
    assert!(
        (n - WEBM_FRAMES).abs() < OPUS_FRAME,
        "{n} frames decoded, ffmpeg gives {WEBM_FRAMES}"
    );
}

#[test]
fn webm_tone_has_the_right_pitch() {
    let server = serve(fixture("tone-10s.webm"), 1 << 20, Duration::ZERO);
    let env = env();
    let mut dec = open_webm(&env, &server.url);
    let all = samples(&mut dec);
    let left: Vec<f32> = all.iter().step_by(2).copied().collect();
    let rising = left
        .windows(2)
        .filter(|w| w[0] < 0.0 && w[1] >= 0.0)
        .count();
    let hz = rising as f64 / (left.len() as f64 / 48_000.0);
    assert!((hz - 440.0).abs() < 2.0, "{hz} Hz");
}

#[test]
fn webm_seek_lands_on_the_target_and_back_to_the_start() {
    let server = serve(fixture("tone-10s.webm"), 1 << 20, Duration::ZERO);
    let env = env();
    let mut dec = open_webm(&env, &server.url);
    let whole = samples(&mut dec);
    let total = whole.len() as i64 / 2;
    dec.try_seek(Duration::from_secs(5)).expect("seek");
    let left = frames(&mut dec);
    let landed = total - left;
    assert!(
        (landed - 240_000).abs() <= TEN_MS,
        "landed at frame {landed} for 5 s (240000)"
    );
    dec.try_seek(Duration::ZERO).expect("seek to the start");
    let again = samples(&mut dec);
    assert_eq!(
        again.len(),
        whole.len(),
        "a different length from the start"
    );
    assert!(
        again[..9600] == whole[..9600],
        "the start plays differently"
    );
}

#[test]
fn webm_seeks_twice_before_reading_and_past_the_end() {
    let server = serve(fixture("tone-10s.webm"), 1 << 20, Duration::ZERO);
    let env = env();
    let mut dec = open_webm(&env, &server.url);
    let total = frames(&mut dec);
    // A drag across the seek bar: the second seek wins.
    dec.try_seek(Duration::from_secs(8)).expect("seek");
    dec.try_seek(Duration::from_secs(2)).expect("seek back");
    let landed = total - frames(&mut dec);
    assert!(
        (landed - 96_000).abs() <= TEN_MS,
        "landed at frame {landed} for 2 s (96000)"
    );
    dec.try_seek(Duration::from_secs(30))
        .expect("seek past the end");
    assert_eq!(dec.next(), None);
}
