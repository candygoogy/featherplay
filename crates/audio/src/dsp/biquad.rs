//! RBJ Audio EQ Cookbook biquads (Robert Bristow-Johnson), in transposed direct form II.
//! Coefficients and state are `f64`: a 60 Hz shelf at 48 kHz has its poles close to the unit
//! circle, where `f32` coefficients colour the sound, and an `f64` state decaying on silence
//! takes far longer than a callback to reach the subnormal range, so flushing it once per
//! block is enough.

use std::f64::consts::PI;

/// State below this is flushed to zero once per block: on silence an IIR state decays
/// towards the subnormal range, which is very slow on x64 (and not flushed by ARM64's
/// default mode either).
const FLUSH: f64 = 1e-15;

/// One second-order section, normalised (`a0` = 1).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Coeffs {
    b0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
}

/// Shelf slope S = 1, the steepest without a bump.
fn shelf_alpha(sin: f64) -> f64 {
    sin / 2.0 * 2f64.sqrt()
}

/// `f0` kept below Nyquist, for a device running at a low rate.
fn omega(rate: u32, f0: f64) -> f64 {
    let rate = f64::from(rate.max(1));
    2.0 * PI * f0.min(0.45 * rate) / rate
}

impl Coeffs {
    fn normalised(b: [f64; 3], a: [f64; 3]) -> Self {
        Coeffs {
            b0: b[0] / a[0],
            b1: b[1] / a[0],
            b2: b[2] / a[0],
            a1: a[1] / a[0],
            a2: a[2] / a[0],
        }
    }

    /// A peaking band of `gain_db` at `f0`, `q` wide.
    pub fn peaking(rate: u32, f0: f64, q: f64, gain_db: f64) -> Self {
        let a = 10f64.powf(gain_db / 40.0);
        let w = omega(rate, f0);
        let (sin, cos) = w.sin_cos();
        let alpha = sin / (2.0 * q);
        Self::normalised(
            [1.0 + alpha * a, -2.0 * cos, 1.0 - alpha * a],
            [1.0 + alpha / a, -2.0 * cos, 1.0 - alpha / a],
        )
    }

    /// A low shelf of `gain_db` below `f0`.
    pub fn low_shelf(rate: u32, f0: f64, gain_db: f64) -> Self {
        let a = 10f64.powf(gain_db / 40.0);
        let (sin, cos) = omega(rate, f0).sin_cos();
        let k = 2.0 * a.sqrt() * shelf_alpha(sin);
        Self::normalised(
            [
                a * ((a + 1.0) - (a - 1.0) * cos + k),
                2.0 * a * ((a - 1.0) - (a + 1.0) * cos),
                a * ((a + 1.0) - (a - 1.0) * cos - k),
            ],
            [
                (a + 1.0) + (a - 1.0) * cos + k,
                -2.0 * ((a - 1.0) + (a + 1.0) * cos),
                (a + 1.0) + (a - 1.0) * cos - k,
            ],
        )
    }

    /// A high shelf of `gain_db` above `f0`.
    pub fn high_shelf(rate: u32, f0: f64, gain_db: f64) -> Self {
        let a = 10f64.powf(gain_db / 40.0);
        let (sin, cos) = omega(rate, f0).sin_cos();
        let k = 2.0 * a.sqrt() * shelf_alpha(sin);
        Self::normalised(
            [
                a * ((a + 1.0) + (a - 1.0) * cos + k),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * cos),
                a * ((a + 1.0) + (a - 1.0) * cos - k),
            ],
            [
                (a + 1.0) - (a - 1.0) * cos + k,
                2.0 * ((a - 1.0) - (a + 1.0) * cos),
                (a + 1.0) - (a - 1.0) * cos - k,
            ],
        )
    }

    /// One sample through the section with state `s`.
    #[inline]
    pub fn run(&self, s: &mut [f64; 2], x: f64) -> f64 {
        let y = self.b0 * x + s[0];
        s[0] = self.b1 * x - self.a1 * y + s[1];
        s[1] = self.b2 * x - self.a2 * y;
        y
    }
}

/// Zeroes the parts of a section's state too small to hear (see `FLUSH`), and any that
/// aren't numbers, which would otherwise stay in the state and silence the band for good.
pub(crate) fn flush(s: &mut [f64; 2]) {
    for v in s {
        if !v.is_finite() || v.abs() < FLUSH {
            *v = 0.0;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RATE: u32 = 48_000;

    /// The level change, in dB, of a sine at `hz` through `c`, measured after it settles.
    fn gain_db(c: &Coeffs, hz: f64) -> f64 {
        let mut s = [0.0; 2];
        let n = RATE as usize;
        let (mut inp, mut out) = (0.0, 0.0);
        for i in 0..n {
            let x = (2.0 * PI * hz * i as f64 / f64::from(RATE)).sin();
            let y = c.run(&mut s, x);
            // The second half, once the filter has settled.
            if i >= n / 2 {
                inp += x * x;
                out += y * y;
            }
        }
        10.0 * (out / inp).log10()
    }

    #[test]
    fn a_6_db_peak_at_1_khz_raises_1_khz_by_6_db_and_leaves_100_hz() {
        let c = Coeffs::peaking(RATE, 1000.0, 1.0, 6.0);
        let at = gain_db(&c, 1000.0);
        let low = gain_db(&c, 100.0);
        assert!((at - 6.0).abs() <= 0.5, "1 kHz: {at:.2} dB");
        assert!(low.abs() < 0.5, "100 Hz: {low:.2} dB");
    }

    #[test]
    fn shelves_boost_their_side_only() {
        let low = Coeffs::low_shelf(RATE, 60.0, 8.0);
        let high = Coeffs::high_shelf(RATE, 12_000.0, 6.0);
        assert!((gain_db(&low, 20.0) - 8.0).abs() < 0.5);
        assert!(gain_db(&low, 2000.0).abs() < 0.2);
        assert!((gain_db(&high, 20_000.0) - 6.0).abs() < 0.7);
        assert!(gain_db(&high, 200.0).abs() < 0.2);
    }
}
