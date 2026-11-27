//! The output's mixer: the voices' sources summed for the master bus, a block at a time.
//!
//! It mixes as rodio's mixer did, sample for sample: each sample is `0.0` plus each
//! source's, in the order the sources came, and a source leaves at its first missing sample.
//! A source in the output's own format is added from its chunks directly. One in another
//! format plays through rodio's converter (`UniformSourceIterator`) as before, a sample at a
//! time. rodio's mixer took every sample through that converter, a dynamic call and a check
//! for new sources, even where the converter had nothing to do: with the bus doing nothing
//! either, one voice at 44.1 kHz stereo cost the audio thread ~0.78 ms a second that way,
//! and ~0.034 ms this way (opt-level 3, 2026-11-27).

use std::sync::mpsc::{Receiver, Sender, channel};

use rodio::source::UniformSourceIterator;
use rodio::{ChannelCount, Sample, SampleRate, Source};

use crate::voice::RingSource;

/// Sources mixed at once without the list growing: the current voice, two fading, and a
/// retired one or two.
const ROOM: usize = 16;

enum Input {
    /// In the output's format.
    Direct(Box<RingSource>),
    /// Converted to the output's channels and rate.
    Converted(Box<UniformSourceIterator<RingSource>>),
}

impl Input {
    /// Adds the source's next `out.len()` samples onto `out`; false once it has ended.
    fn add_to(&mut self, out: &mut [Sample]) -> bool {
        match self {
            Input::Direct(s) => s.add_to(out),
            Input::Converted(c) => {
                for o in out {
                    match c.next() {
                        Some(s) => *o += s,
                        None => return false,
                    }
                }
                true
            }
        }
    }
}

/// Where the engine adds sources, from any thread.
pub(crate) struct Mixer {
    tx: Sender<Input>,
    channels: ChannelCount,
    rate: SampleRate,
}

impl Mixer {
    /// Mixes `src` in from the start of the output's next block. Off the audio thread: the
    /// source is boxed here, as rodio's mixer boxed it.
    pub fn add(&self, src: RingSource) {
        let input = if src.channels() == self.channels && src.sample_rate() == self.rate {
            Input::Direct(Box::new(src))
        } else {
            Input::Converted(Box::new(UniformSourceIterator::new(
                src,
                self.channels,
                self.rate,
            )))
        };
        // Fails only once the output is gone, and the source with it.
        let _ = self.tx.send(input);
    }
}

/// The audio thread's side.
pub(crate) struct MixerSource {
    pending: Receiver<Input>,
    sources: Vec<Input>,
}

impl MixerSource {
    /// Fills `out` (whole frames) with the mix: 0 where no source plays.
    pub fn fill(&mut self, out: &mut [Sample]) {
        self.sources.extend(self.pending.try_iter());
        out.fill(0.0);
        self.sources.retain_mut(|s| s.add_to(out));
    }
}

/// A mixer for an output of `channels` at `rate`.
pub(crate) fn mixer(channels: ChannelCount, rate: SampleRate) -> (Mixer, MixerSource) {
    let (tx, pending) = channel();
    (
        Mixer { tx, channels, rate },
        MixerSource {
            pending,
            sources: Vec::with_capacity(ROOM),
        },
    )
}
