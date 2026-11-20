//! Now playing's ground, the maths: one calm field of the song's cover per theme (`Field`),
//! and from it everything the page sits on.
//!
//! The field is the cover at `SMALL` pixels a side, blurred wide (a box blur run three times,
//! about a Gaussian of 15 % of the cover, so no shape survives), each part keeping its own hue
//! but toned into a narrow band of lightness with its chroma capped: dark (`tone`), or near
//! white. Made once per cover with its colour (`follow`), from the same small decode, off the
//! UI thread. From it:
//! - `Field::stops`: three colours down the page, drawn as solid bands without a picture (the
//!   cover view, and the lyrics with "Cover behind the lyrics" off);
//! - `Field::card`: the tint of Now playing's floating card and of the bar under it;
//! - `Field::ink`: the lyrics' three levels, each keeping its floor (`colour::LYRIC_*`) against
//!   the field's worst pixel, the bands' and the picture's alike, so nothing changes colour
//!   when the picture comes in;
//! - with "Cover behind the lyrics" (`bake`), the picture: the field taken to a `GRID` grid
//!   (bilinear, then smoothed), scaled up to the page in physical pixels with a light dither
//!   against banding, the cover kept square (its middle `KEEP` across the page's longer side),
//!   and the two strips the lines fade into at the top and bottom of the lyrics. Made once per
//!   song (and again for a new theme or a settled resize) on tokio's blocking pool
//!   (`pages::now_playing::backdrop`).
//!
//! Pure.

use std::ops::{Deref, Range};

use image::imageops::FilterType;
use image::{ImageBuffer, Rgba};

use super::colour::{self, Lch, LyricInk, Rgb};

/// The cover is shrunk to this many pixels a side, and blurred and toned there.
pub const SMALL: usize = 24;
/// The box blur's radius in the small picture, run three times: about a Gaussian of
/// sigma √(r² + r) ≈ 3.5 px, 15 % of the cover.
const RADIUS: usize = 3;
/// The picture's grid: the small picture taken up 4× and smoothed, so the scaling up
/// to the page shows no cells (24 across a 4K page are 160 px each, and bilinear's kinks show
/// as stripes on a field this smooth).
const GRID: usize = 96;
/// The share of the cover across the page's longer side: its middle, larger, never stretched.
const KEEP: f32 = 0.8;
/// The contrast an ink keeps above its floor, as a factor: the scaling up mixes neighbouring
/// pixels, and the dither and the rounding to 8 bits move each by up to a level.
const HEADROOM: f32 = 1.04;
/// The longest side made; past it the picture is drawn larger than it is.
pub const MAX_SIDE: u32 = 3840;

/// One theme's field of a cover, and what the page takes from it (all 8 bits a channel but
/// `small`).
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    /// The cover, `SMALL` × `SMALL`, blurred and toned: the picture's source.
    pub small: Vec<Rgb>,
    /// The bands' colours at the page's top, 55 % down and bottom.
    pub stops: [Rgb; 3],
    /// The floating card's and the bar's fill: the field's mean hue, deep or near white.
    pub card: Rgb,
    /// The lyrics' levels; `ground` is the field's mean colour (the side tab's panel).
    pub ink: LyricInk,
}

/// The cover's field for the dark theme and the light one. `None` for an empty cover.
pub fn fields<C: Deref<Target = [u8]>>(cover: &ImageBuffer<Rgba<u8>, C>) -> Option<[Field; 2]> {
    if cover.width() == 0 || cover.height() == 0 {
        return None;
    }
    let n = SMALL as u32;
    let mut small: Vec<Rgb> = image::imageops::resize(cover, n, n, FilterType::Triangle)
        .pixels()
        .map(|p| [p[0], p[1], p[2]].map(|v| f32::from(v) / 255.0))
        .collect();
    blur(&mut small, SMALL, RADIUS, 3);
    Some(toned(&small))
}

/// The fields of a plain mid grey: a song with no cover, or whose cover failed.
pub fn plain() -> [Field; 2] {
    toned(&[[0.5; 3]; SMALL * SMALL])
}

fn toned(small: &[Rgb]) -> [Field; 2] {
    [field(small, true), field(small, false)]
}

/// The calm tone: the pixel's own hue, its lightness in a narrow band (dark, or near white),
/// its chroma capped.
fn tone(p: Rgb, dark: bool) -> Rgb {
    let c = colour::to_lch(p);
    let l = c.l.clamp(0.0, 1.0);
    colour::to_srgb(if dark {
        Lch {
            l: 0.17 + 0.26 * l,
            c: c.c.min(0.10),
            h: c.h,
        }
    } else {
        Lch {
            l: 0.84 + 0.11 * l,
            c: (c.c * 0.6).min(0.055),
            h: c.h,
        }
    })
}

/// One theme's field from the blurred small picture. Not inlined: it's made for both themes.
#[inline(never)]
fn field(blurred: &[Rgb], dark: bool) -> Field {
    let small: Vec<Rgb> = blurred.iter().map(|&p| tone(p, dark)).collect();
    let ground = colour::quantise(mean(&small, 4..20));
    let m = colour::to_lch(ground);
    let card = colour::quantise(colour::to_srgb(if dark {
        Lch {
            l: 0.215,
            c: m.c.min(0.035),
            h: m.h,
        }
    } else {
        Lch {
            l: 0.975,
            c: m.c.min(0.018),
            h: m.h,
        }
    }));
    let stops = stops(&small, dark);
    // What the lines can sit on: the picture as the page shows it, the bands (the renderer
    // mixes their stops in 8 bits), the side tab's panel.
    let bands = (0..=16).flat_map(|k| {
        let t = k as f32 / 16.0;
        [mix(stops[0], stops[1], t), mix(stops[1], stops[2], t)]
    });
    let grid = grid(&small);
    // Lighter text on dark: the lightest pixel is the worst; darker on light, the darkest.
    let sign = if dark { 1.0 } else { -1.0 };
    let worst = shown(&grid).chain(bands).fold(ground, |w, p| {
        if sign * colour::luminance(p) > sign * colour::luminance(w) {
            p
        } else {
            w
        }
    });
    let ink = inks(worst, ground, m, dark);
    Field {
        small,
        stops,
        card,
        ink,
    }
}

/// The lyrics' levels on a field whose worst pixel is `worst` (its lightest on dark, its
/// darkest on light), tinted from its mean `hue`, as `colour::lyric_panel` and
/// `colour::light_tint` make them on the panel: in the dark theme the sung lines the dimmest
/// tint that keeps `LYRIC_PAST`, the lines to come `LYRIC_STEP_PAST` lighter and white playing;
/// in the light one near-black playing, and each level on a hovered or pressed line too.
fn inks(worst: Rgb, ground: Rgb, hue: Lch, dark: bool) -> LyricInk {
    let black = colour::hex(0x000000);
    let under = if dark {
        [worst; 3]
    } else {
        [
            worst,
            colour::over(black, colour::TINT_HOVER, worst),
            colour::over(black, colour::TINT_PRESSED, worst),
        ]
    };
    // Light text moves lighter until it keeps its floor, dark text darker.
    let step = if dark { 0.01 } else { -0.01 };
    let ink = |l: f32, c: f32, min: f32| {
        let at = Lch { l, c, h: hue.h };
        colour::quantise(colour::clamp(at, step, &under, min * HEADROOM))
    };
    let c = if dark {
        (hue.c * 0.35).min(0.05)
    } else {
        (hue.c * 0.9).min(0.06)
    };
    let past_l = if dark {
        colour::to_lch(worst).l + 0.05
    } else {
        0.58
    };
    let past = ink(past_l, c, colour::LYRIC_PAST);
    // A hair over each step, so rounding to 8 bits never lands under it.
    let past_l = colour::to_lch(past).l;
    let next_l = if dark {
        past_l + colour::LYRIC_STEP_PAST + 0.005
    } else {
        (past_l - colour::LYRIC_STEP_NEXT - 0.005).min(0.45)
    };
    LyricInk {
        ground,
        current: if dark {
            colour::hex(0xFFFFFF)
        } else {
            ink(0.16, (hue.c * 0.6).min(0.035), colour::LYRIC_CURRENT)
        },
        next: ink(next_l, c, colour::LYRIC_NEXT),
        past,
    }
}

/// The mean of `rows` of the small picture, its two columns at each side left out (the page
/// shows the cover's middle).
fn mean(small: &[Rgb], rows: Range<usize>) -> Rgb {
    let mut sum = [0f32; 3];
    let mut n = 0.0;
    for y in rows {
        for p in &small[y * SMALL + 2..(y + 1) * SMALL - 2] {
            for k in 0..3 {
                sum[k] += p[k];
            }
            n += 1.0;
        }
    }
    sum.map(|v| v / n)
}

/// The bands' stops: the means of the top, middle and lower thirds of what the page shows,
/// the lowest a step deeper (dark) or lighter (light) and greyer, so the page settles under
/// the bar.
fn stops(small: &[Rgb], dark: bool) -> [Rgb; 3] {
    let mut low = colour::to_lch(mean(small, 16..20));
    if dark {
        low.l = (low.l - 0.05).max(0.13);
        low.c = low.c.min(0.04);
    } else {
        low.l = (low.l + 0.03).min(0.97);
        low.c = low.c.min(0.02);
    }
    [
        mean(small, 4..10),
        mean(small, 10..16),
        colour::to_srgb(low),
    ]
    .map(colour::quantise)
}

fn mix(a: Rgb, b: Rgb, t: f32) -> Rgb {
    std::array::from_fn(|k| a[k] + (b[k] - a[k]) * t)
}

/// A box blur of radius `r` across and down a picture `n` pixels a side, `passes` times; the
/// edges repeat their last pixel.
fn blur(px: &mut [Rgb], n: usize, r: usize, passes: usize) {
    let mut line = vec![[0f32; 3]; n];
    for _ in 0..passes {
        for along_rows in [true, false] {
            for k in 0..n {
                let at = |i: usize| if along_rows { k * n + i } else { i * n + k };
                for (i, out) in line.iter_mut().enumerate() {
                    let mut sum = [0f32; 3];
                    for d in 0..=2 * r {
                        let p = px[at((i + d).saturating_sub(r).min(n - 1))];
                        for c in 0..3 {
                            sum[c] += p[c];
                        }
                    }
                    *out = sum.map(|s| s / (2 * r + 1) as f32);
                }
                for (i, v) in line.iter().enumerate() {
                    px[at(i)] = *v;
                }
            }
        }
    }
}

/// The field at `GRID` pixels a side: bilinear across then down, blurred so no cell's kink
/// shows.
fn grid(small: &[Rgb]) -> Vec<Rgb> {
    const UP: f32 = (GRID / SMALL) as f32;
    // Each grid pixel's two neighbours in the small picture, and where it falls between them.
    let taps: Vec<(usize, usize, f32)> = (0..GRID)
        .map(|o| {
            let u = ((o as f32 + 0.5) / UP - 0.5).clamp(0.0, (SMALL - 1) as f32);
            let i = u.floor() as usize;
            (i, (i + 1).min(SMALL - 1), u - i as f32)
        })
        .collect();
    let mut out = vec![[0f32; 3]; GRID * GRID];
    for (y, &(y0, y1, fy)) in taps.iter().enumerate() {
        for (x, &(x0, x1, fx)) in taps.iter().enumerate() {
            let at = |r: usize| mix(small[r * SMALL + x0], small[r * SMALL + x1], fx);
            out[y * GRID + x] = mix(at(y0), at(y1), fy);
        }
    }
    blur(&mut out, GRID, 2, 3);
    out
}

/// The grid's pixels any page shows (its middle `KEEP` square, and the next one out, which the
/// scaling mixes in): a page shows less than that square down or across, never more.
fn shown(grid: &[Rgb]) -> impl Iterator<Item = Rgb> + '_ {
    let n = GRID as f32;
    let from = (((1.0 - KEEP) / 2.0 * n - 0.5).floor().max(0.0)) as usize;
    let to = ((((1.0 + KEEP) / 2.0) * n - 0.5).ceil() as usize).min(GRID - 1);
    (from..=to).flat_map(move |y| (from..=to).map(move |x| grid[y * GRID + x]))
}

/// The cover's region a page of `w` × `h` shows (0-1, left, top, right, bottom): `KEEP` of it
/// across the longer side, as much less across the other as the page is narrower.
fn fit(w: u32, h: u32) -> [f32; 4] {
    let a = w as f32 / h as f32;
    let (fw, fh) = if a >= 1.0 {
        (KEEP, KEEP / a)
    } else {
        (KEEP * a, KEEP)
    };
    [
        0.5 - fw / 2.0,
        0.5 - fh / 2.0,
        0.5 + fw / 2.0,
        0.5 + fh / 2.0,
    ]
}

/// The box the lines sit in, in the picture's pixels, and the height of the fade at its top
/// and bottom: the strips are cut from there.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Lines {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    pub edge: u32,
}

/// What to make: a field's small picture (`Field::small`), the picture's size and where the
/// lines are.
pub struct Request<'a> {
    pub small: &'a [Rgb],
    pub width: u32,
    pub height: u32,
    pub lines: Option<Lines>,
}

/// A strip the lines fade into: the picture where it lies, its alpha going from opaque at the
/// lyrics' edge to clear `Lines::edge` pixels in, premultiplied.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Strip {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// The picture (opaque, in BGRA: the order the window is drawn in, so the renderer copies it
/// as it is) and its strips (RGBA).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Baked {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub top: Strip,
    pub bottom: Strip,
}

/// The size to make for a page of `width` × `height` physical pixels: the same, or with the
/// longer side at `MAX_SIDE`.
pub fn size_for(width: u32, height: u32) -> (u32, u32) {
    let long = width.max(height);
    if long <= MAX_SIDE {
        return (width, height);
    }
    let k = MAX_SIDE as f32 / long as f32;
    let scale = |v: u32| ((v as f32 * k).round() as u32).max(1);
    (scale(width), scale(height))
}

/// Makes the picture and its strips. `None` for an empty size or field.
pub fn bake(r: &Request) -> Option<Baked> {
    let mut pixels = vec![0; r.width as usize * r.height as usize * 4];
    let (top, bottom) = bake_into(r, &mut pixels)?;
    Some(Baked {
        width: r.width,
        height: r.height,
        pixels,
        top,
        bottom,
    })
}

/// Makes the picture into `out` (BGRA, `width` × `height`) and gives its strips. `None` for an
/// empty size or field, or an `out` of another size.
pub fn bake_into(r: &Request, out: &mut [u8]) -> Option<(Strip, Strip)> {
    if r.width == 0
        || r.height == 0
        || r.small.len() != SMALL * SMALL
        || out.len() != r.width as usize * r.height as usize * 4
    {
        return None;
    }
    scale_up(
        &grid(r.small),
        fit(r.width, r.height),
        r.width,
        r.height,
        out,
    );
    Some(
        r.lines
            .map(|l| strips(out, r.width, r.height, l))
            .unwrap_or_default(),
    )
}

/// A noise from 0 to 1 that changes from pixel to pixel without a pattern the eye picks up
/// (Jimenez's interleaved gradient noise): the dither.
fn noise(x: u32, y: u32) -> f32 {
    let f = 0.067_110_56 * x as f32 + 0.005_837_15 * y as f32;
    (52.982_918 * (f - f.floor())).fract()
}

/// The grid's `region` scaled up to `width` × `height`, bilinear, dithered and rounded to
/// opaque BGRA in `out`.
fn scale_up(grid: &[Rgb], region: [f32; 4], width: u32, height: u32, out: &mut [u8]) {
    let n = GRID as f32;
    let [u0, v0, u1, v1] = region;
    // Where an output pixel's centre falls in the grid, along one side of `len`.
    let place = |i: u32, len: u32, a: f32, b: f32| {
        let u = a + (i as f32 + 0.5) / len as f32 * (b - a);
        let g = (u * n - 0.5).clamp(0.0, n - 1.0);
        let i0 = g.floor() as usize;
        (i0, (i0 + 1).min(GRID - 1), g - i0 as f32)
    };
    let columns: Vec<(usize, usize, f32)> = (0..width).map(|x| place(x, width, u0, u1)).collect();
    let mut row = [[0f32; 3]; GRID];
    let mut px = out.as_chunks_mut::<4>().0.iter_mut();
    for y in 0..height {
        let (y0, y1, fy) = place(y, height, v0, v1);
        for (i, c) in row.iter_mut().enumerate() {
            *c = mix(grid[y0 * GRID + i], grid[y1 * GRID + i], fy);
        }
        for ((x, &(x0, x1, fx)), p) in columns.iter().enumerate().zip(&mut px) {
            let (a, b) = (row[x0], row[x1]);
            let d = noise(x as u32, y) - 0.5;
            for k in 0..3 {
                let v = (a[k] + (b[k] - a[k]) * fx) * 255.0 + d;
                p[2 - k] = v.round().clamp(0.0, 255.0) as u8;
            }
            p[3] = 255;
        }
    }
}

/// The two strips at the lines' top and bottom edges, cut from the picture (BGRA) into RGBA.
fn strips(pixels: &[u8], width: u32, height: u32, l: Lines) -> (Strip, Strip) {
    let x0 = l.x.min(width);
    let w = l.w.min(width - x0);
    let e = l.edge.min(l.h / 2);
    let cut = |y0: u32, top: bool| {
        let y0 = y0.min(height);
        let h = e.min(height - y0);
        let mut out = Vec::with_capacity(w as usize * h as usize * 4);
        for r in 0..h {
            // Opaque at the lyrics' edge, clear `e` pixels in.
            let t = (r as f32 + 0.5) / e as f32;
            let alpha = if top { 1.0 - t } else { t };
            let start = (((y0 + r) * width + x0) * 4) as usize;
            for p in pixels[start..start + w as usize * 4].as_chunks::<4>().0 {
                for v in [p[2], p[1], p[0]] {
                    out.push((f32::from(v) * alpha).round() as u8);
                }
                out.push((alpha * 255.0).round() as u8);
            }
        }
        Strip {
            width: if h > 0 { w } else { 0 },
            height: h,
            pixels: out,
        }
    };
    (cut(l.y, true), cut((l.y + l.h).saturating_sub(e), false))
}

#[cfg(test)]
mod tests {
    use super::*;
    use colour::{contrast, hex, to_lch};
    use image::RgbaImage;
    use proptest::prelude::*;

    const FLOORS: [f32; 3] = [
        colour::LYRIC_CURRENT,
        colour::LYRIC_NEXT,
        colour::LYRIC_PAST,
    ];

    /// A made-up cover of `n` pixels a side, from a colour at each point (0-1 across and down).
    fn paint(n: u32, f: impl Fn(f32, f32) -> Rgb) -> RgbaImage {
        RgbaImage::from_fn(n, n, |x, y| {
            let c = f((x as f32 + 0.5) / n as f32, (y as f32 + 0.5) / n as f32);
            let [r, g, b] = c.map(|v| (v * 255.0).round() as u8);
            Rgba([r, g, b, 255])
        })
    }

    fn inside(x: f32, y: f32, cx: f32, cy: f32, rx: f32, ry: f32) -> bool {
        ((x - cx) / rx).powi(2) + ((y - cy) / ry).powi(2) <= 1.0
    }

    /// The five kinds of cover the field was designed on: a dark, near-monochrome portrait
    /// with pale parts and a red type line; a flat, very saturated lime with black type;
    /// a colourful one; a light one; a near-black one.
    fn kinds() -> [(&'static str, RgbaImage); 5] {
        [
            (
                "mono",
                paint(48, |x, y| {
                    if y > 0.86 {
                        if (x * 10.0).fract() < 0.7 {
                            hex(0xCF1F2B)
                        } else {
                            hex(0x0A0A0A)
                        }
                    } else if inside(x, y, 0.34, 0.12, 0.36, 0.2) {
                        hex(0xE9E3D7)
                    } else if inside(x, y, 0.52, 0.62, 0.22, 0.27) {
                        hex(0xE2CEC6)
                    } else if inside(x, y, 0.07, 0.56, 0.15, 0.12) {
                        hex(0xF2F4F6)
                    } else {
                        hex(0x0C0C0E)
                    }
                }),
            ),
            (
                "lime",
                paint(48, |x, y| {
                    if x > 0.4 && (y * 6.5).fract() < 0.75 && (x * 17.0).fract() < 0.7 {
                        hex(0x050505)
                    } else {
                        hex(0x8ACE00)
                    }
                }),
            ),
            (
                "colour",
                paint(48, |x, y| {
                    if inside(x, y, 0.28, 0.72, 0.22, 0.22) {
                        hex(0x10A0A6)
                    } else if inside(x, y, 0.86, 0.14, 0.4, 0.17) {
                        hex(0x23277F)
                    } else {
                        mix(hex(0xD81B7A), hex(0xFF9A1F), (x + y) / 2.0)
                    }
                }),
            ),
            (
                "light",
                paint(48, |x, y| {
                    if inside(x, y, 0.62, 0.4, 0.22, 0.22) {
                        hex(0xF0C4AE)
                    } else if (0.76..0.79).contains(&y) && (0.1..0.46).contains(&x) {
                        hex(0x2B2B2B)
                    } else {
                        hex(0xF4F1EA)
                    }
                }),
            ),
            (
                "black",
                paint(48, |x, y| {
                    if inside(x, y, 0.68, 0.3, 0.34, 0.22) {
                        hex(0x151C35)
                    } else {
                        hex(0x060607)
                    }
                }),
            ),
        ]
    }

    fn px(b: &Baked, x: u32, y: u32) -> Rgb {
        let i = ((y * b.width + x) * 4) as usize;
        [2, 1, 0].map(|k| f32::from(b.pixels[i + k]) / 255.0)
    }

    /// The lowest contrast each ink (playing, to come, sung) has against `under`.
    fn worst(ink: &LyricInk, under: impl Iterator<Item = Rgb>) -> [f32; 3] {
        let mut w = [f32::MAX; 3];
        for p in under {
            for (k, c) in [ink.current, ink.next, ink.past].into_iter().enumerate() {
                w[k] = w[k].min(contrast(c, p));
            }
        }
        w
    }

    /// Every pixel of a baked picture.
    fn pixels(b: &Baked) -> impl Iterator<Item = Rgb> + '_ {
        (0..b.height).flat_map(move |y| (0..b.width).map(move |x| px(b, x, y)))
    }

    /// The bands as the page draws them: 128, each its stops' mix at its middle, in 8 bits.
    fn bands(f: &Field) -> impl Iterator<Item = Rgb> + '_ {
        (0..128).map(|i| {
            let at = (i as f32 + 0.5) / 128.0;
            colour::quantise(if at < 0.55 {
                mix(f.stops[0], f.stops[1], at / 0.55)
            } else {
                mix(f.stops[1], f.stops[2], (at - 0.55) / 0.45)
            })
        })
    }

    fn request(f: &Field, size: (u32, u32)) -> Request<'_> {
        Request {
            small: &f.small,
            width: size.0,
            height: size.1,
            lines: None,
        }
    }

    /// The floors on the picture at a wide and a narrow page, on the bands and on the side
    /// tab's panel, for a cover's fields; each ink's worst contrast, picture then bands.
    fn assert_floors(what: &str, fields: &[Field; 2]) -> [[f32; 3]; 4] {
        let mut seen = [[0.0; 3]; 4];
        for (t, f) in fields.iter().enumerate() {
            let mut on_picture = [f32::MAX; 3];
            for size in [(160, 90), (90, 120)] {
                let b = bake(&request(f, size)).expect("baked");
                let w = worst(&f.ink, pixels(&b));
                for k in 0..3 {
                    on_picture[k] = on_picture[k].min(w[k]);
                }
            }
            let on_bands = worst(&f.ink, bands(f).chain([f.ink.ground]));
            for (w, on) in [(on_picture, "picture"), (on_bands, "bands")] {
                for k in 0..3 {
                    assert!(
                        w[k] >= FLOORS[k],
                        "{what} dark={} {on} ink {k}: {}",
                        t == 0,
                        w[k]
                    );
                }
            }
            seen[2 * t] = on_picture;
            seen[2 * t + 1] = on_bands;
        }
        seen
    }

    #[test]
    fn the_blur_keeps_a_flat_colour_and_spreads_an_edge() {
        let mut flat = vec![[0.3, 0.5, 0.7]; SMALL * SMALL];
        blur(&mut flat, SMALL, RADIUS, 3);
        assert!(flat.iter().all(|p| (p[1] - 0.5).abs() < 1e-5));
        // Black on the left half, white on the right: a ramp across most of the picture,
        // the far sides keeping their colour.
        let mut step: Vec<Rgb> = (0..SMALL * SMALL)
            .map(|i| [if i % SMALL < SMALL / 2 { 0.0 } else { 1.0 }; 3])
            .collect();
        blur(&mut step, SMALL, RADIUS, 3);
        let at = |x: usize| step[10 * SMALL + x][0];
        assert!(at(0) < 0.01 && at(SMALL - 1) > 0.99);
        let ramp = (0..SMALL)
            .filter(|&x| (0.02..0.98).contains(&at(x)))
            .count();
        assert!((10..=18).contains(&ramp), "a ramp of {ramp} pixels");
    }

    /// Each part keeps its hue, darker and calmer on dark, near white on light.
    #[test]
    fn the_tone_keeps_each_hue_in_a_narrow_band() {
        for rgb in [0xFF0000, 0x8ACE00, 0x10A0A6, 0xF4F1EA, 0x060607, 0xFFFFFF] {
            let p = hex(rgb);
            let (d, l) = (to_lch(tone(p, true)), to_lch(tone(p, false)));
            assert!(
                (0.165..=0.435).contains(&d.l) && d.c <= 0.101,
                "{rgb:06X} dark {d:?}"
            );
            assert!(
                (0.835..=0.955).contains(&l.l) && l.c <= 0.056,
                "{rgb:06X} light {l:?}"
            );
            let src = to_lch(p);
            if src.c > 0.08 {
                let turn = (d.h - src.h).sin().abs().max((l.h - src.h).sin().abs());
                assert!(turn < 0.1, "{rgb:06X} turned {turn}");
            }
        }
    }

    /// On each kind of cover, in either theme, every line keeps its floor on every pixel of
    /// the picture (a wide page and a tall one), of the bands and of the side tab's panel.
    /// With `--no-capture` it prints each worst contrast (playing, to come, sung).
    #[test]
    #[allow(clippy::print_stdout)]
    fn the_five_kinds_of_cover_keep_the_lyrics_floors() {
        for (name, cover) in kinds() {
            let seen = assert_floors(name, &fields(&cover).expect("fields"));
            println!(
                "{name}: dark picture {:.2?}, bands {:.2?}; light picture {:.2?}, bands {:.2?}",
                seen[0], seen[1], seen[2], seen[3]
            );
        }
        assert_floors("plain", &plain());
    }

    /// The field is calm: no pixel of the picture past the tone's band, the bands within
    /// it (the lowest a step deeper or lighter), the card a deep or near-white tint of it.
    #[test]
    fn the_field_stays_in_its_band() {
        for (name, cover) in kinds() {
            let [dark, light] = fields(&cover).expect("fields");
            for (f, (lo, hi)) in [(&dark, (0.125, 0.44)), (&light, (0.83, 0.975))] {
                let b = bake(&request(f, (120, 80))).expect("baked");
                for p in pixels(&b).chain(f.stops) {
                    let l = to_lch(p).l;
                    assert!((lo..=hi).contains(&l), "{name}: {l}");
                }
            }
            let (d, l) = (to_lch(dark.card), to_lch(light.card));
            assert!((d.l - 0.215).abs() < 0.01 && d.c <= 0.037, "{name}: {d:?}");
            assert!((l.l - 0.975).abs() < 0.01 && l.c <= 0.02, "{name}: {l:?}");
        }
    }

    /// The cover's shapes stay round: a disc in the middle of the cover is as wide as it is
    /// tall on a wide page, not stretched to its shape.
    #[test]
    fn the_cover_keeps_its_shape() {
        let disc = paint(48, |x, y| {
            if inside(x, y, 0.5, 0.5, 0.25, 0.25) {
                [1.0; 3]
            } else {
                [0.0; 3]
            }
        });
        let [dark, _] = fields(&disc).expect("fields");
        let b = bake(&request(&dark, (320, 160))).expect("baked");
        let lum = |x: u32, y: u32| colour::luminance(px(&b, x, y));
        let (peak, low) = (lum(160, 80), lum(0, 80).min(lum(160, 0)));
        let half = (peak + low) / 2.0;
        let across = (0..320).filter(|&x| lum(x, 80) > half).count() as f32;
        let down = (0..160).filter(|&y| lum(160, y) > half).count() as f32;
        // Down is cut by the page; across is the disc's width.
        let tall = (0..160).all(|y| lum(160, y) > half);
        assert!(
            tall || (across / down - 1.0).abs() < 0.15,
            "{across} x {down}"
        );
        assert!(across < 0.7 * 320.0, "stretched across: {across}");
    }

    #[test]
    fn the_picture_is_opaque_and_the_pages_size() {
        let [dark, light] = fields(&kinds()[2].1).expect("fields");
        for f in [&dark, &light] {
            let b = bake(&request(f, (240, 150))).expect("baked");
            assert_eq!((b.width, b.height), (240, 150));
            assert_eq!(b.pixels.len(), 240 * 150 * 4);
            assert!(b.pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
            // The cover's colours stay where they were: the page isn't one colour.
            let (a, z) = (to_lch(px(&b, 20, 140)), to_lch(px(&b, 220, 10)));
            assert!((a.h - z.h).sin().abs() > 0.2, "{a:?} {z:?}");
        }
        assert!(bake(&request(&dark, (0, 10))).is_none());
    }

    #[test]
    fn the_strips_are_the_picture_where_the_lines_end_fading_inward() {
        let [dark, _] = fields(&kinds()[2].1).expect("fields");
        let lines = Lines {
            x: 40,
            y: 20,
            w: 100,
            h: 90,
            edge: 12,
        };
        let mut r = request(&dark, (200, 120));
        r.lines = Some(lines);
        let b = bake(&r).expect("baked");
        for (s, y0, top) in [(&b.top, 20, true), (&b.bottom, 98, false)] {
            assert_eq!((s.width, s.height), (100, 12));
            let alpha = |row: u32| s.pixels[(row * s.width * 4 + 3) as usize];
            let (edge, inner) = if top { (0, 11) } else { (11, 0) };
            assert!(alpha(edge) > 240 && alpha(inner) < 15, "top={top}");
            // Unpremultiplied, the strip's pixel is the picture's under it.
            let i = (edge * s.width * 4 + 5 * 4) as usize;
            let a = f32::from(s.pixels[i + 3]) / 255.0;
            let p = px(&b, 45, y0 + edge);
            for k in 0..3 {
                let got = f32::from(s.pixels[i + k]) / 255.0 / a;
                assert!((got - p[k]).abs() < 0.03, "top={top}: {got} vs {}", p[k]);
            }
        }
    }

    #[test]
    fn a_stage_past_the_cap_is_made_smaller_in_proportion() {
        assert_eq!(size_for(1168, 900), (1168, 900));
        assert_eq!(size_for(3240, 2160), (3240, 2160));
        assert_eq!(size_for(5120, 2880), (3840, 2160));
        assert_eq!(size_for(0, 10), (0, 10));
    }

    /// How long the fields take (the option off: the bands, the card and the inks, from the
    /// cover's small decode), and the picture (the option on), at the 1568 x 900 window's
    /// page at 100 % and the 4K page at 150 %. A measurement (ignored): run it optimised, with
    /// `--run-ignored only --no-capture`.
    #[test]
    #[ignore]
    #[allow(clippy::print_stdout)]
    fn how_long_a_field_and_a_picture_take() {
        let best = |f: &mut dyn FnMut()| {
            (0..7)
                .map(|_| {
                    let t = std::time::Instant::now();
                    f();
                    t.elapsed().as_secs_f64() * 1000.0
                })
                .fold(f64::MAX, f64::min)
        };
        // As `follow` decodes it, at 48 px.
        let cover = image::imageops::resize(&kinds()[0].1, 48, 48, FilterType::Triangle);
        let made = best(&mut || {
            std::hint::black_box(fields(&cover));
        });
        println!("fields, both themes: {made:.2} ms best of 7");
        let [dark, light] = fields(&cover).expect("fields");
        for (name, size, lines) in [
            (
                "1568 x 900 page",
                (1568, 900),
                Lines {
                    x: 360,
                    y: 72,
                    w: 736,
                    h: 728,
                    edge: 24,
                },
            ),
            (
                "4K page at 150 %",
                (3840, 2160),
                Lines {
                    x: 870,
                    y: 108,
                    w: 1128,
                    h: 1806,
                    edge: 36,
                },
            ),
        ] {
            for f in [&dark, &light] {
                let mut r = request(f, size);
                r.lines = Some(lines);
                let mut out = vec![0; size.0 as usize * size.1 as usize * 4];
                let t = best(&mut || {
                    bake_into(&r, &mut out).expect("baked");
                });
                println!(
                    "{name} ({}x{}), dark={}: picture {t:.1} ms best of 7",
                    size.0,
                    size.1,
                    std::ptr::eq(f, &dark)
                );
            }
        }
    }

    /// A cover of up to sixteen colours in tiles, any of them.
    fn cover() -> impl Strategy<Value = RgbaImage> {
        prop::collection::vec(prop::array::uniform3(0u8..=255), 1..=16).prop_map(|v| {
            let n = (v.len() as f32).sqrt().ceil() as u32;
            RgbaImage::from_fn(n * 12, n * 12, |x, y| {
                let i = ((y / 12) * n + x / 12) as usize;
                let [r, g, b] = v.get(i).copied().unwrap_or([0; 3]);
                Rgba([r, g, b, 255])
            })
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(48))]

        /// Whatever the cover, in either theme, each line keeps its floor against every pixel
        /// of the picture as drawn, of the bands and of the side tab's panel.
        #[test]
        fn every_cover_keeps_the_lyrics_floors(cover in cover()) {
            let fields = fields(&cover).expect("fields");
            for f in &fields {
                let b = bake(&request(f, (96, 60))).expect("baked");
                for (w, on) in [
                    (worst(&f.ink, pixels(&b)), "picture"),
                    (worst(&f.ink, bands(f).chain([f.ink.ground])), "bands"),
                ] {
                    for k in 0..3 {
                        prop_assert!(w[k] >= FLOORS[k], "{} ink {} at {}", on, k, w[k]);
                    }
                }
            }
        }
    }
}
