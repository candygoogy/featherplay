//! Counters for audio that came out late: output callbacks that came after the device had
//! played its whole buffer (an audible gap), and rings that ran dry while a track played.
//! Atomics only, so the audio thread can count without locking or allocating. The engine
//! logs them when playback pauses or a track ends, never on a timer.

use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

#[derive(Default)]
pub(crate) struct Glitches {
    /// Callbacks that came more than a device buffer after the one before.
    late: AtomicU32,
    /// The longest gap between two callbacks, in microseconds.
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
    /// How long the device plays from one full buffer.
    buffer: Duration,
    last: Option<(Instant, u32)>,
}

impl CallbackClock {
    pub fn new(buffer: Duration) -> Self {
        Self { buffer, last: None }
    }

    /// One callback, now.
    pub fn tick(&mut self, g: &Glitches) {
        let now = Instant::now();
        let epoch = g.epoch.load(Ordering::Relaxed);
        if let Some((last, e)) = self.last
            && e == epoch
        {
            let gap = now - last;
            let us = u32::try_from(gap.as_micros()).unwrap_or(u32::MAX);
            g.max_gap_us.fetch_max(us, Ordering::Relaxed);
            if gap > self.buffer {
                g.late.fetch_add(1, Ordering::Relaxed);
            }
        }
        self.last = Some((now, epoch));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_gap_longer_than_the_buffer_is_late_unless_the_output_restarted() {
        let g = Glitches::default();
        let mut clock = CallbackClock::new(Duration::from_millis(5));
        clock.tick(&g);
        std::thread::sleep(Duration::from_millis(20));
        clock.tick(&g);
        assert_eq!(g.snapshot().late, 1);
        g.restarted();
        std::thread::sleep(Duration::from_millis(20));
        clock.tick(&g);
        assert_eq!(g.snapshot().late, 1);
    }
}
