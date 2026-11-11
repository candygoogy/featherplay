//! The tray icon, rasterised at build time.

use std::io::Read;

use flate2::read::DeflateDecoder;

/// The sizes `build.rs` renders the tray icon at, in order.
const TRAY_SIZES: [u32; 7] = [16, 20, 24, 32, 40, 48, 64];
/// The colour Tri-vane, for a dark taskbar and (the play on a crimson ring) a light one:
/// every size's RGBA in turn, deflated by `build.rs` (41 KB each raw, mostly transparent).
const TRAY_DARK: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tray-dark.rgba.deflate"));
const TRAY_LIGHT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/tray-light.rgba.deflate"));

/// The tray icon for a `size`-pixel slot: the smallest rendering at least that big (the
/// largest above 64 px), as straight-alpha RGBA, and its size.
pub fn tray_icon(size: u32, light_taskbar: bool) -> (Vec<u8>, u32) {
    let mut all = Vec::new();
    DeflateDecoder::new(if light_taskbar { TRAY_LIGHT } else { TRAY_DARK })
        .read_to_end(&mut all)
        .expect("build.rs deflates the tray icons");
    let pick = TRAY_SIZES
        .iter()
        .position(|&s| s >= size)
        .unwrap_or(TRAY_SIZES.len() - 1);
    let start: usize = TRAY_SIZES[..pick]
        .iter()
        .map(|s| (s * s * 4) as usize)
        .sum();
    let px = TRAY_SIZES[pick];
    let len = (px * px * 4) as usize;
    (all[start..start + len].to_vec(), px)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_icon_picks_a_size_that_covers_the_slot() {
        for (slot, want) in [(16, 16), (17, 20), (24, 24), (30, 32), (64, 64), (96, 64)] {
            for light in [false, true] {
                let (rgba, px) = tray_icon(slot, light);
                assert_eq!(px, want);
                assert_eq!(rgba.len(), (px * px * 4) as usize);
            }
        }
    }

    #[test]
    fn tray_icon_is_the_colour_tri_vane_for_each_taskbar() {
        let red = |p: &[u8]| p[3] == 255 && p[0] > 180 && p[1] < 110 && p[2] < 150;
        let white = |p: &[u8]| p[3] == 255 && p[..3] == [0xFF, 0xFF, 0xFF];
        for light in [false, true] {
            let (rgba, _) = tray_icon(16, light);
            let reds = rgba.chunks(4).filter(|p| red(p)).count();
            let whites = rgba.chunks(4).filter(|p| white(p)).count();
            assert!(reds > 30, "the red vanes (light: {light}): {reds}");
            assert!(whites > 4, "the white play (light: {light}): {whites}");
            // Transparent corners: no square plate behind the mark.
            assert_eq!(rgba[3], 0);
        }
        // The light taskbar's drawing is its own (the play on a crimson ring).
        assert_ne!(tray_icon(32, false).0, tray_icon(32, true).0);
    }
}
