/// Maps the 0..=1 volume slider to an amplitude gain on a cubic curve, which tracks
/// perceived loudness far better than a linear gain. Out-of-range input is clamped; NaN is silent.
pub fn perceptual_gain(slider: f32) -> f32 {
    if slider.is_nan() {
        return 0.0;
    }
    slider.clamp(0.0, 1.0).powi(3)
}
