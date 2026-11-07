//! Small icons drawn at runtime: the thumb button glyphs and the tray icon. Shapes are
//! given in a unit square and rasterised with 4×4 supersampling into straight-alpha RGBA.

use std::io::Read;

use flate2::read::DeflateDecoder;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Previous,
    Play,
    Pause,
    Next,
}

type Rgb = [u8; 3];
type Shape = dyn Fn(f32, f32) -> bool;

/// A thumb button glyph, `size` pixels square, in `color`.
pub fn glyph(g: Glyph, size: u32, color: Rgb) -> Vec<u8> {
    let shape: Box<Shape> = match g {
        Glyph::Play => Box::new(|x, y| triangle(x, y, (0.30, 0.18), (0.30, 0.82), (0.82, 0.50))),
        Glyph::Pause => Box::new(|x, y| {
            rect(x, y, 0.24, 0.18, 0.42, 0.82) || rect(x, y, 0.58, 0.18, 0.76, 0.82)
        }),
        Glyph::Next => Box::new(|x, y| {
            triangle(x, y, (0.18, 0.20), (0.18, 0.80), (0.64, 0.50))
                || rect(x, y, 0.66, 0.20, 0.80, 0.80)
        }),
        Glyph::Previous => Box::new(|x, y| {
            triangle(x, y, (0.82, 0.20), (0.82, 0.80), (0.36, 0.50))
                || rect(x, y, 0.20, 0.20, 0.34, 0.80)
        }),
    };
    paint(size, &[(&*shape, color)])
}

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

/// Composites the layers in order ("over") into a `size`×`size` RGBA buffer.
fn paint(size: u32, layers: &[(&Shape, Rgb)]) -> Vec<u8> {
    const SS: u32 = 4;
    let n = size as usize;
    let mut out = vec![0u8; n * n * 4];
    for py in 0..size {
        for px in 0..size {
            // Straight-alpha accumulation in floats.
            let (mut r, mut g, mut b, mut a) = (0f32, 0f32, 0f32, 0f32);
            for (shape, color) in layers {
                let mut hits = 0;
                for sy in 0..SS {
                    for sx in 0..SS {
                        let x = (px as f32 + (sx as f32 + 0.5) / SS as f32) / size as f32;
                        let y = (py as f32 + (sy as f32 + 0.5) / SS as f32) / size as f32;
                        if shape(x, y) {
                            hits += 1;
                        }
                    }
                }
                let ca = hits as f32 / (SS * SS) as f32;
                if ca == 0.0 {
                    continue;
                }
                let out_a = ca + a * (1.0 - ca);
                let mix =
                    |c: u8, under: f32| (c as f32 / 255.0 * ca + under * a * (1.0 - ca)) / out_a;
                r = mix(color[0], r);
                g = mix(color[1], g);
                b = mix(color[2], b);
                a = out_a;
            }
            let i = (py as usize * n + px as usize) * 4;
            out[i] = (r * 255.0).round() as u8;
            out[i + 1] = (g * 255.0).round() as u8;
            out[i + 2] = (b * 255.0).round() as u8;
            out[i + 3] = (a * 255.0).round() as u8;
        }
    }
    out
}

fn rect(x: f32, y: f32, x0: f32, y0: f32, x1: f32, y1: f32) -> bool {
    (x0..x1).contains(&x) && (y0..y1).contains(&y)
}

fn triangle(x: f32, y: f32, a: (f32, f32), b: (f32, f32), c: (f32, f32)) -> bool {
    let edge = |p: (f32, f32), q: (f32, f32)| (q.0 - p.0) * (y - p.1) - (q.1 - p.1) * (x - p.0);
    let (e0, e1, e2) = (edge(a, b), edge(b, c), edge(c, a));
    (e0 >= 0.0 && e1 >= 0.0 && e2 >= 0.0) || (e0 <= 0.0 && e1 <= 0.0 && e2 <= 0.0)
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
