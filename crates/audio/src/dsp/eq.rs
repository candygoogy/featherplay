//! The five-band equalizer: a 60 Hz low shelf, peaks at 250 Hz, 1 kHz and 4 kHz (Q 1), and
//! a 12 kHz high shelf, each −12..+12 dB, behind an automatic preamp of minus the largest
//! boost, so a boost can't clip. Bands at 0 dB are skipped.

use super::biquad::{self, Coeffs};

/// Each band's limit, in dB either way.
pub(crate) const MAX_DB: f32 = 12.0;
const FREQS: [f64; 5] = [60.0, 250.0, 1000.0, 4000.0, 12_000.0];
const Q: f64 = 1.0;

pub(crate) struct Equalizer {
    rate: u32,
    coeffs: [Coeffs; 5],
    /// The bands that aren't at 0 dB, in order; `order[..active]`.
    order: [usize; 5],
    active: usize,
    /// The bands as set, in dB, to tell which changed.
    bands: [f32; 5],
    preamp: f64,
    /// Per channel, per band.
    state: Vec<[[f64; 2]; 5]>,
}

impl Equalizer {
    /// Allocates the state for `channels`; call it off the audio thread.
    pub fn new(channels: usize, rate: u32) -> Self {
        let flat = Coeffs::peaking(rate, FREQS[0], Q, 0.0);
        Equalizer {
            rate,
            coeffs: [flat; 5],
            order: [0; 5],
            active: 0,
            bands: [0.0; 5],
            preamp: 1.0,
            state: vec![[[0.0; 2]; 5]; channels.max(1)],
        }
    }

    /// Some band isn't at 0 dB.
    pub fn is_on(&self) -> bool {
        self.active > 0
    }

    /// Sets the bands, in dB (`None` is flat), clamped to ±`MAX_DB`. A band that comes on
    /// starts from silence; the others keep their state, so a change doesn't click.
    pub fn set(&mut self, bands: Option<[f32; 5]>) {
        let bands = bands.unwrap_or_default().map(|b| {
            if b.is_finite() {
                b.clamp(-MAX_DB, MAX_DB)
            } else {
                0.0
            }
        });
        self.active = 0;
        for (k, &db) in bands.iter().enumerate() {
            if db == 0.0 {
                continue;
            }
            if self.bands[k] == 0.0 {
                for ch in &mut self.state {
                    ch[k] = [0.0; 2];
                }
            }
            let g = f64::from(db);
            self.coeffs[k] = match k {
                0 => Coeffs::low_shelf(self.rate, FREQS[k], g),
                4 => Coeffs::high_shelf(self.rate, FREQS[k], g),
                _ => Coeffs::peaking(self.rate, FREQS[k], Q, g),
            };
            self.order[self.active] = k;
            self.active += 1;
        }
        self.bands = bands;
        let boost = bands.iter().fold(0f32, |m, &b| m.max(b));
        self.preamp = 10f64.powf(-f64::from(boost) / 20.0);
    }

    /// Filters `buf`, interleaved with as many channels as `new` was given.
    pub fn process(&mut self, buf: &mut [f32]) {
        if self.active == 0 {
            return;
        }
        let order = &self.order[..self.active];
        let channels = self.state.len();
        for frame in buf.chunks_exact_mut(channels) {
            for (s, st) in frame.iter_mut().zip(self.state.iter_mut()) {
                let mut x = f64::from(*s) * self.preamp;
                for &k in order {
                    x = self.coeffs[k].run(&mut st[k], x);
                }
                *s = x as f32;
            }
        }
        for st in &mut self.state {
            for band in st {
                biquad::flush(band);
            }
        }
    }

    /// Takes `other`'s settings and state, without allocating. Both were made for the same
    /// output.
    pub fn copy_from(&mut self, other: &Equalizer) {
        self.rate = other.rate;
        self.coeffs = other.coeffs;
        self.order = other.order;
        self.active = other.active;
        self.bands = other.bands;
        self.preamp = other.preamp;
        self.state.copy_from_slice(&other.state);
    }

    #[cfg(test)]
    pub fn state(&self) -> impl Iterator<Item = f64> + '_ {
        self.state.iter().flatten().flatten().copied()
    }
}
