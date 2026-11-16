import { describe, expect, it } from "vitest";
import { compose, fitFirst, frameOf, lyricsPx, narrowFit, shelfFor, LYRICS_MIN } from "../../src/scripts/first-math";

// The first screen's composition (src/scripts/first-math.ts), the study's: the words top left,
// the shelf under them, the window from the middle of the headline's second line on the frame's
// right edge; or the words and the shelf side by side over a full-width window.

/** The headline and column as measured at about 92 px (2560 x 1270). */
const text = (fs: number) => ({ l1w: fs * 10.2, l2w: fs * 5.4, lh: fs * 0.94, colW: 420, colH: 220, colGap: 28 });
const fsFor = (vw: number, vh: number) => Math.min(92, Math.max(54, Math.min(vw * 0.056, vh * 0.095)));
const SIZES: [number, number][] = [
  [2560, 1270],
  [2560, 1440],
  [1920, 1080],
  [1600, 900],
  [1440, 900],
  [1366, 768],
  [1280, 720],
  [1280, 1270],
  [1024, 768],
  [3840, 2160],
  [3840, 600],
  [900, 520],
  [1000, 1400],
];

describe("the window on the first screen", () => {
  it("is the whole wide app wherever its lyrics stay at 13 px", () => {
    const f = fitFirst({ w: 1100, h: 700 });
    expect(f.narrow).toBe(false);
    expect(24 * f.s).toBeGreaterThanOrEqual(LYRICS_MIN);
  });
  it("is the app's narrow layout where they would fall under 13 px, its lyrics larger", () => {
    const f = fitFirst({ w: 700, h: 480 });
    expect(f.narrow).toBe(true);
    expect(f.w).toBeGreaterThanOrEqual(480);
    expect(f.w).toBeLessThanOrEqual(720);
    expect(lyricsPx(f)).toBeGreaterThan(24 * Math.min(700 / 1440, 480 / 852));
  });
  it("never grows past its own size, and always fits its box", () => {
    for (const [w, h] of [[300, 300], [480, 600], [700, 2000], [3000, 3000]] as const) {
      const f = narrowFit({ w, h });
      expect(f.s).toBeLessThanOrEqual(1);
      expect(f.w * f.s).toBeLessThanOrEqual(w + 0.5);
      expect(f.h * f.s).toBeLessThanOrEqual(h + 0.5);
    }
  });
});

describe("the shelf", () => {
  it("is three by two where the room is about square, six in a row where it is long and low", () => {
    expect(shelfFor(560, 380, 14).cols).toBe(3);
    expect(shelfFor(1100, 150, 14).cols).toBe(6);
  });
  it("is never larger than 176 px a sleeve", () => {
    expect(shelfFor(2000, 2000, 18).size).toBe(176);
  });
});

describe("the composition", () => {
  it("at 2560 x 1270: wide, the window's top at the middle of the headline's second line, on the frame's right edge, whole", () => {
    const t = text(92);
    const p = compose(2560, 1270, t)!;
    expect(p.kind).toBe("wide");
    expect(p.win.y - p.text.y).toBeCloseTo(t.lh * 1.5, 3);
    const { fl, fw } = frameOf(2560);
    expect(p.win.x + p.win.w).toBeCloseTo(fl + fw, 3);
    expect(p.fit.narrow).toBe(false);
    // Larger than the 1,102 px window the old first screen had there.
    expect(p.win.w).toBeGreaterThan(1150);
    // The shelf under the words, its foot level with the window's.
    expect(p.shelf.x).toBeCloseTo(fl, 3);
    expect(p.shelf.y + p.shelf.h).toBeCloseTo(p.win.y + p.win.h, 0);
    expect(p.shelf.cols).toBe(3);
  });
  it("in a tall window (half a 2560 screen): the words and the shelf side by side, the window beneath, centred and nearly the frame's width", () => {
    const p = compose(1280, 1270, text(fsFor(1280, 1270)))!;
    expect(p.kind).toBe("tall");
    expect(p.shelf.x).toBeGreaterThan(p.text.x + p.text.w);
    expect(p.win.y).toBeGreaterThanOrEqual(p.shelf.y + p.shelf.h);
    expect(p.win.x - p.fl).toBeCloseTo(p.fl + p.fw - (p.win.x + p.win.w), 1);
    expect(p.win.w).toBeGreaterThan(p.fw * 0.85);
  });
  it("at 1280 x 720: the app's narrow layout, its lyrics at 13 px or more", () => {
    const p = compose(1280, 720, text(fsFor(1280, 720)))!;
    expect(p.fit.narrow).toBe(true);
    expect(lyricsPx(p.fit)).toBeGreaterThanOrEqual(LYRICS_MIN);
  });
  it("in a short wide room keeps the composition and leaves the shelf out where its sleeves wouldn't reach 64 px", () => {
    const p = compose(1440, 600, text(fsFor(1440, 600)))!;
    expect(p).not.toBeNull();
    expect(p.kind).toBe("wide");
    expect(p.shelf.off).toBe(true);
    expect(compose(2560, 1270, text(92))!.shelf.off).toBe(false);
  });
  it("stacks on a phone and in small rooms", () => {
    expect(compose(390, 844, text(40))).toBeNull();
    expect(compose(899, 900, text(54))).toBeNull();
    expect(compose(1440, 519, text(54))).toBeNull();
    expect(compose(899, 900, text(54))).toBeNull();
  });
  it("at every size it composes: the window and the shelf inside the viewport, apart from each other and from the words", () => {
    for (const [vw, vh] of SIZES) {
      const t = text(fsFor(vw, vh));
      const p = compose(vw, vh, t);
      if (!p) continue;
      const at = `${vw}x${vh}`;
      for (const b of p.shelf.off ? [p.win] : [p.win, p.shelf]) {
        expect(b.x, at).toBeGreaterThanOrEqual(0);
        expect(b.x + b.w, at).toBeLessThanOrEqual(vw + 0.5);
        expect(b.y, at).toBeGreaterThanOrEqual(52);
        expect(b.y + b.h, at).toBeLessThanOrEqual(vh + 0.5);
      }
      const apart = (a: { x: number; y: number; w: number; h: number }, b: typeof a) => a.x + a.w <= b.x + 0.5 || b.x + b.w <= a.x + 0.5 || a.y + a.h <= b.y + 0.5 || b.y + b.h <= a.y + 0.5;
      if (!p.shelf.off) expect(apart(p.win, p.shelf), `${at} window and shelf`).toBe(true);
      // The words (below the headline's first line, which may reach over the window's column).
      const words = { x: p.text.x, y: p.text.y + t.lh, w: Math.max(t.l2w, t.colW), h: t.lh + t.colGap + t.colH };
      expect(apart(p.win, words), `${at} window and words`).toBe(true);
      if (!p.shelf.off) expect(apart(p.shelf, words), `${at} shelf and words`).toBe(true);
      expect(lyricsPx(p.fit), at).toBeGreaterThanOrEqual(LYRICS_MIN);
    }
  });
});
