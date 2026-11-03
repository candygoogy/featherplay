//! The cover accent, the maths: one colour from a cover, then the shades the UI needs,
//! each clamped so text and the accent keep WCAG AA contrast on every surface they sit on.
//!
//! Works in OKLCH so lightness can move without shifting the hue. Pure; runs off the UI
//! thread.

use image::RgbaImage;
use image::imageops::FilterType;

pub type Rgb = [f32; 3];

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Lch {
    pub l: f32,
    pub c: f32,
    /// Radians.
    pub h: f32,
}

/// Below this chroma the art counts as neutral (grey, black and white) and the accent falls
/// back to red.
pub const NEUTRAL_CHROMA: f32 = 0.03;

/// The theme's fixed colours (`ui/theme.slint`).
pub struct Ground {
    pub ground: Rgb,
    /// The other resting surfaces the accent and text sit on: the plain bar and panels,
    /// raised cards.
    pub surfaces: [Rgb; 2],
    pub text1: Rgb,
    /// Text and glyphs on an accent fill (`Theme.on-accent`).
    pub on_accent: Rgb,
    /// The fallback accent, the logo's red.
    pub accent: Rgb,
}

pub const DARK: Ground = Ground {
    ground: hex(0x0F0F0F),
    surfaces: [hex(0x181818), hex(0x212121)],
    text1: hex(0xFFFFFF),
    on_accent: hex(0x0F0F0F),
    accent: hex(0xFF3B55),
};
pub const LIGHT: Ground = Ground {
    ground: hex(0xFFFFFF),
    surfaces: [hex(0xF7F7F7), hex(0xF2F2F2)],
    text1: hex(0x0F0F0F),
    on_accent: hex(0xFFFFFF),
    accent: hex(0xCC1036),
};

/// The accent's minimum contrast against what it sits on and what sits on it: AA for text,
/// though it only marks and fills.
pub const ACCENT_MIN: f32 = 4.5;

pub const fn hex(v: u32) -> Rgb {
    [
        ((v >> 16) & 0xFF) as f32 / 255.0,
        ((v >> 8) & 0xFF) as f32 / 255.0,
        (v & 0xFF) as f32 / 255.0,
    ]
}

fn to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn from_linear(c: f32) -> f32 {
    if c <= 0.003_130_8 {
        c * 12.92
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

fn srgb_to_oklab([r, g, b]: Rgb) -> [f32; 3] {
    let (r, g, b) = (to_linear(r), to_linear(g), to_linear(b));
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

/// Linear-light sRGB, possibly out of gamut.
fn oklab_to_linear([l, a, b]: [f32; 3]) -> [f32; 3] {
    let l_ = (l + 0.396_337_78 * a + 0.215_803_76 * b).powi(3);
    let m_ = (l - 0.105_561_346 * a - 0.063_854_17 * b).powi(3);
    let s_ = (l - 0.089_484_18 * a - 1.291_485_5 * b).powi(3);
    [
        4.076_741_7 * l_ - 3.307_711_6 * m_ + 0.230_969_94 * s_,
        -1.268_438 * l_ + 2.609_757_4 * m_ - 0.341_319_38 * s_,
        -0.004_196_086_3 * l_ - 0.703_418_6 * m_ + 1.707_614_7 * s_,
    ]
}

/// OKLCH to sRGB, reducing chroma until the colour is inside the gamut.
pub fn to_srgb(mut c: Lch) -> Rgb {
    loop {
        let lin = oklab_to_linear([c.l, c.c * c.h.cos(), c.c * c.h.sin()]);
        if lin.iter().all(|v| (-0.0005..=1.0005).contains(v)) || c.c <= 0.0 {
            return lin.map(|v| from_linear(v.clamp(0.0, 1.0)));
        }
        c.c = (c.c - 0.002).max(0.0);
    }
}

/// sRGB to OKLCH.
pub fn to_lch(rgb: Rgb) -> Lch {
    let [l, a, b] = srgb_to_oklab(rgb);
    Lch {
        l,
        c: a.hypot(b),
        h: b.atan2(a),
    }
}

fn luminance([r, g, b]: Rgb) -> f32 {
    0.2126 * to_linear(r) + 0.7152 * to_linear(g) + 0.0722 * to_linear(b)
}

/// The WCAG contrast ratio, 1 to 21.
pub fn contrast(a: Rgb, b: Rgb) -> f32 {
    let (la, lb) = (luminance(a), luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// The cover's most prominent colour: a chroma-weighted hue histogram on a 48 px
/// thumbnail. A neutral (chroma 0) when the art has almost no colour.
pub fn dominant(cover: &RgbaImage) -> Lch {
    let small = image::imageops::resize(cover, 48, 48, FilterType::Triangle);
    const BINS: usize = 36;
    let mut weight = [0f32; BINS];
    let mut px = Vec::with_capacity(48 * 48);
    for p in small.pixels() {
        let lab = srgb_to_oklab([
            p[0] as f32 / 255.0,
            p[1] as f32 / 255.0,
            p[2] as f32 / 255.0,
        ]);
        let c = lab[1].hypot(lab[2]);
        let h = lab[2].atan2(lab[1]).rem_euclid(std::f32::consts::TAU);
        let bin = ((h / std::f32::consts::TAU) * BINS as f32) as usize % BINS;
        // Very dark or very light pixels carry little usable hue.
        if c > 0.03 && lab[0] > 0.18 && lab[0] < 0.95 {
            weight[bin] += c;
        }
        px.push((lab, c, bin));
    }
    let total: f32 = weight.iter().sum();
    // Less than ~1 % of the area in a clear colour: treat the art as neutral.
    if total < 0.0015 * px.len() as f32 {
        let l = px.iter().map(|p| p.0[0]).sum::<f32>() / px.len().max(1) as f32;
        return Lch { l, c: 0.0, h: 0.0 };
    }
    let smooth = |i: usize| weight[(i + BINS - 1) % BINS] + weight[i] + weight[(i + 1) % BINS];
    let best = (0..BINS)
        .max_by(|&a, &b| smooth(a).total_cmp(&smooth(b)))
        .unwrap_or(0);
    let near = |bin: usize| {
        let d = (bin as i32 - best as i32).rem_euclid(BINS as i32);
        d <= 1 || d == BINS as i32 - 1
    };
    let (mut l, mut a, mut b, mut w) = (0f32, 0f32, 0f32, 0f32);
    for (lab, c, bin) in &px {
        if *c > 0.03 && near(*bin) {
            l += lab[0] * c;
            a += lab[1] * c;
            b += lab[2] * c;
            w += c;
        }
    }
    if w <= 0.0 {
        return Lch {
            l: 0.5,
            c: 0.0,
            h: 0.0,
        };
    }
    let (a, b) = (a / w, b / w);
    Lch {
        l: l / w,
        c: a.hypot(b),
        h: b.atan2(a),
    }
}

/// The colours the cover accent sets for one theme.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Palette {
    pub accent: Rgb,
    /// The player bar's tint.
    pub bar: Rgb,
    pub text2: Rgb,
}

/// Moves lightness step by step until every pair passes `min` contrast. The steps reach
/// black or white, where every pair here passes.
fn clamp(mut c: Lch, step: f32, against: &[Rgb], min: f32) -> Rgb {
    for _ in 0..120 {
        let rgb = to_srgb(c);
        if against_all(rgb, against, min) {
            return rgb;
        }
        c.l = (c.l + step).clamp(0.0, 1.0);
        if c.l == 0.0 || c.l == 1.0 {
            // At the end of the range: drop the chroma too, so the extreme is reached.
            c.c = 0.0;
        }
    }
    to_srgb(c)
}

/// How strong the cover's wash is at the top of the now-playing stage and of album and
/// playlist headers, per theme.
pub const WASH_ALPHA_DARK: f32 = 0.55;
pub const WASH_ALPHA_LIGHT: f32 = 0.36;

/// The cover's wash for each theme, opaque, to lay over the ground at the alphas above. Low
/// chroma and a mid lightness, so `text1` keeps far more than AA on it (checked in the
/// tests). Grey art washes grey.
pub fn washes(seed: Lch) -> (Rgb, Rgb) {
    let c = if seed.c < NEUTRAL_CHROMA { 0.0 } else { seed.c };
    let lch = |l: f32, cc: f32| Lch {
        l,
        c: cc,
        h: seed.h,
    };
    (
        to_srgb(lch(0.42, c.min(0.12))),
        to_srgb(lch(0.80, c.min(0.09))),
    )
}

/// `top` laid over `ground` at `alpha`, as the renderer blends it.
pub fn over(top: Rgb, alpha: f32, ground: Rgb) -> Rgb {
    std::array::from_fn(|i| top[i] * alpha + ground[i] * (1.0 - alpha))
}

/// The wash at its strongest, over the ground.
pub fn wash_on_ground(seed: Lch, dark: bool) -> Rgb {
    let (d, l) = washes(seed);
    if dark {
        over(d, WASH_ALPHA_DARK, DARK.ground)
    } else {
        over(l, WASH_ALPHA_LIGHT, LIGHT.ground)
    }
}

/// The palette for `seed` on the dark or light theme, or `None` for neutral art, which
/// keeps the plain colours (the red accent, the grey bar).
///
/// The accent is bold: the cover's hue with its chroma lifted, so muted art still gives a
/// clear colour, light on dark and deep on light, then clamped to `ACCENT_MIN` against
/// every surface it marks (the ground, the tinted bar, panels, cards, the now-playing
/// wash). Text on it is black on dark and white on light, which the same clamp keeps AA.
pub fn palette(seed: Lch, dark: bool) -> Option<Palette> {
    if seed.c < NEUTRAL_CHROMA {
        return None;
    }
    let (h, c) = (seed.h, seed.c);
    let g = if dark { &DARK } else { &LIGHT };
    let lch = |l: f32, cc: f32| Lch { l, c: cc, h };

    // The bar's tint: low chroma, so the art leads and text stays readable.
    let bar = if dark {
        clamp(lch(0.23, (c * 0.5).min(0.04)), -0.01, &[g.text1], 7.0)
    } else {
        clamp(lch(0.96, (c * 0.3).min(0.025)), 0.01, &[g.text1], 7.0)
    };
    let [panel, card] = g.surfaces;
    let surfaces = [g.ground, bar, panel, card, wash_on_ground(seed, dark)];
    // Secondary text tinted from the hue, AA on every surface it can sit on.
    let text2 = if dark {
        clamp(lch(0.72, (c * 0.3).min(0.035)), 0.01, &surfaces, 4.5)
    } else {
        clamp(lch(0.47, (c * 0.3).min(0.035)), -0.01, &surfaces, 4.5)
    };
    let chroma = (c * 1.8).clamp(0.12, 0.20);
    let accent = if dark {
        clamp(lch(0.70, chroma), 0.01, &surfaces, ACCENT_MIN)
    } else {
        clamp(lch(0.50, chroma), -0.01, &surfaces, ACCENT_MIN)
    };
    Some(Palette { accent, bar, text2 })
}

/// `c` with only its lightness moved, as little as needed to keep `ACCENT_MIN` against
/// the theme's resting surfaces (and so `on_accent` on it): lighter on dark, deeper on
/// light. A fixed accent (the custom colour) goes through it. Checked after rounding to
/// 8 bits a channel, as it's stored and drawn.
pub fn readable(mut c: Lch, dark: bool) -> Rgb {
    let g = if dark { &DARK } else { &LIGHT };
    let [panel, card] = g.surfaces;
    let against = [g.ground, panel, card, g.on_accent];
    let step = if dark { 0.01 } else { -0.01 };
    for _ in 0..120 {
        let rgb = quantise(clamp(c, step, &against, ACCENT_MIN));
        if against_all(rgb, &against, ACCENT_MIN) {
            return rgb;
        }
        c = to_lch(rgb);
        c.l = (c.l + step).clamp(0.0, 1.0);
    }
    quantise(to_srgb(c))
}

/// `c` rounded to 8 bits a channel.
fn quantise(c: Rgb) -> Rgb {
    c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() / 255.0)
}

pub fn against_all(c: Rgb, surfaces: &[Rgb], min: f32) -> bool {
    surfaces.iter().all(|&s| contrast(c, s) >= min)
}

/// Both themes' palettes for a cover; `None` for neutral art.
pub fn palettes(cover: &RgbaImage) -> Option<(Palette, Palette)> {
    palettes_for(dominant(cover))
}

/// Both themes' palettes for a cover's colour (`dominant`); `None` for neutral art.
pub fn palettes_for(seed: Lch) -> Option<(Palette, Palette)> {
    Some((palette(seed, true)?, palette(seed, false)?))
}

/// The lyrics' ground and their three levels of text: the line playing, the lines to come
/// and the lines already sung.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LyricInk {
    /// What the lines sit on: the cover-colour panel.
    pub ground: Rgb,
    pub current: Rgb,
    pub next: Rgb,
    pub past: Rgb,
}

/// Contrast floors for the three levels. Every line is ≥ 24 px and heavy, WCAG's large
/// text, where AA is 3:1; the lines to read ahead keep body text's 4.5:1 all the same, and
/// the line playing 7:1.
pub const LYRIC_CURRENT: f32 = 7.0;
pub const LYRIC_NEXT: f32 = 4.5;
pub const LYRIC_PAST: f32 = 3.0;
/// The least step of perceived lightness (OKLCH `l`) from a sung line up to a line to come,
/// and from there up to the line playing, so the three read apart at a glance.
pub const LYRIC_STEP_PAST: f32 = 0.18;
pub const LYRIC_STEP_NEXT: f32 = 0.09;

/// White on the panel: deeper than `LYRIC_CURRENT` needs, which leaves room between 3:1 and
/// white for the three levels to sit clearly apart.
const LYRIC_PANEL_WHITE: f32 = 8.0;
/// The light theme's panel keeps this share of the dark one's chroma: a deep block in a light
/// app reads calmer muted.
const LYRIC_LIGHT_CHROMA: f32 = 0.6;

/// What the lyrics' panel takes its colour from: the cover, or for neutral art (and no
/// cover) the accent the app falls back to, red or the user's fixed colour, as the accent
/// itself does. A grey panel read as flat.
pub fn lyric_seed(cover: Lch, accent: Rgb) -> Lch {
    if cover.c < NEUTRAL_CHROMA {
        to_lch(accent)
    } else {
        cover
    }
}

/// A solid panel in the cover's colour with light text tinted from its hue, for the dark or
/// light theme. Deep enough for white to keep `LYRIC_PANEL_WHITE`; a neutral seed would give
/// a grey panel, which `lyric_seed` keeps grey art from. Sung lines are the dimmest tint that keeps `LYRIC_PAST`; the lines to come sit
/// `LYRIC_STEP_PAST` lighter, and white marks the line playing.
pub fn lyric_panel(seed: Lch, dark: bool) -> LyricInk {
    let white = hex(0xFFFFFF);
    let panel = |c: f32| {
        clamp(
            Lch {
                l: 0.46,
                c,
                h: seed.h,
            },
            -0.01,
            &[white],
            LYRIC_PANEL_WHITE,
        )
    };
    let neutral = seed.c < NEUTRAL_CHROMA;
    let deep = panel(if neutral {
        0.0
    } else {
        (seed.c * 1.4).clamp(0.07, 0.15)
    });
    // The light one from the dark one's chroma as drawn: deep panels lose some to the gamut.
    let ground = if dark {
        deep
    } else {
        panel(to_lch(deep).c * LYRIC_LIGHT_CHROMA)
    };
    let c = to_lch(ground).c;
    // Light text in the panel's hue.
    let tint = |l: f32, min: f32| {
        clamp(
            Lch {
                l,
                c: (c * 0.35).min(0.05),
                h: seed.h,
            },
            0.01,
            &[ground],
            min,
        )
    };
    let past = tint(to_lch(ground).l + 0.05, LYRIC_PAST);
    // A hair over the step, so rounding to 8 bits never lands under it.
    let next = tint(to_lch(past).l + LYRIC_STEP_PAST + 0.005, LYRIC_NEXT);
    LyricInk {
        ground,
        current: white,
        next,
        past,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seeds() -> Vec<Lch> {
        let mut out = Vec::new();
        for l in [0.05, 0.2, 0.4, 0.55, 0.7, 0.85, 0.98] {
            for c in [0.0, 0.02, 0.06, 0.12, 0.2, 0.32] {
                for deg in (0..360).step_by(20) {
                    out.push(Lch {
                        l,
                        c,
                        h: (deg as f32).to_radians(),
                    });
                }
            }
        }
        out
    }

    #[test]
    fn clamping_keeps_aa_on_every_surface() {
        for dark in [true, false] {
            let g = if dark { &DARK } else { &LIGHT };
            for seed in seeds() {
                let Some(p) = palette(seed, dark) else {
                    continue;
                };
                let [panel, card] = g.surfaces;
                for (name, surface) in [
                    ("ground", g.ground),
                    ("bar", p.bar),
                    ("panel", panel),
                    ("card", card),
                    ("wash", wash_on_ground(seed, dark)),
                ] {
                    let ctx = format!("{seed:?} dark={dark} on {name}");
                    assert!(contrast(p.accent, surface) >= ACCENT_MIN, "accent {ctx}");
                    assert!(contrast(p.text2, surface) >= 4.5, "text2 {ctx}");
                    assert!(contrast(g.text1, surface) >= 7.0, "text1 {ctx}");
                }
                // The play button's glyph and the selected chip's label, on the accent.
                let on = contrast(g.on_accent, p.accent);
                assert!(on >= 4.5, "on-accent {seed:?} dark={dark}: {on}");
            }
        }
    }

    #[test]
    fn colourful_art_gives_a_bold_accent() {
        // Muted art still gives a clear colour, not a grey.
        for dark in [true, false] {
            for seed in seeds().into_iter().filter(|s| s.c >= 0.06) {
                let p = palette(seed, dark).expect("colourful");
                let got = to_lch(p.accent);
                assert!(got.c >= 0.08, "{seed:?} dark={dark}: {got:?}");
            }
        }
    }

    #[test]
    fn grey_art_falls_back_to_red() {
        let grey = RgbaImage::from_pixel(64, 64, image::Rgba([128, 128, 128, 255]));
        assert_eq!(palettes(&grey), None);
        let black = RgbaImage::from_pixel(64, 64, image::Rgba([4, 4, 4, 255]));
        assert_eq!(palettes(&black), None);
        // Faintly warm paper counts as grey too.
        let paper = RgbaImage::from_pixel(64, 64, image::Rgba([236, 232, 226, 255]));
        assert_eq!(palettes(&paper), None);
    }

    #[test]
    fn a_red_cover_gives_a_red_accent() {
        let red = RgbaImage::from_pixel(64, 64, image::Rgba([200, 30, 40, 255]));
        let seed = dominant(&red);
        assert!(seed.c > 0.1, "{seed:?}");
        // OKLCH red sits near 25 degrees.
        let deg = seed.h.to_degrees().rem_euclid(360.0);
        assert!((10.0..40.0).contains(&deg), "{deg}");
        let (dark, light) = palettes(&red).expect("colourful");
        assert!(
            light.accent[0] > light.accent[2],
            "red over blue: {:?}",
            light.accent
        );
        assert!(
            dark.accent[0] > dark.accent[2],
            "red over blue: {:?}",
            dark.accent
        );
    }

    #[test]
    fn mostly_black_art_with_a_little_colour_still_finds_it() {
        let mut img = RgbaImage::from_pixel(64, 64, image::Rgba([8, 8, 10, 255]));
        for y in 20..44 {
            for x in 20..44 {
                img.put_pixel(x, y, image::Rgba([40, 90, 220, 255]));
            }
        }
        let seed = dominant(&img);
        let deg = seed.h.to_degrees().rem_euclid(360.0);
        assert!(
            seed.c > 0.1 && (240.0..280.0).contains(&deg),
            "{seed:?} {deg}"
        );
    }

    #[test]
    fn lyric_levels_keep_their_contrast_on_the_panel() {
        for dark in [true, false] {
            for seed in seeds() {
                let ink = lyric_panel(seed, dark);
                let what = format!("dark {dark} {seed:?}");
                let (c, n, p) = (
                    contrast(ink.current, ink.ground),
                    contrast(ink.next, ink.ground),
                    contrast(ink.past, ink.ground),
                );
                assert!(c >= LYRIC_CURRENT, "{what}: current {c}");
                assert!(n >= LYRIC_NEXT, "{what}: next {n}");
                assert!(p >= LYRIC_PAST, "{what}: past {p}");
            }
        }
    }

    /// The earlier levels' weakness: sung lines read almost like the ones to come. Now each
    /// level sits a clear step of perceived lightness from the next one up.
    #[test]
    fn sung_lines_are_clearly_dimmer_than_the_lines_to_come() {
        for dark in [true, false] {
            for seed in seeds() {
                let ink = lyric_panel(seed, dark);
                let what = format!("dark {dark} {seed:?}");
                let [c, n, p] = [ink.current, ink.next, ink.past].map(|x| to_lch(x).l);
                assert!(n - p >= LYRIC_STEP_PAST, "{what}: sung {p} vs to come {n}");
                assert!(
                    c - n >= LYRIC_STEP_NEXT,
                    "{what}: to come {n} vs current {c}"
                );
            }
        }
    }

    /// In the light theme the panel is a deep block in a light app: it keeps the cover's hue
    /// with less of its colour, so it doesn't shout.
    #[test]
    fn the_light_theme_panel_is_softer() {
        for seed in seeds().into_iter().filter(|s| s.c >= NEUTRAL_CHROMA) {
            let dark = to_lch(lyric_panel(seed, true).ground);
            let light = to_lch(lyric_panel(seed, false).ground);
            assert!(
                light.c <= dark.c * 0.75,
                "{seed:?}: light {} vs dark {}",
                light.c,
                dark.c
            );
        }
    }
}
