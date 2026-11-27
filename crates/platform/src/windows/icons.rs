//! Small icons drawn at runtime: the thumb button glyphs and the tray icon. Shapes are
//! given in a unit square and rasterised with 4×4 supersampling into straight-alpha RGBA.

use crate::common::tray::drawings;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Glyph {
    Previous,
    Play,
    Pause,
    Next,
}

type Rgb = [u8; 3];
type Shape = fn(f32, f32) -> bool;

/// A thumb button glyph, `size` pixels square, in `color`.
pub fn glyph(g: Glyph, size: u32, color: Rgb) -> Vec<u8> {
    let shape: Shape = match g {
        Glyph::Play => |x, y| triangle(x, y, (0.30, 0.18), (0.30, 0.82), (0.82, 0.50)),
        Glyph::Pause => {
            |x, y| rect(x, y, 0.24, 0.18, 0.42, 0.82) || rect(x, y, 0.58, 0.18, 0.76, 0.82)
        }
        Glyph::Next => |x, y| {
            triangle(x, y, (0.18, 0.20), (0.18, 0.80), (0.64, 0.50))
                || rect(x, y, 0.66, 0.20, 0.80, 0.80)
        },
        Glyph::Previous => |x, y| {
            triangle(x, y, (0.82, 0.20), (0.82, 0.80), (0.36, 0.50))
                || rect(x, y, 0.20, 0.20, 0.34, 0.80)
        },
    };
    paint(size, shape, color)
}

/// The tray icon for a `size`-pixel slot: the smallest rendering at least that big (the
/// largest above 64 px), as straight-alpha RGBA, and its size.
pub fn tray_icon(size: u32, light_taskbar: bool) -> (Vec<u8>, u32) {
    let mut all = drawings(light_taskbar);
    let pick = all
        .iter()
        .position(|&(s, _)| s >= size)
        .unwrap_or(all.len() - 1);
    let (px, rgba) = all.swap_remove(pick);
    (rgba, px)
}

/// `shape` in `color`, as a `size`×`size` straight-alpha RGBA buffer: the colour wherever
/// the shape covers a sample, and the share of the pixel's samples it covers as the alpha.
fn paint(size: u32, shape: Shape, color: Rgb) -> Vec<u8> {
    const SS: u32 = 4;
    let n = size as usize;
    let mut out = vec![0u8; n * n * 4];
    let at = |p: u32, s: u32| (p as f32 + (s as f32 + 0.5) / SS as f32) / size as f32;
    for (i, px) in out.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (x, y) = ((i % n) as u32, (i / n) as u32);
        let hits = (0..SS * SS)
            .filter(|k| shape(at(x, k % SS), at(y, k / SS)))
            .count();
        if hits > 0 {
            let alpha = hits as f32 / (SS * SS) as f32;
            *px = [color[0], color[1], color[2], (alpha * 255.0).round() as u8];
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
