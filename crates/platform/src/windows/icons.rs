//! Small icons drawn at runtime: the thumb button glyphs. Shapes are given in a unit square
//! and rasterised with 4×4 supersampling into straight-alpha RGBA.

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
