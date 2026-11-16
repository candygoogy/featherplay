// The desk: one sheet of frosted glass in the playing song's colour (src/components/Desk.astro).
// Each song gives four tones per theme by one rule, so any song behaves like the six: the
// cover's hue at fixed lightnesses, its chroma capped (yellows get 72 % of the cap: they read
// brighter). The study's recipe ("K2, clean", mockups/11-study/notes.md):
//   dark:  ground L 0.15 (C <= 0.04), deep 0.07 (<= 0.02), light 0.56 (<= 0.13), brightest 0.74
//          (<= 0.12); light theme: ground 0.95 (<= 0.022), deep 0.72 (<= 0.075), the light and the
//          brightest white.
// With the owner's change for the dark theme: a warm hue (reds, oranges, golds) turned dark
// reads as brown, so a warm song filled the screen with one brown while the blue and teal ones
// fell off to near-black on their own. By the cover's warmth (its hue, scaled by how much
// colour it has), the dark desk is deepened: the ground and the deep fall nearer to black and
// carry less colour, the broad light is cut back (to half at the most: some of the screen always
// carries the song) and the lit corner gets more of the brightest, so the colour sits in the
// corner and a little in the low rise. The light theme is unchanged.

export type Theme = "dark" | "light";
export type DeskTones = {
  /** The ground, the deep, the light and the brightest: CSS oklch() colours. */
  qg: string;
  qs: string;
  ql: string;
  qh: string;
  /** Opacities of the light, the brightest and the deep layers. */
  lit: number;
  hi: number;
  shade: number;
};

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));
const lin = (c: number) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);

/** OKLCH of a hex colour: [lightness 0..1, chroma, hue in degrees]. */
export function oklchOf(hex: string): [number, number, number] {
  const n = parseInt(hex.replace("#", "").slice(0, 6), 16);
  const [r, g, b] = [(n >> 16) & 255, (n >> 8) & 255, n & 255].map((v) => lin(v / 255));
  const l = Math.cbrt(0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b);
  const m = Math.cbrt(0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b);
  const s = Math.cbrt(0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b);
  const L = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
  const A = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
  const B = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
  return [L, Math.hypot(A, B), ((Math.atan2(B, A) * 180) / Math.PI + 360) % 360];
}

/** How warm a colour is, 0..1: full round orange (hue 65°), none from about 140° on either
 *  side, scaled down for a colour with little chroma (a grey isn't warm, whatever its hue). */
export function warmth(hex: string): number {
  const [, c, h] = oklchOf(hex);
  const byHue = clamp((Math.cos(((h - 65) * Math.PI) / 180) - 0.2) / 0.6, 0, 1);
  return byHue * clamp(c / 0.05, 0, 1);
}

const fmt = (L: number, C: number, h: number) => `oklch(${+L.toFixed(3)} ${+C.toFixed(3)} ${+h.toFixed(1)})`;

export function deskTones(hex: string, theme: Theme): DeskTones {
  const [, c, h] = oklchOf(hex);
  const yellow = h > 60 && h < 120 ? 0.72 : 1;
  const tone = (L: number, cMax: number, k = 1) => fmt(L, Math.min(c, cMax) * yellow * k, h);
  if (theme === "light")
    return { qg: tone(0.95, 0.022), qs: tone(0.72, 0.075), ql: "oklch(0.995 0.004 90)", qh: "oklch(1 0 0)", lit: 1, hi: 0.6, shade: 0.55 };
  const w = warmth(hex);
  return {
    qg: tone(0.15 - 0.06 * w, 0.04, 1 - 0.7 * w),
    qs: tone(0.07 - 0.035 * w, 0.02, 1 - 0.7 * w),
    ql: tone(0.56, 0.13),
    qh: tone(0.74, 0.12),
    lit: +(1 - 0.25 * w).toFixed(3),
    hi: +(0.6 + 0.25 * w).toFixed(3),
    shade: +(0.9 + 0.1 * w).toFixed(3),
  };
}
