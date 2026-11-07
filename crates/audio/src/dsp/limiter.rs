//! The safety limiter at the end of the bus, while the equalizer or the compressor is on: a
//! peak limiter with an instant attack, so no sample leaves above `CEILING`, and a 50 ms
//! release. Linked across channels.

/// −0.3 dBFS.
pub(crate) const CEILING: f32 = 0.966_050_9;
/// How long the gain takes to come back (to 1/e of the way).
const RELEASE_S: f32 = 0.05;

#[derive(Clone, Copy)]
pub(crate) struct Limiter {
    gain: f32,
    /// Per-frame step of the release towards 1.
    release: f32,
}

impl Limiter {
    pub fn new(rate: u32) -> Self {
        Limiter {
            gain: 1.0,
            release: 1.0 - (-1.0 / (RELEASE_S * rate.max(1) as f32)).exp(),
        }
    }

    /// Starts again at full gain (it was off for a while).
    pub fn reset(&mut self) {
        self.gain = 1.0;
    }

    /// Limits `buf`, interleaved with `channels` channels.
    pub fn process(&mut self, buf: &mut [f32], channels: usize) {
        for frame in buf.chunks_exact_mut(channels.max(1)) {
            self.gain += (1.0 - self.gain) * self.release;
            let peak = frame.iter().fold(0f32, |m, s| m.max(s.abs()));
            if peak * self.gain > CEILING {
                // A hair under, so rounding can't land the peak above the ceiling.
                self.gain = CEILING / peak * (1.0 - 1e-6);
            }
            for s in frame {
                *s *= self.gain;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_full_scale_sine_boosted_6_db_stays_under_the_ceiling() {
        let rate = 48_000;
        let mut lim = Limiter::new(rate);
        let mut buf: Vec<f32> = (0..rate as usize)
            .flat_map(|i| {
                let s = 2.0 * (2.0 * std::f32::consts::PI * 440.0 * i as f32 / rate as f32).sin();
                [s, -s]
            })
            .collect();
        for block in buf.chunks_mut(960) {
            lim.process(block, 2);
        }
        let peak = buf.iter().fold(0f32, |m, s| m.max(s.abs()));
        assert!(peak <= CEILING, "peak {peak}");
        assert!(20.0 * peak.log10() <= -0.3 + 1e-4);
        // It limits, it doesn't mute.
        assert!(peak > 0.9 * CEILING, "peak {peak}");
    }

    #[test]
    fn the_ceiling_is_minus_0_3_dbfs() {
        assert!((20.0 * CEILING.log10() + 0.3).abs() < 1e-5);
    }
}
