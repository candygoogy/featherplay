//! `image::imageops::resize` with `FilterType::CatmullRom`, byte for byte, for RGBA, on
//! x86-64 at four times the speed in the release build. An RGB source (every JPEG cover) is
//! read as RGBA with an opaque alpha, the bytes its `into_rgba8` copy gave, without the copy
//! (~0.13 ms and a third more than the decoded image's bytes per cover, 2026-11-27).
//!
//! image's resize is generic, so it's compiled in this crate at `opt-level = "z"`, where
//! nothing is vectorised: a 4K card's cover (300 px from 544) took ~3.0 ms of resizing
//! against ~0.6 ms of decoding (2026-11-21). Here each pixel's four channels are one SSE2
//! register (x86-64's baseline). Every operation is the IEEE one image does, lane by lane and
//! in the same order (no fused multiply-add), so the bytes come out the same: ~0.75 ms.
//! Other architectures keep image's resize (`with_image`), which the tests also run on x86-64
//! to check the two against each other.

/// A decoded cover: RGBA, or RGB (read with an opaque alpha).
pub type Source<P> = image::ImageBuffer<P, Vec<u8>>;

/// The `w`×`h` rectangle at `x`, `y` of `img`, resized to `nw`×`nh` into `out` (RGBA, `nw`
/// pixels a row).
pub fn catmull_rom_into<P: image::Pixel<Subpixel = u8> + 'static>(
    img: &Source<P>,
    rect: (u32, u32, u32, u32),
    nw: u32,
    nh: u32,
    out: &mut [u8],
) {
    #[cfg(target_arch = "x86_64")]
    sse2::catmull_rom_into(img, rect, nw, nh, out);
    #[cfg(not(target_arch = "x86_64"))]
    with_image(img, rect, nw, nh, out);
}

/// image's own resize: what architectures without the SSE2 path run.
#[cfg_attr(
    all(target_arch = "x86_64", not(test)),
    expect(
        dead_code,
        reason = "x86-64 runs the SSE2 path; the tests check it against this"
    )
)]
fn with_image<P: image::Pixel<Subpixel = u8> + 'static>(
    img: &Source<P>,
    (x, y, w, h): (u32, u32, u32, u32),
    nw: u32,
    nh: u32,
    out: &mut [u8],
) {
    let view = image::imageops::crop_imm(img, x, y, w, h);
    let resized = image::imageops::resize(&*view, nw, nh, image::imageops::FilterType::CatmullRom);
    to_rgba(resized.as_raw(), P::CHANNEL_COUNT.into(), out);
}

/// `pixels` (RGBA, or RGB with `channels` 3) into `out` as RGBA, an RGB pixel opaque.
pub fn to_rgba(pixels: &[u8], channels: usize, out: &mut [u8]) {
    if channels == 4 {
        out.copy_from_slice(pixels);
        return;
    }
    for (o, p) in out
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(pixels.as_chunks::<3>().0)
    {
        *o = [p[0], p[1], p[2], 255];
    }
}

/// image's Catmull-Rom resize with a pixel's four channels in one SSE2 register.
#[cfg(target_arch = "x86_64")]
mod sse2 {
    /// One output pixel's inputs along an axis: the first one and the normalised weights.
    struct Taps {
        left: usize,
        ws: Vec<f32>,
    }

    /// image 0.25's `bc_cubic_spline(x, 0.0, 0.5)`, the Catmull-Rom kernel.
    fn catmull_rom(x: f32) -> f32 {
        let (b, c) = (0.0f32, 0.5f32);
        let a = x.abs();
        let k = if a < 1.0 {
            (12.0 - 9.0 * b - 6.0 * c) * a.powi(3)
                + (-18.0 + 12.0 * b + 6.0 * c) * a.powi(2)
                + (6.0 - 2.0 * b)
        } else if a < 2.0 {
            (-b - 6.0 * c) * a.powi(3)
                + (6.0 * b + 30.0 * c) * a.powi(2)
                + (-12.0 * b - 48.0 * c) * a
                + (8.0 * b + 24.0 * c)
        } else {
            0.0
        };
        k / 6.0
    }

    /// The weights image's `vertical_sample` and `horizontal_sample` work out per output pixel.
    fn taps(from: u32, to: u32) -> Vec<Taps> {
        let ratio = from as f32 / to as f32;
        let sratio = if ratio < 1.0 { 1.0 } else { ratio };
        let support = 2.0 * sratio;
        (0..to)
            .map(|out| {
                let input = (out as f32 + 0.5) * ratio;
                let left = ((input - support).floor() as i64).clamp(0, i64::from(from) - 1);
                let right = ((input + support).ceil() as i64).clamp(left + 1, i64::from(from));
                let input = input - 0.5;
                let mut ws: Vec<f32> = (left..right)
                    .map(|i| catmull_rom((i as f32 - input) / sratio))
                    .collect();
                let sum = ws.iter().fold(0.0, |s, w| s + w);
                for w in &mut ws {
                    *w /= sum;
                }
                Taps {
                    left: left as usize,
                    ws,
                }
            })
            .collect()
    }

    /// `super::catmull_rom_into`: rows first, then columns, as image does.
    pub(super) fn catmull_rom_into<P: image::Pixel<Subpixel = u8> + 'static>(
        img: &super::Source<P>,
        rect: (u32, u32, u32, u32),
        nw: u32,
        nh: u32,
        out: &mut [u8],
    ) {
        let src = (img.as_raw().as_slice(), img.width() as usize);
        if P::CHANNEL_COUNT == 3 {
            resize::<3>(src, rect, nw, nh, out);
        } else {
            resize::<4>(src, rect, nw, nh, out);
        }
    }

    /// `catmull_rom_into` from `C` bytes a pixel (3: RGB, read with an opaque alpha).
    fn resize<const C: usize>(
        (src, stride): (&[u8], usize),
        (x, y, w, h): (u32, u32, u32, u32),
        nw: u32,
        nh: u32,
        out: &mut [u8],
    ) {
        let (x, y, row_len) = (x as usize, y as usize, w as usize * 4);
        let mut mid = vec![0f32; row_len * nh as usize];
        for (row, t) in mid.chunks_exact_mut(row_len).zip(taps(h, nh)) {
            let lines: Vec<&[[u8; C]]> = (0..t.ws.len())
                .map(|i| {
                    let at = ((y + t.left + i) * stride + x) * C;
                    src[at..at + w as usize * C].as_chunks::<C>().0
                })
                .collect();
            for (px, m) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let mut sum = Px::zero();
                for (line, &wt) in lines.iter().zip(&t.ws) {
                    let p = &line[px];
                    let p = [p[0], p[1], p[2], if C == 4 { p[C - 1] } else { 255 }];
                    sum = sum.add(Px::bytes(p).mul(wt));
                }
                sum.store(m);
            }
        }
        let cols = taps(w, nw);
        for (orow, row) in out
            .chunks_exact_mut(nw as usize * 4)
            .zip(mid.chunks_exact(row_len))
        {
            for (o, t) in orow.as_chunks_mut::<4>().0.iter_mut().zip(&cols) {
                let mut sum = Px::zero();
                for (i, &wt) in t.ws.iter().enumerate() {
                    let at = (t.left + i) * 4;
                    sum = sum.add(Px::floats(&row[at..at + 4]).mul(wt));
                }
                sum.to_bytes(o);
            }
        }
    }

    /// A pixel's four channels as f32 in one SSE2 register.
    #[derive(Clone, Copy)]
    pub(super) struct Px(std::arch::x86_64::__m128);

    impl Px {
        fn zero() -> Px {
            // SAFETY: SSE2 is part of x86-64's baseline; this only reads registers.
            Px(unsafe { std::arch::x86_64::_mm_setzero_ps() })
        }

        /// Four bytes, each to f32 (exact).
        fn bytes(p: [u8; 4]) -> Px {
            use std::arch::x86_64::*;
            let four = i32::from_le_bytes(p);
            // SAFETY: SSE2 is part of x86-64's baseline; these only work on registers.
            Px(unsafe {
                let zero = _mm_setzero_si128();
                let v = _mm_cvtsi32_si128(four);
                _mm_cvtepi32_ps(_mm_unpacklo_epi16(_mm_unpacklo_epi8(v, zero), zero))
            })
        }

        pub(super) fn floats(p: &[f32]) -> Px {
            // SAFETY: SSE2 is part of x86-64's baseline; this only works on registers.
            Px(unsafe { std::arch::x86_64::_mm_set_ps(p[3], p[2], p[1], p[0]) })
        }

        fn mul(self, w: f32) -> Px {
            use std::arch::x86_64::*;
            // SAFETY: SSE2 is part of x86-64's baseline; these only work on registers.
            Px(unsafe { _mm_mul_ps(self.0, _mm_set1_ps(w)) })
        }

        fn add(self, o: Px) -> Px {
            // SAFETY: SSE2 is part of x86-64's baseline; this only works on registers.
            Px(unsafe { std::arch::x86_64::_mm_add_ps(self.0, o.0) })
        }

        fn store(self, m: &mut [f32; 4]) {
            // SAFETY: SSE2 is part of x86-64's baseline; `m` holds the four floats written (an
            // unaligned store).
            unsafe { std::arch::x86_64::_mm_storeu_ps(m.as_mut_ptr(), self.0) }
        }

        /// Clamped to 0..=255 and rounded half away from zero, as image's `FloatNearest`: for
        /// these values that's a half less a hair added, then truncated (0.49999997 is the
        /// largest f32 below a half: adding a half would take it to 1).
        pub(super) fn to_bytes(self, o: &mut [u8; 4]) {
            use std::arch::x86_64::*;
            // SAFETY: SSE2 is part of x86-64's baseline; these only work on registers.
            let four = unsafe {
                let v = _mm_min_ps(_mm_max_ps(self.0, _mm_setzero_ps()), _mm_set1_ps(255.0));
                let i = _mm_cvttps_epi32(_mm_add_ps(v, _mm_set1_ps(0.499_999_97)));
                _mm_cvtsi128_si32(_mm_packus_epi16(_mm_packs_epi32(i, i), _mm_setzero_si128()))
            };
            *o = four.to_le_bytes();
        }
    }
}

#[cfg(all(test, target_arch = "x86_64"))]
#[cfg(all(test, target_arch = "x86_64"))]
mod tests {
    /// The SSE2 path and image's resize (`with_image`, the other architectures' path) give
    /// the same bytes: shrinking and growing, square and 16:9, a crop off the centre, with
    /// alpha and hard edges (the extremes clamp).
    #[test]
    fn the_bytes_are_images() {
        let (w, h) = (163, 121);
        let img = image::RgbaImage::from_fn(w, h, |x, y| {
            let n = (x * 7919 + y * 104_729) ^ (x * y);
            let hard = if (x / 9 + y / 7) % 2 == 0 { 255 } else { 0 };
            image::Rgba([(n % 256) as u8, hard, (x * 255 / w) as u8, (y * 2) as u8])
        });
        for (x, y, cw, ch, nw, nh) in [
            (21, 0, 121, 121, 60, 60),
            (21, 0, 121, 121, 40, 40),
            (21, 0, 121, 121, 300, 300),
            (0, 15, 163, 91, 96, 54),
            (0, 0, 163, 121, 163, 60),
            (3, 5, 100, 100, 101, 33),
        ] {
            let size = nw as usize * nh as usize * 4;
            let (mut theirs, mut ours) = (vec![0; size], vec![0; size]);
            super::with_image(&img, (x, y, cw, ch), nw, nh, &mut theirs);
            super::sse2::catmull_rom_into(&img, (x, y, cw, ch), nw, nh, &mut ours);
            assert!(theirs == ours, "{cw}x{ch} at {x},{y} to {nw}x{nh}");
            // RGB, read with an opaque alpha: the bytes of its RGBA copy, both paths.
            let rgb = image::DynamicImage::ImageRgba8(img.clone()).into_rgb8();
            let opaque = image::DynamicImage::ImageRgb8(rgb.clone()).into_rgba8();
            super::sse2::catmull_rom_into(&opaque, (x, y, cw, ch), nw, nh, &mut theirs);
            super::sse2::catmull_rom_into(&rgb, (x, y, cw, ch), nw, nh, &mut ours);
            assert!(theirs == ours, "RGB {cw}x{ch} at {x},{y} to {nw}x{nh}");
            super::with_image(&rgb, (x, y, cw, ch), nw, nh, &mut ours);
            assert!(
                theirs == ours,
                "image's RGB {cw}x{ch} at {x},{y} to {nw}x{nh}"
            );
        }
    }

    /// The rounding is f32::round (clamped to 0..=255) for the floats round every whole
    /// number and every half from -1 to 256. (Every float from 0 to 255 was checked once.)
    #[test]
    fn the_rounding_is_rounds() {
        let mut out = [0u8; 4];
        for k in -2..=512 {
            let at = (k as f32 / 2.0).to_bits();
            for bits in at.saturating_sub(300)..=at + 300 {
                let v = f32::from_bits(bits);
                super::sse2::Px::floats(&[v; 4]).to_bytes(&mut out);
                assert_eq!(out[0], v.clamp(0.0, 255.0).round() as u8, "{v}");
            }
        }
    }
}
