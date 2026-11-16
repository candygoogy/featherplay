// The first screen's composition (the study's, mockups/11-study), apart from the page so it can
// be tested. Two arrangements, the larger window wins:
// - wide: the headline top left; under the words a shelf of the six sleeves, three by two, its
//   foot level with the window's where there is room; the window from the middle of the
//   headline's second line, on the frame's right edge, as large as the room allows and whole;
// - tall: the words and the shelf side by side, the window beneath them, centred, as wide as the
//   frame where the height allows.
// The app stays readable: where the wide app's lyrics would fall under 13 px the window is the
// app's own narrow layout instead. Phones and small rooms (under 900 x 520, or where neither
// arrangement fits) stack in one column, by CSS alone (Stage.astro).

import { IDEAL } from "../demo/fit";

export type Box = { x: number; y: number; w: number; h: number };
/** The window's layout size in app px, its scale, and whether it is the narrow layout. */
export type WinFit = { w: number; h: number; s: number; narrow: boolean };
export type Text = {
  /** The headline's two lines' widths and its line height, the column's height and width, the
   *  gap between headline and column. */
  l1w: number;
  l2w: number;
  lh: number;
  colW: number;
  colH: number;
  colGap: number;
};
export type Plan = {
  kind: "wide" | "tall";
  /** The frame (the bar's docked row shares it). */
  fl: number;
  fw: number;
  /** Where the words start (top left of the headline), how wide they are. */
  text: { x: number; y: number; w: number };
  /** The room the window is fitted into (window.ts places it at its top right) and the fit. */
  slot: Box;
  win: Box;
  fit: WinFit;
  /** `off`: no room for sleeves of 64 px: the shelf steps aside, the rest stays composed. */
  shelf: Box & { cols: number; size: number; gap: number; off: boolean };
};

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));
export const BAR = 52;
/** The smallest lyrics the first screen shows of the wide app (its lines are 24 app px). */
export const LYRICS_MIN = 13;
const SLEEVE_MAX = 176;
const SLEEVE_MIN = 64;

/** The first screen's window in a box: the whole wide app where its lyrics stay at 13 px or
 *  more, else the app's narrow layout (about 680 app px tall, 480 to 720 wide; lyrics 20 to 22
 *  app px), never larger than its own size. */
export function fitFirst(box: { w: number; h: number }): WinFit {
  const s = Math.min(1, box.w / IDEAL.w, box.h / IDEAL.h);
  if (24 * s >= LYRICS_MIN) return { w: IDEAL.w, h: IDEAL.h, s, narrow: false };
  return narrowFit(box);
}
export function narrowFit(box: { w: number; h: number }): WinFit {
  let s = Math.min(1, box.h / 680, box.w / 480);
  const w = clamp(box.w / s, 480, 720);
  const h = clamp(box.h / s, 560, 860);
  s = Math.min(s, box.h / h, box.w / w);
  return { w: Math.round(w), h: Math.round(h), s, narrow: true };
}
/** The narrow layout's lyrics, page px. */
export const lyricsPx = (f: WinFit) => (f.narrow ? (f.w < 600 ? 20 : 22) : 24) * f.s;

/** Six sleeves in a box: three by two, else six in a row, as large as fits. */
export function shelfFor(w: number, h: number, gap: number): { cols: number; size: number } {
  const a = Math.min((w - 2 * gap) / 3, (h - gap) / 2);
  const b = Math.min((w - 5 * gap) / 6, h);
  return a >= b * 1.15 ? { cols: 3, size: Math.min(SLEEVE_MAX, a) } : { cols: 6, size: Math.min(SLEEVE_MAX, b) };
}

/** The frame on the first screen: 1,880 px at most, else 89 % of the width, else 20 px a side. */
export const frameOf = (vw: number) => {
  const fw = Math.min(1880, vw * 0.89, vw - 40);
  return { fl: (vw - fw) / 2, fw };
};

/** The composition for a viewport, or null where the page should stack (phones, small rooms). */
export function compose(vw: number, vh: number, t: Text): Plan | null {
  if (vw < 900 || vh < 520) return null;
  const { fl, fw } = frameOf(vw);
  const fr = fl + fw;
  const gap = Math.round(clamp(vw * 0.0105, 10, 18));
  const yMax = vh - Math.round(clamp(vh * 0.045, 16, 60));
  const gapX = Math.round(clamp(vw * 0.03, 36, 96));
  const textH = t.lh * 2 + t.colGap + t.colH;
  const plans: Plan[] = [];

  // Wide: the window from the middle of the headline's second line.
  {
    const top = BAR + Math.round(clamp(vh * 0.06, 22, 80));
    const leftW = Math.max(t.colW, t.l2w, fw * 0.27);
    const xw = fl + leftW + gapX;
    const yw = top + t.lh * 1.5;
    const slot = { x: xw, y: yw, w: fr - xw, h: yMax - yw };
    const fit = fitFirst(slot);
    const win = { x: fr - fit.w * fit.s, y: yw, w: fit.w * fit.s, h: fit.h * fit.s };
    const shelfTop = top + textH + Math.round(clamp(vh * 0.03, 16, 36));
    const sh = shelfFor(leftW, yMax - shelfTop, gap);
    const shW = sh.cols * sh.size + (sh.cols - 1) * gap;
    const shH = (6 / sh.cols) * sh.size + (6 / sh.cols - 1) * gap;
    // Its foot level with the window's where there is room, else right under the words.
    const sy = Math.max(shelfTop, Math.min(win.y + win.h - shH, yMax - shH));
    if (slot.w > 300)
      plans.push({ kind: "wide", fl, fw, text: { x: fl, y: top, w: leftW }, slot, win, fit, shelf: { x: fl, y: sy, w: shW, h: shH, cols: sh.cols, size: sh.size, gap, off: sh.size < SLEEVE_MIN } });
  }
  // Tall: the words and the shelf side by side, the window beneath at the frame's width.
  {
    const top = BAR + Math.round(clamp(vh * 0.05, 22, 72));
    const leftW = Math.max(t.l1w, t.colW);
    const shRoom = Math.min(fr - (fl + leftW + gapX), 640);
    const sh = shelfFor(shRoom, textH, gap);
    const shW = sh.cols * sh.size + (sh.cols - 1) * gap;
    const shH = (6 / sh.cols) * sh.size + (6 / sh.cols - 1) * gap;
    const yw = top + Math.max(textH, shH) + Math.round(clamp(vh * 0.035, 24, 48));
    const slot = { x: fl, y: yw, w: fw, h: yMax - yw };
    const fit = fitFirst(slot);
    // Centred in the frame.
    const win = { x: fl + (fw - fit.w * fit.s) / 2, y: yw, w: fit.w * fit.s, h: fit.h * fit.s };
    if (sh.size >= SLEEVE_MIN && slot.h > 200)
      plans.push({ kind: "tall", fl, fw, text: { x: fl, y: top, w: leftW }, slot, win, fit, shelf: { x: fr - shW, y: top + textH - shH, w: shW, h: shH, cols: sh.cols, size: sh.size, gap, off: false } });
  }
  // Of the readable ones: one with its shelf first, then the wide app over the narrow one,
  // then the larger window.
  const readable = plans.filter((p) => lyricsPx(p.fit) >= LYRICS_MIN);
  const rank = (p: Plan) => (p.shelf.off ? 0 : 2) + (p.fit.narrow ? 0 : 1);
  const best = readable.sort((a, b) => rank(b) - rank(a) || b.win.w * b.win.h - a.win.w * a.win.h)[0];
  if (!best) return null;
  // The whole composition a little above the middle of the room under the bar, when it is
  // shorter than the room: the eye starts high.
  const bottom = Math.max(best.win.y + best.win.h, best.shelf.off ? 0 : best.shelf.y + best.shelf.h);
  const free = vh - bottom - (best.text.y - BAR);
  const shift = free > 0 ? Math.max(0, Math.min(free * 0.4, yMax - bottom)) : 0;
  if (shift > 1) {
    best.text.y += shift;
    best.slot.y += shift;
    best.win.y += shift;
    best.shelf.y += shift;
  }
  return best;
}
