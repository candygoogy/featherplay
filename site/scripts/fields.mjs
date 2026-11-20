// Now playing's ground for each demo cover, as the app works it out (follow/backdrop.rs): the
// cover shrunk to 24 px, blurred wide, each part toned dark or near white in its own hue; from
// that the three bands down the page, the floating card's and the bar's tint, and the lyrics'
// three inks, each keeping its contrast floor on the field's worst pixel. Writes
// src/styles/fields.css, one rule per song (run by hand when a cover changes, then commit it):
//   node scripts/fields.mjs
// The covers are decoded by the test browser (Playwright's Chromium); the maths is here.
import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const ids = ["california-lullabye", "invisible-light", "she-dreams-in-blue", "watching-seabirds-soar", "origami", "in-this-moment"];
const out = fileURLToPath(new URL("../src/styles/fields.css", import.meta.url));

// ---- colour (follow/colour.rs) ----
const toLinear = (c) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
const fromLinear = (c) => (c <= 0.0031308 ? c * 12.92 : 1.055 * c ** (1 / 2.4) - 0.055);
function toLch([r, g, b]) {
  [r, g, b] = [r, g, b].map(toLinear);
  const l = Math.cbrt(0.41222146 * r + 0.53633255 * g + 0.051445995 * b);
  const m = Math.cbrt(0.2119035 * r + 0.6806995 * g + 0.10739696 * b);
  const s = Math.cbrt(0.08830246 * r + 0.28171885 * g + 0.6299787 * b);
  const L = 0.21045426 * l + 0.7936178 * m - 0.004072047 * s;
  const A = 1.9779985 * l - 2.4285922 * m + 0.4505937 * s;
  const B = 0.025904037 * l + 0.78277177 * m - 0.80867577 * s;
  return { l: L, c: Math.hypot(A, B), h: Math.atan2(B, A) };
}
function toSrgb({ l, c, h }) {
  for (;;) {
    const a = c * Math.cos(h);
    const b = c * Math.sin(h);
    const l_ = (l + 0.39633778 * a + 0.21580376 * b) ** 3;
    const m_ = (l - 0.105561346 * a - 0.06385417 * b) ** 3;
    const s_ = (l - 0.08948418 * a - 1.2914855 * b) ** 3;
    const lin = [4.0767417 * l_ - 3.3077116 * m_ + 0.23096994 * s_, -1.268438 * l_ + 2.6097574 * m_ - 0.34131938 * s_, -0.0041960863 * l_ - 0.7034186 * m_ + 1.7076147 * s_];
    if (lin.every((v) => v >= -0.0005 && v <= 1.0005) || c <= 0) return lin.map((v) => fromLinear(Math.min(1, Math.max(0, v))));
    c = Math.max(0, c - 0.002);
  }
}
const luminance = ([r, g, b]) => 0.2126 * toLinear(r) + 0.7152 * toLinear(g) + 0.0722 * toLinear(b);
function contrast(a, b) {
  const [x, y] = [luminance(a), luminance(b)];
  return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
}
const quantise = (c) => c.map((v) => Math.round(Math.min(1, Math.max(0, v)) * 255) / 255);
const over = (top, alpha, ground) => top.map((v, i) => v * alpha + ground[i] * (1 - alpha));
const mix = (a, b, t) => a.map((v, k) => v + (b[k] - v) * t);
function clampInk(c, step, against, min) {
  c = { ...c };
  for (let i = 0; i < 120; i++) {
    const rgb = toSrgb(c);
    if (against.every((s) => contrast(rgb, s) >= min)) return rgb;
    c.l = Math.min(1, Math.max(0, c.l + step));
    if (c.l === 0 || c.l === 1) c.c = 0;
  }
  return toSrgb(c);
}
const LYRIC_CURRENT = 7.0;
const LYRIC_NEXT = 4.5;
const LYRIC_PAST = 3.0;
const LYRIC_STEP_PAST = 0.18;
const LYRIC_STEP_NEXT = 0.09;
const TINT_HOVER = 0x12 / 255;
const TINT_PRESSED = 0x2b / 255;

// ---- the field (follow/backdrop.rs) ----
const SMALL = 24;
const GRID = 96;
const KEEP = 0.8;
const HEADROOM = 1.04;

/** The image crate's Triangle resize (vertical pass, then horizontal), on sRGB values. */
function resize(px, w, h, n) {
  const pass = (src, sw, sh, vertical) => {
    const len = vertical ? sh : sw;
    const ratio = len / n;
    const support = Math.max(1, ratio);
    const ow = vertical ? sw : n;
    const oh = vertical ? n : sh;
    const dst = new Float64Array(ow * oh * 3);
    for (let o = 0; o < n; o++) {
      const centre = (o + 0.5) * ratio;
      const from = Math.max(0, Math.floor(centre - support));
      const to = Math.min(len, Math.ceil(centre + support));
      const ws = [];
      let sum = 0;
      for (let j = from; j < to; j++) {
        const w0 = Math.max(0, 1 - Math.abs((j + 0.5 - centre) / support));
        ws.push(w0);
        sum += w0;
      }
      const other = vertical ? sw : sh;
      for (let k = 0; k < other; k++) {
        const acc = [0, 0, 0];
        ws.forEach((wt, i) => {
          const j = from + i;
          const at = (vertical ? j * sw + k : k * sw + j) * 3;
          for (let c = 0; c < 3; c++) acc[c] += src[at + c] * wt;
        });
        const dAt = (vertical ? o * ow + k : k * ow + o) * 3;
        for (let c = 0; c < 3; c++) dst[dAt + c] = acc[c] / sum;
      }
    }
    return dst;
  };
  const v = pass(px, w, h, true);
  const both = pass(v, w, n, false);
  const outPx = [];
  for (let i = 0; i < n * n; i++) outPx.push([both[i * 3], both[i * 3 + 1], both[i * 3 + 2]].map((x) => Math.min(255, Math.max(0, Math.round(x))) / 255));
  return outPx;
}

function blur(px, n, r, passes) {
  const line = new Array(n);
  for (let p = 0; p < passes; p++)
    for (const rows of [true, false])
      for (let k = 0; k < n; k++) {
        const at = (i) => (rows ? k * n + i : i * n + k);
        for (let i = 0; i < n; i++) {
          const sum = [0, 0, 0];
          for (let d = 0; d <= 2 * r; d++) {
            const q = px[at(Math.min(n - 1, Math.max(0, i + d - r)))];
            for (let c = 0; c < 3; c++) sum[c] += q[c];
          }
          line[i] = sum.map((s) => s / (2 * r + 1));
        }
        for (let i = 0; i < n; i++) px[at(i)] = line[i];
      }
}

function tone(p, dark) {
  const c = toLch(p);
  const l = Math.min(1, Math.max(0, c.l));
  return toSrgb(dark ? { l: 0.17 + 0.26 * l, c: Math.min(c.c, 0.1), h: c.h } : { l: 0.84 + 0.11 * l, c: Math.min(c.c * 0.6, 0.055), h: c.h });
}
function mean(small, y0, y1) {
  const sum = [0, 0, 0];
  let n = 0;
  for (let y = y0; y < y1; y++)
    for (let x = 2; x < SMALL - 2; x++) {
      for (let k = 0; k < 3; k++) sum[k] += small[y * SMALL + x][k];
      n++;
    }
  return sum.map((v) => v / n);
}
function stops(small, dark) {
  const low = toLch(mean(small, 16, 20));
  if (dark) {
    low.l = Math.max(0.13, low.l - 0.05);
    low.c = Math.min(low.c, 0.04);
  } else {
    low.l = Math.min(0.97, low.l + 0.03);
    low.c = Math.min(low.c, 0.02);
  }
  return [mean(small, 4, 10), mean(small, 10, 16), toSrgb(low)].map(quantise);
}
function grid(small) {
  const UP = GRID / SMALL;
  const taps = Array.from({ length: GRID }, (_, o) => {
    const u = Math.min(SMALL - 1, Math.max(0, (o + 0.5) / UP - 0.5));
    const i = Math.floor(u);
    return [i, Math.min(SMALL - 1, i + 1), u - i];
  });
  const g = new Array(GRID * GRID);
  taps.forEach(([y0, y1, fy], y) =>
    taps.forEach(([x0, x1, fx], x) => {
      const at = (r) => mix(small[r * SMALL + x0], small[r * SMALL + x1], fx);
      g[y * GRID + x] = mix(at(y0), at(y1), fy);
    }),
  );
  blur(g, GRID, 2, 3);
  return g;
}
function shown(g) {
  const n = GRID;
  const from = Math.max(0, Math.floor(((1 - KEEP) / 2) * n - 0.5));
  const to = Math.min(GRID - 1, Math.ceil(((1 + KEEP) / 2) * n - 0.5));
  const outPx = [];
  for (let y = from; y <= to; y++) for (let x = from; x <= to; x++) outPx.push(g[y * GRID + x]);
  return outPx;
}
function inks(worst, ground, hue, dark) {
  const black = [0, 0, 0];
  const under = dark ? [worst] : [worst, over(black, TINT_HOVER, worst), over(black, TINT_PRESSED, worst)];
  const step = dark ? 0.01 : -0.01;
  const ink = (l, c, min) => quantise(clampInk({ l, c, h: hue.h }, step, under, min * HEADROOM));
  const c = dark ? Math.min(hue.c * 0.35, 0.05) : Math.min(hue.c * 0.9, 0.06);
  const past = ink(dark ? toLch(worst).l + 0.05 : 0.58, c, LYRIC_PAST);
  const pastL = toLch(past).l;
  const nextL = dark ? pastL + LYRIC_STEP_PAST + 0.005 : Math.min(pastL - LYRIC_STEP_NEXT - 0.005, 0.45);
  return {
    ground,
    current: dark ? [1, 1, 1] : ink(0.16, Math.min(hue.c * 0.6, 0.035), LYRIC_CURRENT),
    next: ink(nextL, c, LYRIC_NEXT),
    past,
  };
}
function field(blurred, dark) {
  const small = blurred.map((p) => tone(p, dark));
  const ground = quantise(mean(small, 4, 20));
  const m = toLch(ground);
  const card = quantise(toSrgb(dark ? { l: 0.215, c: Math.min(m.c, 0.035), h: m.h } : { l: 0.975, c: Math.min(m.c, 0.018), h: m.h }));
  const st = stops(small, dark);
  const bands = [];
  for (let k = 0; k <= 16; k++) bands.push(mix(st[0], st[1], k / 16), mix(st[1], st[2], k / 16));
  const sign = dark ? 1 : -1;
  const worst = [...shown(grid(small)), ...bands].reduce((w, p) => (sign * luminance(p) > sign * luminance(w) ? p : w), ground);
  return { stops: st, card, ink: inks(worst, ground, m, dark) };
}

const hex = (c) => "#" + c.map((v) => Math.round(v * 255).toString(16).padStart(2, "0")).join("");

// ---- decode the covers ----
const browser = await chromium.launch();
const page = await browser.newPage();
const rules = [];
for (const id of ids) {
  const url = new URL(`../public/covers/${id}.webp`, import.meta.url);
  const b64 = (await import("node:fs")).readFileSync(url).toString("base64");
  const { w, h, data } = await page.evaluate(async (src) => {
    const img = new Image();
    img.src = src;
    await img.decode();
    const c = document.createElement("canvas");
    c.width = img.naturalWidth;
    c.height = img.naturalHeight;
    const g = c.getContext("2d");
    g.drawImage(img, 0, 0);
    return { w: c.width, h: c.height, data: [...g.getImageData(0, 0, c.width, c.height).data] };
  }, `data:image/webp;base64,${b64}`);
  const rgb = new Float64Array(w * h * 3);
  for (let i = 0; i < w * h; i++) for (let c = 0; c < 3; c++) rgb[i * 3 + c] = data[i * 4 + c];
  const small = resize(rgb, w, h, SMALL);
  blur(small, SMALL, 3, 3);
  const [d, l] = [field(small, true), field(small, false)];
  const vars = (f, t) =>
    [
      `--f${t}-top: ${hex(f.stops[0])};`,
      `--f${t}-mid: ${hex(f.stops[1])};`,
      `--f${t}-low: ${hex(f.stops[2])};`,
      `--f${t}-card: ${hex(f.card)};`,
      `--f${t}-ground: ${hex(f.ink.ground)};`,
      `--f${t}-ink1: ${hex(f.ink.current)};`,
      `--f${t}-ink2: ${hex(f.ink.next)};`,
      `--f${t}-ink3: ${hex(f.ink.past)};`,
    ].join(" ");
  rules.push(`[data-song="${id}"] {\n  ${vars(d, "d")}\n  ${vars(l, "l")}\n}`);
}
await browser.close();

writeFileSync(
  out,
  `/* Written by scripts/fields.mjs: Now playing's field for each demo cover, as the app's
   follow/backdrop.rs makes it. Per theme (d dark, l light): the bands at the page's top, 55 %
   down and foot; the Up next card's and the bar's tint; the field's mean (the side tab's lyrics
   card); the lyrics' inks (playing, to come, sung). replica.css and pieces.css mix each pair
   on --light. The three --w- are the bands as painted, registered so a song change fades them
   (they paint a gradient). */
@property --w-top { syntax: "<color>"; inherits: false; initial-value: transparent; }
@property --w-mid { syntax: "<color>"; inherits: false; initial-value: transparent; }
@property --w-low { syntax: "<color>"; inherits: false; initial-value: transparent; }
${rules.join("\n")}
`,
);
console.log(`wrote ${out}`);
