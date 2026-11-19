//! "Cover behind the lyrics", the maths: the song's cover, blurred, under a veil of the
//! lyrics' panel colour, as one opaque picture the size of Now playing's page in physical
//! pixels (the stage, and round the Up next card), and the two strips the lines fade into at the top and bottom of the lyrics.
//!
//! Made once per song (and again for a new theme or a settled resize) on tokio's blocking
//! pool, never while the song plays (`pages::now_playing::backdrop` asks for it). The cover is
//! shrunk to `SMALL` pixels a side and blurred there, with a box blur run three times (about a
//! Gaussian); the veil goes on in the small picture, then it's scaled up to the page with a
//! light dither against banding. Pure.
//!
//! Contrast: the veil starts at `VEIL_DARK` or `VEIL_LIGHT` of the panel colour and goes up in
//! `VEIL_STEP`s until each of the lyrics' inks (the line playing, the lines to come, the lines
//! sung) keeps its floor (`colour::LYRIC_*`, or what it has on the plain panel where that's
//! less) against every pixel of the picture: the lines, the song's title and the header all sit
//! on it. Up to `VEIL_DARK_MOST` or `VEIL_LIGHT_MOST`; past that the veil goes up only on the
//! pixels that still need it, each by as little as they need.

use image::imageops::FilterType;
use image::{ImageBuffer, Rgba};

use super::colour::{self, LyricInk, Rgb};

/// The cover is shrunk to this many pixels a side before it's blurred.
pub const SMALL: usize = 64;
/// The box blur's radius in the small picture, run three times: about a Gaussian of
/// sigma √(r² + r) ≈ 3.5 px, a twentieth of the cover, as wide as the board's blur.
const RADIUS: usize = 3;
/// The share of each side of the cover left out: the picture is its middle, larger (the board
/// draws the blurred cover 160 px past each side of the stage).
const MARGIN: f32 = 0.1;
/// More colour after the blur, as the board's `saturate(1.2)`: a blur greys colours.
const SATURATE: f32 = 1.2;
/// The least veil of the panel colour over the picture, per theme.
pub const VEIL_DARK: f32 = 0.58;
pub const VEIL_LIGHT: f32 = 0.70;
/// The most the veil goes up as a whole, per theme; past it, only where it must
/// (`local_veil`). The deep panel of the dark theme holds its sung lines at their floor, so any
/// part of a cover lighter than it needs more veil there, which on most covers would take the
/// whole picture to the plain colour.
pub const VEIL_DARK_MOST: f32 = 0.70;
pub const VEIL_LIGHT_MOST: f32 = 0.82;
/// How far the veil goes up at a time while an ink is under its floor.
const VEIL_STEP: f32 = 0.02;
/// The contrast a step must keep above the floors, as a factor: the scaling up mixes
/// neighbouring pixels (a mix of two hues can be a little darker than both), and the dither
/// and the rounding to 8 bits move each pixel by up to a level.
const HEADROOM: f32 = 1.04;
/// The longest side made; past it the picture is drawn larger than it is.
pub const MAX_SIDE: u32 = 3840;

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

/// What to make: the cover's pixels (RGBA), the picture's size, where the lines are, the
/// lyrics' inks for the theme shown and which theme that is.
pub struct Request<'a> {
    pub cover: &'a [u8],
    pub cover_w: u32,
    pub cover_h: u32,
    pub width: u32,
    pub height: u32,
    pub lines: Option<Lines>,
    pub ink: LyricInk,
    pub dark: bool,
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
/// as it is), its strips (RGBA), and the veil it took.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Baked {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
    pub top: Strip,
    pub bottom: Strip,
    pub veil: f32,
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

/// Makes the picture and its strips. `None` for an empty cover or size.
pub fn bake(r: &Request) -> Option<Baked> {
    let mut pixels = vec![0; r.width as usize * r.height as usize * 4];
    let (veil, top, bottom) = bake_into(r, &mut pixels)?;
    Some(Baked {
        width: r.width,
        height: r.height,
        pixels,
        top,
        bottom,
        veil,
    })
}

/// Makes the picture into `out` (BGRA, `width` × `height`), and gives the veil it took and
/// its strips. `None` for an empty cover or size, or an `out` of another size.
pub fn bake_into(r: &Request, out: &mut [u8]) -> Option<(f32, Strip, Strip)> {
    if r.width == 0 || r.height == 0 || out.len() != r.width as usize * r.height as usize * 4 {
        return None;
    }
    let mut small = shrink(r.cover, r.cover_w, r.cover_h)?;
    blur(&mut small);
    for p in &mut small {
        *p = saturate(*p);
    }
    let veil = veil_for(&small, &r.ink, r.dark);
    let limits = Limits::new(&r.ink);
    let veiled: Vec<Rgb> = small
        .iter()
        .map(|&p| local_veil(p, r.ink.ground, veil, &limits))
        .collect();
    scale_up(&veiled, r.width, r.height, out);
    let (top, bottom) = r
        .lines
        .map(|l| strips(out, r.width, r.height, l))
        .unwrap_or_default();
    Some((veil, top, bottom))
}

/// The cover at `SMALL` × `SMALL`, as colours from 0 to 1.
fn shrink(cover: &[u8], w: u32, h: u32) -> Option<Vec<Rgb>> {
    let view: ImageBuffer<Rgba<u8>, &[u8]> = ImageBuffer::from_raw(w, h, cover)?;
    if w == 0 || h == 0 {
        return None;
    }
    let n = SMALL as u32;
    let small = image::imageops::resize(&view, n, n, FilterType::Triangle);
    Some(
        small
            .pixels()
            .map(|p| [p[0], p[1], p[2]].map(|v| f32::from(v) / 255.0))
            .collect(),
    )
}

/// A box blur of `RADIUS` across and down, three times; the edges repeat their last pixel.
fn blur(px: &mut [Rgb]) {
    let mut line = [[0f32; 3]; SMALL];
    for _ in 0..3 {
        for along_rows in [true, false] {
            for k in 0..SMALL {
                let at = |i: usize| {
                    if along_rows {
                        k * SMALL + i
                    } else {
                        i * SMALL + k
                    }
                };
                for (i, out) in line.iter_mut().enumerate() {
                    let mut sum = [0f32; 3];
                    for d in 0..=2 * RADIUS {
                        let j = (i + d).saturating_sub(RADIUS).min(SMALL - 1);
                        let p = px[at(j)];
                        for c in 0..3 {
                            sum[c] += p[c];
                        }
                    }
                    *out = sum.map(|s| s / (2 * RADIUS + 1) as f32);
                }
                for (i, v) in line.iter().enumerate() {
                    px[at(i)] = *v;
                }
            }
        }
    }
}

/// CSS's `saturate(SATURATE)`.
fn saturate([r, g, b]: Rgb) -> Rgb {
    let s = SATURATE;
    [
        (0.213 + 0.787 * s) * r + (0.715 - 0.715 * s) * g + (0.072 - 0.072 * s) * b,
        (0.213 - 0.213 * s) * r + (0.715 + 0.285 * s) * g + (0.072 - 0.072 * s) * b,
        (0.213 - 0.213 * s) * r + (0.715 - 0.715 * s) * g + (0.072 + 0.928 * s) * b,
    ]
    .map(|v| v.clamp(0.0, 1.0))
}

/// The small picture's pixels the stage shows (`MARGIN` in from each side, and the next one
/// out, which the scaling mixes in).
fn shown(small: &[Rgb]) -> impl Iterator<Item = Rgb> + '_ {
    let n = SMALL as f32;
    let from = ((MARGIN * n - 0.5).floor().max(0.0)) as usize;
    let to = (((1.0 - MARGIN) * n - 0.5).ceil() as usize).min(SMALL - 1);
    (from..=to).flat_map(move |y| (from..=to).map(move |x| small[y * SMALL + x]))
}

/// What each ink must keep against a pixel, as the most luminance (dark theme: light text) or
/// the least (light theme: dark text) the pixel may have: its floor with `HEADROOM`, or what
/// it has on the plain panel where that's less (the sung lines' ink is the dimmest that keeps
/// its floor there, so it needs exactly the panel's).
struct Limits {
    /// The inks' luminance and the contrast each needs.
    inks: [(f32, f32); 3],
}

impl Limits {
    fn new(ink: &LyricInk) -> Limits {
        let floors = [
            (ink.current, colour::LYRIC_CURRENT),
            (ink.next, colour::LYRIC_NEXT),
            (ink.past, colour::LYRIC_PAST),
        ];
        Limits {
            inks: floors.map(|(c, floor)| {
                let panel = shifted(ink.ground).map(|g| colour::contrast(c, g));
                let need = (floor * HEADROOM).min(panel[0]).min(panel[1]);
                (colour::luminance(c), need)
            }),
        }
    }

    /// Every ink keeps what it needs against `p`, and against `p` a level of 8 bits lighter
    /// or darker, where the dither and the rounding may take it.
    fn hold(&self, p: Rgb) -> bool {
        shifted(p).into_iter().all(|p| {
            let y = colour::luminance(p);
            self.inks.iter().all(|&(i, need)| {
                let (hi, lo) = if i > y { (i, y) } else { (y, i) };
                (hi + 0.05) / (lo + 0.05) >= need
            })
        })
    }
}

/// `p` a level of 8 bits darker and lighter.
fn shifted(p: Rgb) -> [Rgb; 2] {
    [-1.0, 1.0].map(|d: f32| p.map(|v| (v + d / 255.0).clamp(0.0, 1.0)))
}

/// The veil over `small`: the theme's least, raised in `VEIL_STEP`s until each ink keeps what
/// it needs (`Limits`) against every pixel shown, up to `VEIL_MOST`. Past that, the veil goes
/// up only where a pixel still needs it (`local_veil`), so a bright patch of the cover doesn't
/// veil the whole picture.
pub fn veil_for(small: &[Rgb], ink: &LyricInk, dark: bool) -> f32 {
    let (base, most) = if dark {
        (VEIL_DARK, VEIL_DARK_MOST)
    } else {
        (VEIL_LIGHT, VEIL_LIGHT_MOST)
    };
    let limits = Limits::new(ink);
    let mut veil = base;
    while veil < most {
        if shown(small).all(|p| limits.hold(colour::over(ink.ground, veil, p))) {
            return veil;
        }
        veil = (veil + VEIL_STEP).min(most);
    }
    most
}

/// `p` under the veil, raised further where it must be: the least more of the panel colour
/// that lets every ink keep what it needs (to a thousandth), so the picture keeps its
/// gradients. The panel colour itself always holds.
fn local_veil(p: Rgb, ground: Rgb, veil: f32, limits: &Limits) -> Rgb {
    let at = |v: f32| colour::over(ground, v, p);
    if limits.hold(at(veil)) {
        return at(veil);
    }
    let (mut lo, mut hi) = (veil, 1.0);
    while hi - lo > 0.001 {
        let mid = (lo + hi) / 2.0;
        if limits.hold(at(mid)) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    at(hi)
}

/// A noise from 0 to 1 that changes from pixel to pixel without a pattern the eye picks up
/// (Jimenez's interleaved gradient noise): the dither.
fn noise(x: u32, y: u32) -> f32 {
    let f = 0.067_110_56 * x as f32 + 0.005_837_15 * y as f32;
    (52.982_918 * (f - f.floor())).fract()
}

/// The veiled small picture's middle scaled up to `width` × `height`, bilinear, dithered and
/// rounded to opaque BGRA in `out`.
fn scale_up(small: &[Rgb], width: u32, height: u32, out: &mut [u8]) {
    let n = SMALL as f32;
    // Where an output pixel's centre falls in the small picture, along one side of `len`.
    let place = |i: u32, len: u32| {
        let u = MARGIN + (i as f32 + 0.5) / len as f32 * (1.0 - 2.0 * MARGIN);
        let g = (u * n - 0.5).clamp(0.0, n - 1.0);
        let i0 = g.floor() as usize;
        (i0, (i0 + 1).min(SMALL - 1), g - i0 as f32)
    };
    let columns: Vec<(usize, usize, f32)> = (0..width).map(|x| place(x, width)).collect();
    let mut row = [[0f32; 3]; SMALL];
    let mut px = out.as_chunks_mut::<4>().0.iter_mut();
    for y in 0..height {
        let (y0, y1, fy) = place(y, height);
        for (i, c) in row.iter_mut().enumerate() {
            let (a, b) = (small[y0 * SMALL + i], small[y1 * SMALL + i]);
            *c = std::array::from_fn(|k| a[k] + (b[k] - a[k]) * fy);
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
    use colour::{Lch, contrast, hex, light_tint, lyric_panel, to_lch};
    use proptest::prelude::*;

    fn rgba(colours: &[Rgb], side: u32) -> Vec<u8> {
        // `colours` in square tiles, `side` pixels a tile, in a row of tiles per sqrt.
        let n = (colours.len() as f32).sqrt().ceil() as u32;
        let w = n * side;
        let mut out = Vec::with_capacity((w * w * 4) as usize);
        for y in 0..w {
            for x in 0..w {
                let i = ((y / side) * n + x / side) as usize;
                let c = colours.get(i).copied().unwrap_or([0.0; 3]);
                out.extend(c.map(|v| (v * 255.0).round() as u8));
                out.push(255);
            }
        }
        out
    }

    /// The inks as `LyricsData` holds them: 8 bits a channel.
    fn inks(seed: Lch, dark: bool) -> LyricInk {
        let i = if dark {
            lyric_panel(seed, true)
        } else {
            light_tint(seed)
        };
        let q = |c: Rgb| c.map(|v| (v * 255.0).round() / 255.0);
        LyricInk {
            ground: q(i.ground),
            current: q(i.current),
            next: q(i.next),
            past: q(i.past),
        }
    }

    /// What each ink must keep on the picture: its floor, or where that's less what it has
    /// on the plain panel a level of 8 bits off (the sung lines' ink is the dimmest that keeps
    /// its floor there; the dither moves a pixel by up to a level).
    fn least(ink: &LyricInk) -> [f32; 3] {
        let panel = [ink.current, ink.next, ink.past].map(|c| {
            let [a, b] = shifted(ink.ground).map(|g| contrast(c, g));
            a.min(b)
        });
        std::array::from_fn(|k| FLOORS[k].min(panel[k]))
    }

    fn request<'a>(
        cover: &'a [u8],
        side: u32,
        size: (u32, u32),
        ink: LyricInk,
        dark: bool,
    ) -> Request<'a> {
        Request {
            cover,
            cover_w: side,
            cover_h: side,
            width: size.0,
            height: size.1,
            lines: None,
            ink,
            dark,
        }
    }

    fn px(b: &Baked, x: u32, y: u32) -> Rgb {
        let i = ((y * b.width + x) * 4) as usize;
        [2, 1, 0].map(|k| f32::from(b.pixels[i + k]) / 255.0)
    }

    /// The lowest contrast each ink has against the picture, every pixel of it.
    fn worst(b: &Baked, ink: &LyricInk) -> [f32; 3] {
        let mut w = [f32::MAX; 3];
        for y in 0..b.height {
            for x in 0..b.width {
                let p = px(b, x, y);
                for (k, c) in [ink.current, ink.next, ink.past].into_iter().enumerate() {
                    w[k] = w[k].min(contrast(c, p));
                }
            }
        }
        w
    }

    const FLOORS: [f32; 3] = [
        colour::LYRIC_CURRENT,
        colour::LYRIC_NEXT,
        colour::LYRIC_PAST,
    ];

    #[test]
    fn the_blur_keeps_a_flat_colour_and_spreads_an_edge() {
        let mut flat = vec![[0.3, 0.5, 0.7]; SMALL * SMALL];
        blur(&mut flat);
        assert!(flat.iter().all(|p| (p[1] - 0.5).abs() < 1e-5));

        // Black on the left half, white on the right: after the blur the step is a ramp
        // several pixels wide, and the far sides keep their colour.
        let mut step: Vec<Rgb> = (0..SMALL * SMALL)
            .map(|i| {
                if i % SMALL < SMALL / 2 {
                    [0.0; 3]
                } else {
                    [1.0; 3]
                }
            })
            .collect();
        blur(&mut step);
        let at = |x: usize| step[10 * SMALL + x][0];
        assert!(at(0) < 0.01 && at(SMALL - 1) > 0.99);
        let ramp = (0..SMALL)
            .filter(|&x| (0.02..0.98).contains(&at(x)))
            .count();
        assert!((8..=20).contains(&ramp), "a ramp of {ramp} pixels");
        assert!((at(SMALL / 2) - 0.5).abs() < 0.15);
    }

    #[test]
    fn a_cover_in_the_panels_colour_takes_the_least_veil() {
        let seed = Lch {
            l: 0.5,
            c: 0.12,
            h: 4.4,
        };
        for dark in [true, false] {
            let ink = inks(seed, dark);
            let small = vec![ink.ground; SMALL * SMALL];
            let base = if dark { VEIL_DARK } else { VEIL_LIGHT };
            assert_eq!(veil_for(&small, &ink, dark), base, "dark={dark}");
        }
    }

    #[test]
    fn a_white_cover_raises_the_veil_on_dark_and_black_on_light() {
        let seed = Lch {
            l: 0.5,
            c: 0.12,
            h: 1.0,
        };
        let white = vec![[1.0; 3]; SMALL * SMALL];
        let black = vec![[0.0; 3]; SMALL * SMALL];
        assert!(veil_for(&white, &inks(seed, true), true) > VEIL_DARK);
        assert!(veil_for(&black, &inks(seed, false), false) > VEIL_LIGHT);
    }

    #[test]
    fn past_the_most_the_veil_rises_only_where_a_pixel_needs_it() {
        let seed = Lch {
            l: 0.5,
            c: 0.12,
            h: 4.4,
        };
        let ink = inks(seed, true);
        let limits = Limits::new(&ink);
        // Darker than the deep panel: the veil as it is. White: all but the panel colour.
        let dark = local_veil([0.02; 3], ink.ground, VEIL_DARK_MOST, &limits);
        assert_eq!(dark, colour::over(ink.ground, VEIL_DARK_MOST, [0.02; 3]));
        let white = local_veil([1.0; 3], ink.ground, VEIL_DARK_MOST, &limits);
        assert!(limits.hold(white));
        assert!(contrast(white, ink.ground) < 1.1, "{white:?}");
    }

    #[test]
    fn the_picture_is_opaque_the_stages_size_and_keeps_the_floors() {
        let seed = Lch {
            l: 0.6,
            c: 0.15,
            h: 0.8,
        };
        // An indigo night with a warm sun, as the board's cover.
        let cover = rgba(
            &[hex(0x3F488C), hex(0xB0607A), hex(0x2A2F6B), hex(0x8D5A9E)],
            32,
        );
        for dark in [true, false] {
            let ink = inks(seed, dark);
            let b = bake(&request(&cover, 64, (240, 150), ink, dark)).expect("baked");
            assert_eq!((b.width, b.height), (240, 150));
            assert_eq!(b.pixels.len(), 240 * 150 * 4);
            assert!(b.pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 255));
            let (w, least) = (worst(&b, &ink), least(&ink));
            for k in 0..3 {
                assert!(w[k] >= least[k], "dark={dark} ink {k}: {}", w[k]);
            }
            assert!(b.veil < 1.0, "dark={dark}: all panel");
            // Still the cover's picture, not the panel: the colours vary across it.
            let (a, z) = (to_lch(px(&b, 10, 10)), to_lch(px(&b, 230, 140)));
            assert!(
                (a.l - z.l).abs() + (a.h - z.h).abs() > 0.01,
                "a flat picture"
            );
        }
    }

    #[test]
    fn the_strips_are_the_picture_where_the_lines_end_fading_inward() {
        let seed = Lch {
            l: 0.5,
            c: 0.1,
            h: 2.0,
        };
        let cover = rgba(&[hex(0x3F488C), hex(0xF5B26B)], 16);
        let lines = Lines {
            x: 40,
            y: 20,
            w: 100,
            h: 90,
            edge: 12,
        };
        let mut r = request(&cover, 32, (200, 120), inks(seed, true), true);
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

    /// How long a picture takes, cover to strips: at the stage of a 1568 x 900 window at
    /// 100 % and of a 4K one at 150 % (side panel open), and at the whole 4K page (the
    /// picture's size since it runs round the Up next card), from a cover decoded at Now
    /// playing's size. A measurement (ignored): run it optimised, with `--run-ignored only --no-capture`.
    #[test]
    #[ignore]
    #[allow(clippy::print_stdout)]
    fn how_long_a_picture_takes() {
        let colours: Vec<Rgb> = (0..16)
            .map(|i| hex([0x3F488C, 0xB0607A, 0x2A2F6B, 0xF5B26B][i % 4] + i as u32 * 0x010203))
            .collect();
        let cover = rgba(&colours, 110);
        let seed = Lch {
            l: 0.5,
            c: 0.12,
            h: 4.4,
        };
        for (name, size, lines) in [
            (
                "1568 x 900",
                (1168, 900),
                Lines {
                    x: 360,
                    y: 72,
                    w: 736,
                    h: 728,
                    edge: 24,
                },
            ),
            (
                "4K at 150 %",
                (3240, 2160),
                Lines {
                    x: 870,
                    y: 108,
                    w: 1128,
                    h: 1806,
                    edge: 36,
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
            for dark in [true, false] {
                let mut r = request(&cover, 440, size, inks(seed, dark), dark);
                r.lines = Some(lines);
                let mut out = vec![0; size.0 as usize * size.1 as usize * 4];
                let runs: Vec<f64> = (0..5)
                    .map(|_| {
                        let t = std::time::Instant::now();
                        bake_into(&r, &mut out).expect("baked");
                        t.elapsed().as_secs_f64() * 1000.0
                    })
                    .collect();
                let best = runs.iter().copied().fold(f64::MAX, f64::min);
                let worst = runs.iter().copied().fold(0.0, f64::max);
                println!(
                    "{name} ({}x{}), dark={dark}: {best:.1} ms best, {worst:.1} worst of 5",
                    size.0, size.1
                );
            }
        }
    }

    /// A cover of up to sixteen colours in tiles, any of them.
    fn cover() -> impl Strategy<Value = Vec<Rgb>> {
        prop::collection::vec(prop::array::uniform3(0u8..=255), 1..=16).prop_map(|v| {
            v.into_iter()
                .map(|c| c.map(|x| f32::from(x) / 255.0))
                .collect()
        })
    }

    fn seed() -> impl Strategy<Value = Lch> {
        (0.05f32..0.98, 0.0f32..0.32, 0.0f32..std::f32::consts::TAU).prop_map(|(l, c, h)| Lch {
            l,
            c,
            h,
        })
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// Whatever the cover and its colour, in either theme, each ink keeps its floor (or
        /// what it has on the plain panel) against every pixel of the picture as drawn: the
        /// veil goes up as far as it must.
        #[test]
        fn every_cover_keeps_the_lyrics_floors(colours in cover(), seed in seed(), dark: bool) {
            let ink = inks(seed, dark);
            let pixels = rgba(&colours, 12);
            let side = (pixels.len() / 4).isqrt() as u32;
            let b = bake(&request(&pixels, side, (96, 60), ink, dark)).expect("baked");
            let (w, least) = (worst(&b, &ink), least(&ink));
            for k in 0..3 {
                prop_assert!(w[k] >= least[k], "ink {} at {} (veil {})", k, w[k], b.veil);
            }
        }
    }
}
