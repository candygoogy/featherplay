use featherplay_audio::{loudness_gain, perceptual_gain};

#[test]
fn gain_curve() {
    assert_eq!(perceptual_gain(0.5), 0.125);
    assert_eq!(perceptual_gain(1.2), 1.0);
    assert_eq!(perceptual_gain(-1.0), 0.0);
    assert_eq!(perceptual_gain(0.0), 0.0);
    assert_eq!(perceptual_gain(1.0), 1.0);
}

#[test]
fn a_song_louder_than_the_reference_is_turned_down_by_its_loudness() {
    // Stats for Nerds shows 5.3 dB of content loudness as "Normalized 54%".
    assert!((loudness_gain(Some(5.3)) - 0.543).abs() < 1e-3);
    assert!((loudness_gain(Some(7.17)) - 0.438).abs() < 1e-3);
    assert!((loudness_gain(Some(20.0)) - 0.1).abs() < 1e-6);
}

#[test]
fn a_quieter_song_or_one_without_a_value_plays_unchanged() {
    assert_eq!(loudness_gain(Some(-4.5)), 1.0);
    assert_eq!(loudness_gain(Some(0.0)), 1.0);
    assert_eq!(loudness_gain(None), 1.0);
    assert_eq!(loudness_gain(Some(f32::NAN)), 1.0);
}
