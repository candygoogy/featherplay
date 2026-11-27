//! Samples over full scale, as a loud master's lossy encoding has: they reach the chain as
//! they are and its gains turn them down, with no clipping on the way. `overs.webm` (Opus,
//! 48 kHz) and `overs.m4a` (AAC, 44.1 kHz) are 1 s of a square wave at 0.98 on each channel
//! (220 Hz left, 330 Hz right), whose encodings ring to ~1.3, made with
//! `ffmpeg -f lavfi -i "aevalsrc=0.98*sgn(sin(2*PI*220*t))|0.98*sgn(sin(2*PI*330*t)):s=48000:d=1"
//! -c:a libopus -b:a 96k -f webm -cluster_time_limit 1000 -cues_to_front 1 overs.webm`, and
//! the same at `s=44100` with `-c:a aac -b:a 96k -f mp4 -movflags +dash+global_sidx
//! -frag_duration 1000000 overs.m4a`.

use std::time::Duration;

use super::webm::WEBM;
use super::{engine_tmp, env, local, wait_event};
use crate::decoder::Decoder;
use crate::{AudioEngine, AudioEvent, StreamSource};

fn overs(ext: &str) -> StreamSource {
    let mime = if ext == "webm" { WEBM } else { "audio/mp4" };
    StreamSource {
        mime: mime.into(),
        ..local(&format!("overs.{ext}"))
    }
}

fn peak(samples: &[f32]) -> f32 {
    samples.iter().fold(0.0, |m, s| m.max(s.abs()))
}

/// The longest run of one channel's samples that hold the same value, among those above
/// `floor`: a clipped stretch is flat at its ceiling.
fn longest_flat_top(samples: &[f32], floor: f32) -> usize {
    let mut longest = 0;
    for ch in 0..2 {
        let mut run = 1;
        let mut prev = f32::NAN;
        for &s in samples.iter().skip(ch).step_by(2) {
            run = if s == prev && s.abs() > floor {
                run + 1
            } else {
                1
            };
            longest = longest.max(run);
            prev = s;
        }
    }
    longest
}

#[test]
fn the_decoders_keep_samples_over_full_scale() {
    let env = env();
    for ext in ["webm", "m4a"] {
        let src = overs(ext);
        let (media, n, _) = env.open(&src.url);
        let mut dec = Decoder::open(media, n, &src.mime).expect("decoder");
        let all: Vec<f32> = dec.by_ref().collect();
        assert!(peak(&all) > 1.1, "{ext}: peak {}", peak(&all));
        assert!(longest_flat_top(&all, 0.9) < 3, "{ext}: clipped");
    }
}

/// At a volume below full, what plays has the overs turned down with the rest, not clipped
/// at full scale before the gain.
#[test]
fn the_chain_turns_overs_down_without_clipping_them() {
    let engine = AudioEngine::new_null(engine_tmp());
    let events = engine.events();
    engine.set_volume(0.8);
    let gain = crate::perceptual_gain(0.8);
    engine.capture();
    engine.load(overs("webm"), Duration::ZERO);
    engine.play();
    wait_event(&events, Duration::from_secs(3), AudioEvent::Started);
    wait_event(&events, Duration::from_secs(5), AudioEvent::Ended);
    let out = engine.take_capture();
    assert!(
        peak(&out) > 1.1 * gain,
        "peak {} at gain {gain}",
        peak(&out)
    );
    assert!(longest_flat_top(&out, 0.9 * gain) < 3, "clipped");
}
