// How featherplay's window fits a box on the page. The app lays itself out for the window's own
// size (replica.css, by container size), and the page shows that window scaled to fit, never
// past the box's edges:
// - the whole wide layout (1440 x 852, the app's size on a 1440 x 900 desktop) wherever it can
//   be shown with its lyrics at 16 px or more (scale 2/3 of the app's 24 px lines);
// - else a narrower layout at that scale, the app reflowing as a real window does, down to its
//   least size (480 x 600, the app's own minimum);
// - only then smaller text.
// Pure, so it can be tested for every size (tests/unit/fit.test.ts).

export type Size = { w: number; h: number };
export type Box = { x: number; y: number; w: number; h: number };
/** The app's layout size (its own px) and the scale it's drawn at. */
export type Fit = { w: number; h: number; s: number };
export type Align = "start" | "center" | "end";

export const IDEAL: Size = { w: 1440, h: 852 };
export const MIN: Size = { w: 480, h: 600 };
/** The app's lyric lines are 24 px; 16 px is the smallest the page shows them by choice. */
export const S_MIN = 16 / 24;

const clamp = (v: number, lo: number, hi: number) => Math.min(hi, Math.max(lo, v));

export function fitWindow(box: Size, opts: { want?: Size; ideal?: Size; min?: Size; sMin?: number } = {}): Fit {
  if (opts.want) {
    const { w, h } = opts.want;
    return { w, h, s: Math.min(1, box.w / w, box.h / h) };
  }
  const ideal = opts.ideal ?? IDEAL;
  const min = opts.min ?? MIN;
  const sMin = opts.sMin ?? S_MIN;
  const whole = Math.min(1, box.w / ideal.w, box.h / ideal.h);
  if (whole >= sMin) return { w: ideal.w, h: ideal.h, s: whole };
  const w = clamp(box.w / sMin, min.w, ideal.w);
  const h = clamp(box.h / sMin, min.h, ideal.h);
  return { w, h, s: Math.min(sMin, box.w / w, box.h / h) };
}

/** Where a fitted window sits in its box. */
export function placeIn(box: Box, f: Fit, ax: Align, ay: Align): { x: number; y: number } {
  const room = (a: Align, free: number) => (a === "start" ? 0 : a === "center" ? free / 2 : free);
  return { x: box.x + room(ax, box.w - f.w * f.s), y: box.y + room(ay, box.h - f.h * f.s) };
}

/** A rect moved (not resized) so it lies wholly inside the bounds; one too large sits at their top left. */
export function clampInto(r: Box, b: Box): { x: number; y: number } {
  return {
    x: Math.max(b.x, Math.min(r.x, b.x + b.w - r.w)),
    y: Math.max(b.y, Math.min(r.y, b.y + b.h - r.h)),
  };
}
