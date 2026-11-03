// Small colour helpers: sRGB mixing as CSS `color-mix(in srgb, …)` does it, and WCAG 2
// contrast. Used by the tests and by the glow.

type Rgb = [number, number, number];

export function parse(hex: string): Rgb {
  const h = hex.replace("#", "");
  const full = h.length === 3 ? [...h].map((c) => c + c).join("") : h.slice(0, 6);
  return [0, 2, 4].map((i) => parseInt(full.slice(i, i + 2), 16)) as Rgb;
}

const toHex = (rgb: Rgb) =>
  "#" + rgb.map((v) => Math.round(Math.min(255, Math.max(0, v))).toString(16).padStart(2, "0")).join("").toUpperCase();

/** `a` at `amount` (0–1) mixed into `b`, like `color-mix(in srgb, a amount, b)`. */
export function mix(a: string, b: string, amount: number): string {
  const [x, y] = [parse(a), parse(b)];
  return toHex(x.map((v, i) => v * amount + y[i] * (1 - amount)) as Rgb);
}

function luminance(hex: string): number {
  const [r, g, b] = parse(hex).map((v) => {
    const c = v / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

export function contrast(a: string, b: string): number {
  const [hi, lo] = [luminance(a), luminance(b)].sort((x, y) => y - x);
  return (hi + 0.05) / (lo + 0.05);
}

/** The lyrics panel and its three levels of text, as the app's `colour::lyric_panel`. */
export type LyricInks = {
  ground: string;
  current: string;
  /** The lines to come and the sung lines as drawn on the panel... */
  next: string;
  past: string;
  /** ...and as CSS colours: white at the alpha that draws them, so a line slid out past the
   *  panel's clipped edge isn't taken for opaque text on the page behind. */
  nextCss: string;
  pastCss: string;
};

/** White's contrast on the panel (the app's LYRIC_PANEL_WHITE). */
const PANEL_WHITE = 8;
/** The lines to come and the sung lines, against the panel (LYRIC_NEXT, LYRIC_PAST), each with
 *  a hair to spare so 8-bit rounding never lands under it. */
const NEXT = 4.55;
const PAST = 3.05;

/**
 * The lyrics panel for a cover colour: the colour mixed 60 % into the dark ground, deepened
 * until white keeps 8:1 on it; white marks the line playing; the lines to come are white
 * mixed into the panel until they reach 4.5:1, the sung ones until 3:1 (so both keep the
 * panel's hue, as the app's tints do).
 */
export function lyricInks(colour: string): LyricInks {
  let amount = 0.6;
  let ground = mix(colour, "#0F0F0F", amount);
  while (contrast("#FFFFFF", ground) < PANEL_WHITE && amount > 0) {
    amount = Math.max(0, amount - 0.01);
    ground = mix(colour, "#0F0F0F", amount);
  }
  const tint = (min: number) => {
    for (let a = 0; a <= 100; a++) {
      const c = mix("#FFFFFF", ground, a / 100);
      if (contrast(c, ground) >= min) return { c, css: `rgb(255 255 255 / ${a / 100})` };
    }
    return { c: "#FFFFFF", css: "#FFFFFF" };
  };
  const [next, past] = [tint(NEXT), tint(PAST)];
  return { ground, current: "#FFFFFF", next: next.c, past: past.c, nextCss: next.css, pastCss: past.css };
}
