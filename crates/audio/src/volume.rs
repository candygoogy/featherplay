/// Maps the 0..=1 volume slider to an amplitude gain on a cubic curve, which tracks
/// perceived loudness far better than a linear gain. Out-of-range input is clamped; NaN is silent.
pub fn perceptual_gain(slider: f32) -> f32 {
    if slider.is_nan() {
        return 0.0;
    }
    slider.clamp(0.0, 1.0).powi(3)
}

/// "Match song volumes": the amplitude gain for a song YouTube measured `loudness_db` dB
/// above its reference level (the player data's `loudnessDb`). Louder songs are turned down
/// by that much and quieter ones play as they are, as YouTube's own player does (Stats for
/// Nerds: "content loudness 5.3 dB", "Normalized 54%"). No value (or NaN) is 1.
pub fn loudness_gain(loudness_db: Option<f32>) -> f32 {
    match loudness_db {
        Some(db) if db > 0.0 => 10f32.powf(-db / 20.0),
        _ => 1.0,
    }
}
