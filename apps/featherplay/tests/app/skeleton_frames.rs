//! The loading skeleton's sweep repaints only where its band is, at 4K 150 %, playing: Home
//! loading, its shelves' placeholder cards, past the skeleton's delay and fade.
//!
//! Every placeholder read the band's position, which moves every frame, so every one was
//! marked dirty in every frame though only those under the band change: the whole page
//! repainted (30 % of a 4K window on Home, 3.8 ms a frame; 7–11 ms over Now playing) for as
//! long as a load lasts (Explore's first visit, each song's lyrics lookup; measured
//! 2026-11-01). Now 6 % and ~1.9 ms. The ceiling holds in optimised builds.

use crate::animations::{scene, step};
use crate::support::quantile;
use featherplay::ui::HomeData;
use slint::ComponentHandle;

/// The most a frame of the sweep may cost, ms of CPU (optimised), at the median.
const SWEEP_MEDIAN_MS: f64 = 4.0;

#[test]
fn the_skeleton_sweep_repaints_only_under_its_band() {
    let scene = scene();
    let home = scene.ui.global::<HomeData>();
    home.set_error("".into());
    home.set_shelves(Default::default());
    home.set_loading(true);
    // Past the delay (150 ms) and the fade in.
    for _ in 0..120 {
        step(&scene);
    }
    let window = 3840.0 * 2160.0;
    let mut last = String::new();
    for _ in 0..3 {
        // Most of a sweep: 2.2 s a period, the band moving for three quarters of it.
        let frames: Vec<_> = (0..400).map(|_| step(&scene)).collect();
        assert!(home.get_loading(), "Home stopped loading");
        let moving: Vec<_> = frames.iter().filter(|f| f.damaged > 0).collect();
        assert!(
            moving.len() > 100,
            "the band didn't move ({} frames)",
            moving.len()
        );
        let damage: Vec<f64> = moving
            .iter()
            .map(|f| f.damaged as f64 / window * 100.0)
            .collect();
        let cpu: Vec<f64> = moving.iter().map(|f| f.cpu).collect();
        let (dmg, ms) = (quantile(&damage, 0.5), quantile(&cpu, 0.5));
        println!(
            "the skeleton's sweep at 4K 150 %: {dmg:.1} % of the window and {ms:.2} ms a frame (median)"
        );
        assert!(
            dmg < 30.0,
            "the sweep repainted {dmg:.1} % of the window a frame"
        );
        if cfg!(debug_assertions) || ms <= SWEEP_MEDIAN_MS {
            return;
        }
        last =
            format!("a frame of the sweep cost {ms:.2} ms at the median, over {SWEEP_MEDIAN_MS}");
    }
    panic!("{last}");
}
