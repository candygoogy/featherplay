//! Counters for audio that came out late: output callbacks that came after the device had
//! played its whole buffer (an audible gap), or nearly (ALSA's rule, `Alsa`), and rings
//! that ran dry while a track played.
//! Atomics only, so the audio thread can count without locking or allocating. The engine
//! logs them when playback pauses or a track ends, never on a timer.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

#[derive(Default)]
pub(crate) struct Glitches {
    /// Callbacks that came more than a device buffer after the one before (on ALSA, less a
    /// quarter period).
    late: AtomicU32,
    /// The longest gap between two callbacks, in microseconds (on ALSA, from the first one
    /// the device paced).
    max_gap_us: AtomicU32,
    /// Times a playing track's ring ran dry (the decoder fell behind, or a stalled download).
    starved: AtomicU32,
    /// Bumped each time the output starts, so the first callback after a stop isn't late.
    epoch: AtomicU32,
}

/// What the counters held at one point; see `Glitches::snapshot`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    pub late: u32,
    pub max_gap: Duration,
    pub starved: u32,
}

impl Glitches {
    pub fn starved(&self) {
        self.starved.fetch_add(1, Ordering::Relaxed);
    }

    /// The output (re)starts: the gap until its first callback doesn't count.
    pub fn restarted(&self) {
        self.epoch.fetch_add(1, Ordering::Relaxed);
    }

    /// How many times the output started. For tests.
    #[cfg(test)]
    pub fn starts(&self) -> u32 {
        self.epoch.load(Ordering::Relaxed)
    }

    pub fn snapshot(&self) -> Counts {
        Counts {
            late: self.late.load(Ordering::Relaxed),
            max_gap: Duration::from_micros(self.max_gap_us.load(Ordering::Relaxed).into()),
            starved: self.starved.load(Ordering::Relaxed),
        }
    }

    /// Clears the longest gap, so each report says the longest since the last one.
    pub fn reset_max_gap(&self) {
        self.max_gap_us.store(0, Ordering::Relaxed);
    }
}

/// Times the output callbacks of one stream, on its audio thread.
pub(crate) struct CallbackClock {
    rule: Rule,
    last: Option<(Instant, u32)>,
}

/// When a gap between two callbacks counts as late.
enum Rule {
    /// After a gap longer than this, how long the device plays from one full buffer. WASAPI,
    /// whose callback refills the whole buffer, and the tests' null output.
    Buffer(Duration),
    /// ALSA (Linux): see `Alsa`.
    Alsa(Alsa),
}

impl CallbackClock {
    #[cfg_attr(
        all(target_os = "linux", not(test)),
        allow(dead_code, reason = "WASAPI's rule, and the null output's in tests")
    )]
    pub fn new(buffer: Duration) -> Self {
        Self {
            rule: Rule::Buffer(buffer),
            last: None,
        }
    }

    /// For an ALSA stream of `channels` at `rate`.
    #[cfg_attr(
        not(any(target_os = "linux", test)),
        allow(dead_code, reason = "ALSA's rule")
    )]
    pub fn alsa(channels: u16, rate: u32) -> Self {
        Self {
            rule: Rule::Alsa(Alsa {
                channels: u64::from(channels.max(1)),
                rate: rate.max(1),
                filling: None,
                buffer: 0,
            }),
            last: None,
        }
    }

    /// One callback, now, asked for `samples` (all channels). Only ALSA's rule uses them.
    pub fn tick(&mut self, g: &Glitches, samples: usize) {
        self.tick_at(Instant::now(), g, samples);
    }

    fn tick_at(&mut self, now: Instant, g: &Glitches, samples: usize) {
        let epoch = g.epoch.load(Ordering::Relaxed);
        if let Some((last, e)) = self.last
            && e == epoch
        {
            let gap = now - last;
            match &mut self.rule {
                Rule::Buffer(buffer) => {
                    let us = u32::try_from(gap.as_micros()).unwrap_or(u32::MAX);
                    g.max_gap_us.fetch_max(us, Ordering::Relaxed);
                    if gap > *buffer {
                        g.late.fetch_add(1, Ordering::Relaxed);
                    }
                }
                Rule::Alsa(a) => a.gap(gap, samples, g),
            }
        } else if let Rule::Alsa(a) = &mut self.rule {
            a.filling = Some(samples as u64 / a.channels);
        }
        self.last = Some((now, epoch));
    }
}

/// ALSA's rule. cpal sets ALSA up with a buffer of two periods or more and calls back each
/// time a period of it is free, so a callback a little over a period after the one before
/// is still well ahead of the device (with PipeWire it often is: 674 such in a track that
/// played without a gap). It's late once the gap nears the whole buffer (`late_after`).
///
/// The buffer is measured: when the output starts, cpal fills it with callbacks in a row
/// until it's full, then the device's pace takes over. Those gaps, and the one to the first
/// paced callback, are the start, not a gap in the sound: nothing played before it. (Through
/// PulseAudio's null sink that one took ~0.9 s; the first-sound line in the log shows it.)
struct Alsa {
    channels: u64,
    rate: u32,
    /// Frames written since the output started, while it fills its buffer.
    filling: Option<u64>,
    /// The most frames a start wrote before the device took over: its buffer.
    buffer: u64,
}

impl Alsa {
    fn gap(&mut self, gap: Duration, samples: usize, g: &Glitches) {
        let period = samples as u64 / self.channels;
        let period_time = Duration::from_secs_f64(period as f64 / f64::from(self.rate));
        if let Some(filled) = self.filling {
            if gap < period_time / 2 {
                self.filling = Some(filled + period);
            } else {
                // The first callback the device paced: the start is over.
                self.buffer = self.buffer.max(filled);
                self.filling = None;
            }
            return;
        }
        let us = u32::try_from(gap.as_micros()).unwrap_or(u32::MAX);
        g.max_gap_us.fetch_max(us, Ordering::Relaxed);
        if gap > late_after(self.buffer, period, self.rate) {
            g.late.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// The longest gap between two ALSA callbacks of `period` frames, to a device that holds
/// `buffer` frames (two periods at least, as cpal asks), that still leaves the device a
/// quarter period to play while the callback fills and writes its period.
fn late_after(buffer: u64, period: u64, rate: u32) -> Duration {
    let buffer = buffer.max(2 * period);
    Duration::from_secs_f64((buffer as f64 - period as f64 / 4.0) / f64::from(rate.max(1)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gap_longer_than_the_buffer_is_late_unless_the_output_restarted() {
        let g = Glitches::default();
        let mut clock = CallbackClock::new(Duration::from_millis(5));
        clock.tick(&g, 0);
        std::thread::sleep(Duration::from_millis(20));
        clock.tick(&g, 0);
        assert_eq!(g.snapshot().late, 1);
        g.restarted();
        std::thread::sleep(Duration::from_millis(20));
        clock.tick(&g, 0);
        assert_eq!(g.snapshot().late, 1);
    }

    #[test]
    fn alsa_is_late_near_the_end_of_its_buffer_not_after_a_period() {
        let ms = |d: Duration| (d.as_secs_f64() * 1000.0 * 10.0).round() / 10.0;
        // pipewire-alsa: two periods of 2048 frames at 48 kHz (85.3 ms).
        assert_eq!(ms(late_after(4096, 2048, 48_000)), 74.7);
        // The pulse plugin: periods of 1365 frames, the 4096 cpal asked for (three).
        assert_eq!(ms(late_after(4095, 1365, 48_000)), 78.2);
        // Never under two periods, whatever a start measured.
        assert_eq!(late_after(0, 2048, 48_000), late_after(4096, 2048, 48_000));
    }

    #[test]
    fn alsa_skips_its_start_and_counts_gaps_near_the_buffer() {
        let g = Glitches::default();
        let mut clock = CallbackClock::alsa(2, 48_000);
        let period = 2 * 1365;
        let t0 = Instant::now();
        let mut at = |ms: u64| clock.tick_at(t0 + Duration::from_millis(ms), &g, period);
        // The start through the pulse plugin: three periods in a row, then the server held
        // them for 870 ms.
        at(0);
        at(0);
        at(1);
        at(871);
        // Then its pace: 20 and 40 ms (over a 28 ms period, under the old rule late), then
        // a gap past the 85 ms buffer.
        at(891);
        at(931);
        assert_eq!(g.snapshot().late, 0);
        assert_eq!(g.snapshot().max_gap, Duration::from_millis(40));
        at(1021);
        assert_eq!(g.snapshot().late, 1);
    }
}
