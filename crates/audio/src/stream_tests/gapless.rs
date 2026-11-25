//! Gapless joins: one sweep cut into two files at a sample, played one after the other on
//! the null output, and the join measured in what the output played.
//!
//! `sweep-a` and `sweep-b` are the two halves (2 s each) of a 4 s stereo chirp,
//! `0.5 sin(2π(200 t + 150 t²))`, made with `ffmpeg -f lavfi -i "aevalsrc=E|E:s=44100:d=4"`
//! (and `s=48000` for the WebM pair), cut with `atrim=end_sample=` / `atrim=start_sample=`
//! (88200 at 44.1 kHz, 96000 at 48 kHz). The m4a pair is laid out like YouTube's DASH
//! streams (`-movflags +dash+global_sidx -frag_duration 1000000`), with the edit list
//! YouTube's carry added after (`elst` of one entry: the length, and the encoder delay
//! of 1024, ffmpeg's AAC encoder's). The WebM pair is `-c:a libopus -b:a 96k -f webm
//! -cluster_time_limit 1000 -cues_to_front 1`: its last block carries the end padding
//! (`DiscardPadding`), and its `OpusHead` the pre-skip.

use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use super::webm::WEBM;
use super::{engine_tmp, env, fixture, local, serve, src, wait_event};
use crate::decoder::Decoder;
use crate::{AudioEngine, AudioEvent, SkipSilence, StreamSource};

/// The output's rate (the null output's).
const OUT: f64 = 44_100.0;
/// Where the source was cut, in seconds.
const CUT: f64 = 2.0;
/// Frames of the window compared on each side of the join: 50 ms.
const WINDOW: usize = 2205;

/// The source the two halves were cut from, at `t` seconds.
fn sweep(t: f64) -> f32 {
    (0.5 * (2.0 * std::f64::consts::PI * (200.0 * t + 150.0 * t * t)).sin()) as f32
}

#[derive(Clone, Copy, Debug)]
enum Format {
    Aac,
    Opus,
}

impl Format {
    fn pair(self) -> (StreamSource, StreamSource) {
        let (ext, mime) = match self {
            Format::Aac => ("m4a", "audio/mp4"),
            Format::Opus => ("webm", WEBM),
        };
        let with = |name: String| StreamSource {
            mime: mime.into(),
            ..local(&name)
        };
        (
            with(format!("sweep-a.{ext}")),
            with(format!("sweep-b.{ext}")),
        )
    }

    /// The source's rate, and the frames in each half.
    fn rate(self) -> u32 {
        match self {
            Format::Aac => 44_100,
            Format::Opus => 48_000,
        }
    }
}

/// The offset `o` in `from..to` at which `left[o + i]` best matches the source from `t0`
/// on (`sweep(t0 + i / rate)`), over `WINDOW` frames: where the source's time 0 falls in
/// `left`, judged by that stretch of it. Also the match's correlation (1 is exact).
fn align(left: &[f32], rate: f64, t0: f64, from: usize, to: usize) -> (usize, f32) {
    let start = (t0 * rate).round() as usize;
    let want: Vec<f32> = (0..WINDOW).map(|i| sweep(t0 + i as f64 / rate)).collect();
    let energy: f32 = want.iter().map(|w| w * w).sum();
    let mut best = (from, f32::MIN);
    for o in from..to {
        let Some(got) = left.get(o + start..o + start + WINDOW) else {
            break;
        };
        let (mut dot, mut e) = (0.0f32, 0.0f32);
        for (g, w) in got.iter().zip(&want) {
            dot += g * w;
            e += g * g;
        }
        let r = dot / (energy * e).sqrt().max(f32::MIN_POSITIVE);
        if r > best.1 {
            best = (o, r);
        }
    }
    best
}

/// The longest run of silent frames (under −60 dBFS) in `left[from..to]`.
fn longest_silence(left: &[f32], from: usize, to: usize) -> usize {
    let (mut run, mut longest) = (0, 0);
    for s in &left[from.min(left.len())..to.min(left.len())] {
        if s.abs() < 0.001 {
            run += 1;
            longest = longest.max(run);
        } else {
            run = 0;
        }
    }
    longest
}

/// How a join came out in what the output played.
#[derive(Debug)]
struct Join {
    /// How much later the second half came than it would have in the uncut source:
    /// silence or padding added at the join, negative for audio lost. In ms.
    late_ms: f64,
    /// The longest run of silence around the join, in ms.
    silence_ms: f64,
    /// How well each side matched the source (1 is exact).
    fit: (f32, f32),
}

/// Measures the join in `out`, what the output played (stereo at `OUT`).
fn measure(out: &[f32]) -> Join {
    let left: Vec<f32> = out.iter().step_by(2).copied().collect();
    let first = left
        .iter()
        .position(|s| s.abs() >= 0.001)
        .expect("the output played sound");
    // The first half's last 50 ms before the cut, the second's first 50 ms after it.
    let before = CUT - 0.06;
    let after = CUT + 0.01;
    let (a, fit_a) = align(&left, OUT, before, first.saturating_sub(4000), first + 4000);
    let (b, fit_b) = align(&left, OUT, after, a.saturating_sub(4000), a + 20_000);
    let late = b as f64 - a as f64;
    let silence = longest_silence(
        &left,
        a + (before * OUT) as usize,
        b + ((after * OUT) as usize) + WINDOW,
    );
    Join {
        late_ms: late / OUT * 1000.0,
        silence_ms: silence as f64 / OUT * 1000.0,
        fit: (fit_a, fit_b),
    }
}

/// How the second half takes over from the first.
#[derive(Clone, Copy, Debug)]
enum Handover {
    /// As without a join: on `Ended`, a `load` and a `play`, as the controller runs the
    /// player's `Load` and `Play`.
    Load,
    /// `join_next`.
    Join,
}

/// The first of `Joined` and `Ended` within `timeout`; panics on an error or neither.
fn end(events: &Receiver<AudioEvent>, timeout: Duration) -> AudioEvent {
    let t = Instant::now();
    loop {
        match events.recv_timeout(timeout.saturating_sub(t.elapsed())) {
            Ok(AudioEvent::Error(e)) => panic!("engine error: {e}"),
            Ok(ev @ (AudioEvent::Joined | AudioEvent::Ended)) => return ev,
            Ok(_) => {}
            Err(_) => panic!("no end within {timeout:?}"),
        }
    }
}

/// Plays `a` from `from`, then `b`, on the null output, `b` preloaded once `a` starts.
/// Returns what the output played.
fn play(a: StreamSource, from: Duration, b: StreamSource, how: Handover) -> Vec<f32> {
    let engine = AudioEngine::new_null(engine_tmp());
    let events = engine.events();
    engine.capture();
    engine.load(a, from);
    engine.play();
    wait_event(&events, Duration::from_secs(3), AudioEvent::Started);
    engine.preload(b.clone());
    match how {
        Handover::Load => {
            wait_event(&events, Duration::from_secs(10), AudioEvent::Ended);
            engine.load(b, Duration::ZERO);
            engine.play();
        }
        Handover::Join => {
            engine.join_next(Some(&b.url));
            assert_eq!(end(&events, Duration::from_secs(10)), AudioEvent::Joined);
        }
    }
    wait_event(&events, Duration::from_secs(3), AudioEvent::Started);
    std::thread::sleep(Duration::from_millis(300));
    engine.take_capture()
}

/// Decodes `src` to its end: the frames it gave, and where the source's time 0 (for the
/// second half, the cut) falls in them, in frames: 0 when the codec's delay is dropped.
fn decode(src: &StreamSource, fmt: Format, second: bool) -> (usize, usize) {
    let env = env();
    let (media, n, _) = env.open(&src.url);
    let mut dec = Decoder::open(media, n, &src.mime).expect("decoder");
    let channels = usize::from(dec.channels().get());
    let all: Vec<f32> = dec.by_ref().collect();
    let rate = f64::from(fmt.rate());
    // The second half is the source from the cut on: the first half's place goes in front
    // of it, so `align` finds the source's time 0 at 0.
    let pad = if second { (CUT * rate) as usize } else { 0 };
    let mut left = vec![0.0; pad];
    left.extend(all.iter().step_by(channels));
    let t0 = if second { CUT + 0.01 } else { 0.01 };
    let (o, fit) = align(&left, rate, t0, 0, 4000);
    assert!(
        fit > 0.99,
        "{fmt:?}: the decoded half isn't the source ({fit})"
    );
    (left.len() - pad, o)
}

#[test]
fn gapless_halves_decode_to_the_source_with_no_delay_or_padding() {
    for fmt in [Format::Aac, Format::Opus] {
        let (a, b) = fmt.pair();
        let half = (CUT * f64::from(fmt.rate())) as usize;
        let (na, lead_a) = decode(&a, fmt, false);
        let (nb, lead_b) = decode(&b, fmt, true);
        assert_eq!((na, nb), (half, half), "{fmt:?}: frames decoded");
        assert_eq!(
            (lead_a, lead_b),
            (0, 0),
            "{fmt:?}: the codec's delay left in"
        );
    }
}

/// The audio thread goes on into the next track on the sample after the last, so this
/// holds at any machine speed.
#[test]
fn gapless_join_plays_the_cut_source_with_no_gap() {
    for fmt in [Format::Aac, Format::Opus] {
        let (a, b) = fmt.pair();
        let join = measure(&play(a, Duration::ZERO, b, Handover::Join));
        assert!(join.fit.0 > 0.99 && join.fit.1 > 0.99, "{fmt:?}: {join:?}");
        assert!(
            join.late_ms.abs() < 1.0 && join.silence_ms < 1.0,
            "{fmt:?}: {join:?}"
        );
    }
}

/// A preload without its first samples at the end (its server is slow) isn't joined: the
/// track ends as without a join, and the load then plays it.
#[test]
fn gapless_join_falls_back_when_the_preload_isnt_ready() {
    let server = serve(fixture("sweep-b.m4a"), 1 << 20, Duration::from_secs(3));
    let (a, _) = Format::Aac.pair();
    let b = src(&server.url);
    let engine = AudioEngine::new_null(engine_tmp());
    let events = engine.events();
    engine.load(a, Duration::from_millis(1500));
    engine.play();
    wait_event(&events, Duration::from_secs(3), AudioEvent::Started);
    engine.preload(b.clone());
    engine.join_next(Some(&b.url));
    assert_eq!(end(&events, Duration::from_secs(3)), AudioEvent::Ended);
    engine.load(b, Duration::ZERO);
    engine.play();
    wait_event(&events, Duration::from_secs(10), AudioEvent::Started);
}

/// Skip silence would cut into the join: with it on, the track ends as without one.
#[test]
fn gapless_join_leaves_the_end_to_skip_silence() {
    let (a, b) = Format::Opus.pair();
    let engine = AudioEngine::new_null(engine_tmp());
    let events = engine.events();
    engine.set_skip_silences(SkipSilence::StartAndEnd);
    engine.load(a, Duration::from_millis(1500));
    engine.play();
    wait_event(&events, Duration::from_secs(3), AudioEvent::Started);
    engine.preload(b.clone());
    engine.join_next(Some(&b.url));
    assert_eq!(end(&events, Duration::from_secs(3)), AudioEvent::Ended);
}

/// `FEATHERPLAY_GAPLESS_PAIR`: two files of an album that join with no gap, `a|b`, of one
/// format (`.m4a` or `.webm`).
fn real_pair() -> Option<(StreamSource, StreamSource)> {
    let pair = std::env::var("FEATHERPLAY_GAPLESS_PAIR").ok()?;
    let (a, b) = pair.split_once('|')?;
    let stream = |path: &str| {
        let mime = if path.ends_with(".webm") {
            WEBM
        } else {
            "audio/mp4"
        };
        let path = path.replace('\\', "/");
        StreamSource {
            mime: mime.into(),
            ..src(&format!("file:///{}", path.trim_start_matches('/')))
        }
    };
    Some((stream(a), stream(b)))
}

/// The frames `src` decodes to, and its rate.
fn frames(src: &StreamSource) -> (usize, u32) {
    let env = env();
    let (media, n, _) = env.open(&src.url);
    let mut dec = Decoder::open(media, n, &src.mime).expect("decoder");
    let (ch, rate) = (dec.channels().get(), dec.sample_rate().get());
    (dec.by_ref().count() / usize::from(ch), rate)
}

/// The longest silence within 300 ms of the join in `out`, in ms: the first track played
/// its last `before` up to it.
fn silence_at_join(out: &[f32], before: Duration) -> f64 {
    let left: Vec<f32> = out.iter().step_by(2).copied().collect();
    let first = left.iter().position(|s| *s != 0.0).expect("sound");
    let join = first + (before.as_secs_f64() * OUT) as usize;
    let near = (0.3 * OUT) as usize;
    longest_silence(&left, join.saturating_sub(near), join + near) as f64 / OUT * 1000.0
}

#[test]
#[ignore = "measurement: prints each format's join, loaded and joined (and a real pair's)"]
fn gapless_measure() {
    for fmt in [Format::Aac, Format::Opus] {
        for how in [Handover::Load, Handover::Join] {
            for run in 0..3 {
                let (a, b) = fmt.pair();
                let join = measure(&play(a, Duration::ZERO, b, how));
                println!("{fmt:?}, {how:?}, run {run}: {join:?}");
            }
        }
    }
    let Some((a, b)) = real_pair() else {
        println!("no FEATHERPLAY_GAPLESS_PAIR: no real pair");
        return;
    };
    let (na, rate) = frames(&a);
    println!("real pair: {na} + {} frames decoded", frames(&b).0);
    let before = Duration::from_secs(3);
    let from = Duration::from_secs_f64(na as f64 / f64::from(rate)) - before;
    for how in [Handover::Load, Handover::Join] {
        for run in 0..3 {
            let out = play(a.clone(), from, b.clone(), how);
            let ms = silence_at_join(&out, before);
            println!("real pair, {how:?}, run {run}: silence at the join {ms:.1} ms");
        }
    }
}
