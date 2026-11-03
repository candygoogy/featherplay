use featherplay_audio::perceptual_gain;

#[test]
fn gain_curve() {
    assert_eq!(perceptual_gain(0.5), 0.125);
    assert_eq!(perceptual_gain(1.2), 1.0);
    assert_eq!(perceptual_gain(-1.0), 0.0);
    assert_eq!(perceptual_gain(0.0), 0.0);
    assert_eq!(perceptual_gain(1.0), 1.0);
}
