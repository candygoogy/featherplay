//! The master bus: what every output callback does to the mix before converting it to the
//! device's format. In order: the equalizer, the compressor ("Even out loudness"), the
//! volume, and a safety limiter while either of the first two is on. One state serves every
//! voice, so a later crossfade gets one equalizer and one compressor, as a master bus should.
//!
//! The settings reach the audio thread as atomics in `BusControls`, with a generation
//! counter: the bus looks at it once per callback and recomputes its coefficients only when
//! it moved. Nothing on the audio thread locks, allocates or does I/O. With everything off
//! and the volume at 1.0 the bus is a plain copy. The bus only runs while the output stream
//! does, so it costs nothing while paused.

mod biquad;
mod compressor;
#[cfg(test)]
mod cost;
mod eq;
mod limiter;

use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};

use rodio::cpal::FromSample;

pub use compressor::Compressor;

/// Frames the bus works on at once; a longer callback is done in several steps.
const STEP_FRAMES: usize = 1024;

/// The bus's settings, written by the engine and read by the output callback.
pub(crate) struct BusControls {
    /// The volume as an amplitude gain (`perceptual_gain` of the slider), as `f32` bits.
    gain: AtomicU32,
    eq_on: AtomicBool,
    bands: [AtomicU32; 5],
    compressor: AtomicU8,
    /// Bumped after each change of the equalizer or the compressor.
    generation: AtomicU32,
}

impl Default for BusControls {
    fn default() -> Self {
        BusControls {
            gain: AtomicU32::new(1.0f32.to_bits()),
            eq_on: AtomicBool::new(false),
            bands: Default::default(),
            compressor: AtomicU8::new(0),
            generation: AtomicU32::new(0),
        }
    }
}

impl BusControls {
    pub fn set_gain(&self, g: f32) {
        self.gain.store(g.to_bits(), Ordering::Relaxed);
    }

    fn gain(&self) -> f32 {
        f32::from_bits(self.gain.load(Ordering::Relaxed))
    }

    /// The equalizer's bands in dB, `None` for off.
    pub fn set_equalizer(&self, bands: Option<[f32; 5]>) {
        self.eq_on.store(bands.is_some(), Ordering::Relaxed);
        for (a, b) in self.bands.iter().zip(bands.unwrap_or_default()) {
            a.store(b.to_bits(), Ordering::Relaxed);
        }
        self.generation.fetch_add(1, Ordering::Release);
    }

    pub fn set_compressor(&self, c: Compressor) {
        let v = match c {
            Compressor::Off => 0,
            Compressor::Light => 1,
            Compressor::Strong => 2,
        };
        self.compressor.store(v, Ordering::Relaxed);
        self.generation.fetch_add(1, Ordering::Release);
    }

    /// The current settings and their generation. A change made while this reads bumps the
    /// generation again, so the next callback reads it whole.
    fn read(&self) -> (u32, Option<[f32; 5]>, Compressor) {
        let generation = self.generation.load(Ordering::Acquire);
        let bands = self.eq_on.load(Ordering::Relaxed).then(|| {
            self.bands
                .each_ref()
                .map(|b| f32::from_bits(b.load(Ordering::Relaxed)))
        });
        let compressor = match self.compressor.load(Ordering::Relaxed) {
            1 => Compressor::Light,
            2 => Compressor::Strong,
            _ => Compressor::Off,
        };
        (generation, bands, compressor)
    }
}

/// The DSP chain, without the controls: pure processing on interleaved `f32` blocks.
pub(crate) struct Chain {
    channels: usize,
    eq: eq::Equalizer,
    compressor: compressor::State,
    limiter: limiter::Limiter,
}

impl Chain {
    /// Allocates everything for `channels` at `rate`; call it off the audio thread.
    pub fn new(channels: usize, rate: u32) -> Self {
        let channels = channels.max(1);
        Chain {
            channels,
            eq: eq::Equalizer::new(channels, rate),
            compressor: compressor::State::new(channels, rate),
            limiter: limiter::Limiter::new(rate),
        }
    }

    fn shaped(&self) -> bool {
        self.eq.is_on() || self.compressor.is_on()
    }

    pub fn set(&mut self, bands: Option<[f32; 5]>, c: Compressor) {
        let was = self.shaped();
        self.eq.set(bands);
        self.compressor.set(c);
        if !was {
            self.limiter.reset();
        }
    }

    /// Takes `other`'s settings and state, without allocating. Both were made for the same
    /// output.
    fn copy_from(&mut self, other: &Chain) {
        self.eq.copy_from(&other.eq);
        self.compressor.copy_from(&other.compressor);
        self.limiter = other.limiter;
    }

    /// Runs `buf` (whole frames) through the chain at volume `gain`. Not inlined: one copy
    /// of the chain in the exe, not one per caller.
    #[inline(never)]
    pub fn process(&mut self, buf: &mut [f32], gain: f32) {
        let shaped = self.shaped();
        if shaped {
            self.eq.process(buf);
            self.compressor.process(buf, self.channels);
        }
        if gain != 1.0 {
            for s in buf.iter_mut() {
                *s *= gain;
            }
        }
        if shaped {
            self.limiter.process(buf, self.channels);
        }
    }
}

/// The bus of one output: the chain, fed from its controls, with room for one step of the
/// mix in `f32`.
///
/// A change of the equalizer or the compressor mid-song would jump the output (the preamp,
/// a band coming in or out, the compressor's makeup), so for `FADE_FRAMES` after one the bus
/// runs the chain as it was beside the new one and crossfades from the first to the second.
pub(crate) struct Bus {
    controls: std::sync::Arc<crate::voice::Controls>,
    channels: usize,
    chain: Chain,
    /// The chain as it was before the latest change, while the crossfade runs.
    old: Chain,
    /// Frames of the crossfade still to go.
    fading: usize,
    seen: u32,
    scratch: Vec<f32>,
    old_scratch: Vec<f32>,
}

/// The crossfade after a change: about 5 ms.
const FADE_FRAMES: usize = 256;

impl Bus {
    /// Allocates the bus for an output of `channels` at `rate`; call it off the audio thread.
    pub fn new(controls: std::sync::Arc<crate::voice::Controls>, channels: u16, rate: u32) -> Self {
        let channels = usize::from(channels.max(1));
        let mut chain = Chain::new(channels, rate);
        let (seen, bands, c) = controls.bus.read();
        chain.set(bands, c);
        Bus {
            controls,
            channels,
            chain,
            old: Chain::new(channels, rate),
            fading: 0,
            seen,
            scratch: vec![0.0; STEP_FRAMES * channels],
            old_scratch: vec![0.0; STEP_FRAMES * channels],
        }
    }

    /// Fills `out` (whole frames) from `source` through the chain. On the audio thread.
    pub fn fill<T: FromSample<f32>>(
        &mut self,
        source: &mut impl Iterator<Item = f32>,
        out: &mut [T],
    ) {
        debug_assert!(
            out.len().is_multiple_of(self.channels),
            "a callback of whole frames"
        );
        self.update();
        let gain = self.controls.bus.gain();
        for chunk in out.chunks_mut(self.scratch.len()) {
            let buf = &mut self.scratch[..chunk.len()];
            for s in buf.iter_mut() {
                // A sample that isn't a number would stay in the filters' state.
                *s = source.next().filter(|s| s.is_finite()).unwrap_or(0.0);
            }
            self.step(chunk.len(), gain);
            for (d, &s) in chunk.iter_mut().zip(self.scratch.iter()) {
                *d = T::from_sample_(s);
            }
        }
    }

    /// Takes the controls' settings if they changed, and starts the crossfade to them.
    fn update(&mut self) {
        if self.controls.bus.generation.load(Ordering::Acquire) == self.seen {
            return;
        }
        let (seen, bands, c) = self.controls.bus.read();
        self.old.copy_from(&self.chain);
        self.chain.set(bands, c);
        self.seen = seen;
        self.fading = FADE_FRAMES;
    }

    /// Runs the first `len` samples of `scratch` through the chain, crossfading from the old
    /// one while a change is fading in. Not generic over the device's format, so the exe
    /// holds it once.
    #[inline(never)]
    fn step(&mut self, len: usize, gain: f32) {
        let buf = &mut self.scratch[..len];
        if self.fading == 0 {
            self.chain.process(buf, gain);
            return;
        }
        let old = &mut self.old_scratch[..len];
        old.copy_from_slice(buf);
        self.old.process(old, gain);
        self.chain.process(buf, gain);
        let done = FADE_FRAMES - self.fading;
        let frames = self.fading.min(len / self.channels);
        for (i, (new, old)) in buf
            .chunks_exact_mut(self.channels)
            .zip(old.chunks_exact(self.channels))
            .take(frames)
            .enumerate()
        {
            let w = (done + i + 1) as f32 / FADE_FRAMES as f32;
            for (n, o) in new.iter_mut().zip(old) {
                *n = o + (*n - o) * w;
            }
        }
        self.fading -= frames;
    }
}

#[cfg(test)]
mod tests {
    use std::num::NonZero;
    use std::sync::Arc;

    use rodio::buffer::SamplesBuffer;
    use rodio::mixer::mixer;

    use super::*;
    use crate::perceptual_gain;
    use crate::voice::Controls;

    const RATE: u32 = 48_000;

    /// `frames` of a stereo signal: a 440 Hz sine left, a 3 kHz one right, at `amp`.
    fn signal(frames: usize, amp: f32) -> Vec<f32> {
        let w =
            |hz: f32, i: usize| (2.0 * std::f32::consts::PI * hz * i as f32 / RATE as f32).sin();
        (0..frames)
            .flat_map(|i| [amp * w(440.0, i), amp * w(3000.0, i)])
            .collect()
    }

    #[test]
    fn a_flat_equalizer_and_no_compressor_leave_the_samples_bit_identical() {
        let mut chain = Chain::new(2, RATE);
        chain.set(Some([0.0; 5]), Compressor::Off);
        let input = signal(4800, 0.99);
        let mut buf = input.clone();
        chain.process(&mut buf, 1.0);
        assert!(
            buf.iter()
                .zip(&input)
                .all(|(a, b)| a.to_bits() == b.to_bits())
        );
    }

    #[test]
    fn a_long_silence_after_a_loud_burst_leaves_no_subnormal_in_the_filters() {
        let mut chain = Chain::new(2, RATE);
        chain.set(Some([12.0, -12.0, 6.0, -6.0, 12.0]), Compressor::Strong);
        let mut burst = signal(4800, 1.0);
        chain.process(&mut burst, 1.0);
        // A second of silence, in 10 ms callbacks.
        for _ in 0..100 {
            let mut quiet = vec![0f32; 960];
            chain.process(&mut quiet, 1.0);
        }
        let state: Vec<f64> = chain.eq.state().collect();
        assert!(state.iter().all(|&v| v == 0.0), "{state:?}");
    }

    #[test]
    fn the_volume_scales_the_mix_exactly_as_before() {
        // Before, each voice multiplied its samples by the gain, and the mixer summed them;
        // with one voice the output was `sample * perceptual_gain(slider)`.
        let controls = Arc::new(Controls::new());
        controls.bus.set_gain(perceptual_gain(0.6));
        let input = signal(3000, 0.8);
        let (m, mut source) = mixer(NonZero::new(2).unwrap(), NonZero::new(RATE).unwrap());
        m.add(SamplesBuffer::new(
            NonZero::new(2).unwrap(),
            NonZero::new(RATE).unwrap(),
            input.clone(),
        ));
        let mut bus = Bus::new(controls, 2, RATE);
        let mut out = vec![0f32; input.len()];
        bus.fill(&mut source, &mut out);
        let want: Vec<f32> = input.iter().map(|s| s * perceptual_gain(0.6)).collect();
        assert_eq!(out, want);
    }

    #[test]
    fn a_change_reaches_the_bus_on_its_next_callback() {
        let controls = Arc::new(Controls::new());
        let mut bus = Bus::new(controls.clone(), 2, RATE);
        let mut source = std::iter::repeat(0.5f32);
        let mut out = vec![0f32; 960];
        bus.fill(&mut source, &mut out);
        assert!(out.iter().all(|&s| s == 0.5));
        controls.bus.set_equalizer(Some([0.0, 0.0, 0.0, 0.0, 6.0]));
        bus.fill(&mut source, &mut out);
        // The preamp turns the level down by the boost (−6 dB), which the 12 kHz shelf
        // doesn't give back at DC; by the callback's end the crossfade is done.
        let last = out[out.len() - 1];
        assert!((last - 0.5 * 0.501).abs() < 0.01, "{last}");
    }

    #[test]
    fn a_change_mid_song_doesnt_jump() {
        let controls = Arc::new(Controls::new());
        let mut bus = Bus::new(controls.clone(), 2, RATE);
        let mut source = signal(RATE as usize, 0.9).into_iter();
        // Callbacks of 484 frames: the changes land at frames 484 and 1452, where the 3 kHz
        // sine (16 frames a period) is at a peak.
        let n = 2 * 484;
        let mut out = vec![0f32; 4 * n];
        bus.fill(&mut source, &mut out[..n]);
        controls.bus.set_equalizer(Some([12.0, 0.0, 0.0, 0.0, 0.0]));
        bus.fill(&mut source, &mut out[n..2 * n]);
        controls.bus.set_compressor(Compressor::Strong);
        bus.fill(&mut source, &mut out[2 * n..3 * n]);
        controls.bus.set_equalizer(None);
        controls.bus.set_compressor(Compressor::Off);
        bus.fill(&mut source, &mut out[3 * n..]);
        // The 3 kHz sine on the right moves at most 0.9 × 2π × 3000 / 48000 ≈ 0.35 per
        // sample; a jump of the level (the −12 dB preamp, the makeup) would add to that.
        let right: Vec<f32> = out.iter().skip(1).step_by(2).copied().collect();
        let step = right
            .windows(2)
            .fold(0f32, |m, w| m.max((w[1] - w[0]).abs()));
        assert!(step < 0.4, "a step of {step}");
    }
}
