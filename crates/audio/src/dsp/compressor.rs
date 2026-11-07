//! "Even out loudness": a feed-forward compressor with a soft knee, after Giannoulis,
//! Massberg and Reiss, "Digital Dynamic Range Compressor Design" (JAES 2012), the method
//! behind rodio's `limit.rs`. The channels are linked: one gain from the loudest channel's
//! peak, so the stereo image doesn't move. The gain is computed once per `BLOCK` frames, in
//! dB, and ramped linearly across the block.

/// How hard the compressor works. Off by default.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Compressor {
    #[default]
    Off,
    /// Gentle: −24 dB, 3:1, a 10 dB knee.
    Light,
    /// Pear's Audio Compressor: −50 dB, 12:1, a 40 dB knee.
    Strong,
}

/// Frames per gain step.
const BLOCK: usize = 16;
/// The detector's floor, so silence has a level.
const FLOOR_DB: f32 = -120.0;

/// The gain computer and the smoothing times of one setting.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Curve {
    threshold: f32,
    ratio: f32,
    knee: f32,
    attack_s: f32,
    release_s: f32,
}

impl Compressor {
    fn curve(self) -> Option<Curve> {
        match self {
            Compressor::Off => None,
            Compressor::Light => Some(Curve {
                threshold: -24.0,
                ratio: 3.0,
                knee: 10.0,
                attack_s: 0.005,
                release_s: 0.25,
            }),
            Compressor::Strong => Some(Curve {
                threshold: -50.0,
                ratio: 12.0,
                knee: 40.0,
                attack_s: 0.001,
                release_s: 0.25,
            }),
        }
    }
}

impl Curve {
    /// The gain, in dB (0 or less), for a level of `x` dB: the soft-knee static curve. As in
    /// WebAudio (and so Pear), the knee starts at the threshold and spans `knee` dB above it;
    /// Giannoulis's knee is centred on its threshold, so it's centred half a knee up.
    fn gain_db(&self, x: f32) -> f32 {
        let centre = self.threshold + self.knee / 2.0;
        let over = x - centre;
        let y = if 2.0 * over < -self.knee {
            x
        } else if 2.0 * over.abs() <= self.knee {
            let k = over + self.knee / 2.0;
            x + (1.0 / self.ratio - 1.0) * k * k / (2.0 * self.knee)
        } else {
            centre + over / self.ratio
        };
        y - x
    }

    /// Makeup gain in dB, as WebAudio's `DynamicsCompressorNode` makes it up (Chromium's
    /// `dynamics_compressor.cc`): the inverse of the curve's gain at full scale, to the
    /// power 0.6.
    fn makeup_db(&self) -> f32 {
        -0.6 * self.gain_db(0.0)
    }
}

/// How far the detector looks ahead: the signal is delayed by this much, so the gain is
/// already down when a transient after a quiet passage comes out, rather than the safety
/// limiter clipping it.
const LOOKAHEAD_S: f32 = 0.002;

/// The compressor's state for one output.
pub(crate) struct State {
    curve: Option<Curve>,
    makeup_db: f32,
    /// Per-block smoothing coefficients.
    attack: f32,
    release: f32,
    /// The smoothed gain reduction, in dB.
    smoothed_db: f32,
    /// The linear gain the last block ended on.
    gain: f32,
    rate: u32,
    /// The look-ahead delay line, interleaved, and the slot of its oldest sample.
    delay: Vec<f32>,
    at: usize,
}

impl State {
    /// Allocates the look-ahead for `channels`; call it off the audio thread.
    pub fn new(channels: usize, rate: u32) -> Self {
        let rate = rate.max(1);
        let frames = ((LOOKAHEAD_S * rate as f32).round() as usize).max(1);
        let mut s = State {
            curve: None,
            makeup_db: 0.0,
            attack: 0.0,
            release: 0.0,
            smoothed_db: 0.0,
            gain: 1.0,
            rate,
            delay: vec![0.0; frames * channels.max(1)],
            at: 0,
        };
        s.set(Compressor::Off);
        s
    }

    pub fn is_on(&self) -> bool {
        self.curve.is_some()
    }

    /// Takes a new setting. Between Light and Strong the smoothed gain carries over; coming
    /// on, the look-ahead starts empty rather than with what it held when last used.
    pub fn set(&mut self, c: Compressor) {
        let was_on = self.is_on();
        self.curve = c.curve();
        let Some(curve) = self.curve else {
            self.smoothed_db = 0.0;
            self.gain = 1.0;
            return;
        };
        if !was_on {
            self.delay.fill(0.0);
            self.at = 0;
        }
        self.makeup_db = curve.makeup_db();
        let block_s = BLOCK as f32 / self.rate as f32;
        self.attack = (-block_s / curve.attack_s).exp();
        self.release = (-block_s / curve.release_s).exp();
    }

    /// Takes `other`'s settings and state, without allocating. Both were made for the same
    /// output.
    pub fn copy_from(&mut self, other: &State) {
        self.curve = other.curve;
        self.makeup_db = other.makeup_db;
        self.attack = other.attack;
        self.release = other.release;
        self.smoothed_db = other.smoothed_db;
        self.gain = other.gain;
        self.rate = other.rate;
        self.delay.copy_from_slice(&other.delay);
        self.at = other.at;
    }

    /// Compresses `buf`, interleaved with `channels` channels (as `new` was given). Its
    /// output is `LOOKAHEAD_S` behind its input.
    pub fn process(&mut self, buf: &mut [f32], channels: usize) {
        let Some(curve) = self.curve else { return };
        let channels = channels.max(1);
        for block in buf.chunks_mut(BLOCK * channels) {
            // The level of what goes in now, which comes out a look-ahead later.
            let peak = block.iter().fold(0f32, |m, s| m.max(s.abs()));
            let level = if peak > 0.0 {
                (20.0 * peak.log10()).max(FLOOR_DB)
            } else {
                FLOOR_DB
            };
            let target = curve.gain_db(level);
            let coeff = if target < self.smoothed_db {
                self.attack
            } else {
                self.release
            };
            self.smoothed_db = coeff * self.smoothed_db + (1.0 - coeff) * target;
            // Settled: no subnormal creeping towards zero on silence.
            if self.smoothed_db.abs() < 1e-6 {
                self.smoothed_db = 0.0;
            }
            let to = 10f32.powf((self.smoothed_db + self.makeup_db) / 20.0);
            let frames = block.len() / channels;
            let step = (to - self.gain) / frames.max(1) as f32;
            for (i, frame) in block.chunks_exact_mut(channels).enumerate() {
                let g = self.gain + step * (i + 1) as f32;
                for s in frame {
                    let delayed = std::mem::replace(&mut self.delay[self.at], *s);
                    self.at = (self.at + 1) % self.delay.len();
                    *s = delayed * g;
                }
            }
            self.gain = to;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    /// The RMS level, in dBFS, of 2 s of a 1 kHz sine at `level_db` peak, after `c`, over the
    /// last second.
    fn level_after(c: Compressor, level_db: f32) -> f32 {
        let amp = 10f32.powf(level_db / 20.0);
        let mut st = State::new(2, RATE);
        st.set(c);
        let frames = 2 * RATE as usize;
        let mut buf: Vec<f32> = (0..frames)
            .flat_map(|i| {
                let s = amp * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / RATE as f32).sin();
                [s, s]
            })
            .collect();
        for block in buf.chunks_mut(960) {
            st.process(block, 2);
        }
        let tail = &buf[buf.len() / 2..];
        let rms = (tail.iter().map(|s| s * s).sum::<f32>() / tail.len() as f32).sqrt();
        20.0 * rms.log10()
    }

    /// A sine's RMS sits 3 dB under its peak.
    const SINE_RMS_DB: f32 = -3.01;

    #[test]
    fn strong_brings_a_loud_sine_down() {
        let after = level_after(Compressor::Strong, -6.0);
        assert!(after < -6.0 + SINE_RMS_DB - 3.0, "{after:.1} dBFS");
    }

    #[test]
    fn strong_brings_a_quiet_sine_up() {
        let after = level_after(Compressor::Strong, -60.0);
        assert!(after > -60.0 + SINE_RMS_DB + 3.0, "{after:.1} dBFS");
    }

    #[test]
    fn off_leaves_the_samples_alone() {
        let after = level_after(Compressor::Off, -20.0);
        assert!(
            (after - (-20.0 + SINE_RMS_DB)).abs() < 0.01,
            "{after:.2} dBFS"
        );
    }

    #[test]
    fn a_loud_hit_after_a_quiet_passage_comes_out_compressed() {
        // A second at −60 dBFS sets the gain well up; then a hit at −1 dBFS. The look-ahead
        // brings the gain down before the hit comes out, so it stays under full scale.
        let mut st = State::new(2, RATE);
        st.set(Compressor::Strong);
        // A cosine, so the hit starts at its peak, as a drum does.
        let sine = |amp: f32, n: usize| -> Vec<f32> {
            (0..n)
                .flat_map(|i| {
                    let s =
                        amp * (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / RATE as f32).cos();
                    [s, s]
                })
                .collect()
        };
        for mut block in sine(0.001, RATE as usize).chunks(960).map(<[f32]>::to_vec) {
            st.process(&mut block, 2);
        }
        let mut hit = sine(0.89, 4800);
        for block in hit.chunks_mut(960) {
            st.process(block, 2);
        }
        let peak = hit.iter().fold(0f32, |m, s| m.max(s.abs()));
        assert!(peak < 1.0, "peak {peak}");
    }

    #[test]
    fn the_smoothed_gain_settles_to_zero_on_silence() {
        let mut st = State::new(2, RATE);
        st.set(Compressor::Strong);
        let mut loud = vec![0.9f32; 4800];
        st.process(&mut loud, 2);
        for _ in 0..100 {
            let mut quiet = vec![0f32; 4800];
            st.process(&mut quiet, 2);
        }
        assert!(st.smoothed_db == 0.0 || st.smoothed_db.is_normal());
    }
}
