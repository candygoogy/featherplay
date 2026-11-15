import { describe, expect, it } from "vitest";
import { IDEAL, MIN, S_MIN, clampInto, fitWindow, placeIn } from "../../src/demo/fit";

// Every box from a phone's sliver to a 4K screen, odd sizes included.
const sizes: number[] = [];
for (let v = 120; v <= 4000; v += 37) sizes.push(v);

describe("fitting the window into a box", () => {
  it("always fits inside the box, at any size", () => {
    for (const w of sizes) {
      for (const h of sizes) {
        const f = fitWindow({ w, h });
        expect(f.w * f.s, `${w}x${h}`).toBeLessThanOrEqual(w + 1e-6);
        expect(f.h * f.s, `${w}x${h}`).toBeLessThanOrEqual(h + 1e-6);
        expect(f.s).toBeGreaterThan(0);
        expect(f.s).toBeLessThanOrEqual(1);
        expect(f.w).toBeGreaterThanOrEqual(MIN.w);
        expect(f.w).toBeLessThanOrEqual(IDEAL.w);
        expect(f.h).toBeGreaterThanOrEqual(MIN.h);
        expect(f.h).toBeLessThanOrEqual(IDEAL.h);
      }
    }
  });
  it("shows the whole wide layout at the app's own size where there's room", () => {
    expect(fitWindow({ w: 2000, h: 1200 })).toEqual({ w: IDEAL.w, h: IDEAL.h, s: 1 });
  });
  it("scales the whole wide layout down while its lyrics stay at 16 px or more", () => {
    const f = fitWindow({ w: 1100, h: 700 });
    expect(f.w).toBe(IDEAL.w);
    expect(f.s).toBeGreaterThanOrEqual(S_MIN);
    expect(f.s).toBeCloseTo(Math.min(1100 / 1440, 700 / 852), 6);
  });
  it("lays the app out narrower before its text would shrink under 16 px", () => {
    const f = fitWindow({ w: 700, h: 600 });
    expect(f.s).toBeCloseTo(S_MIN, 6);
    expect(f.w).toBeCloseTo(700 / S_MIN, 6);
    expect(f.h).toBe(IDEAL.h); // 900 would do; the wide layout is never taller than its own
  });
  it("only shrinks the text once the app is at its least size", () => {
    const f = fitWindow({ w: 300, h: 300 });
    expect(f).toEqual({ w: MIN.w, h: MIN.h, s: 0.5 });
  });
  it("takes a wanted layout (the narrow window) and only scales it", () => {
    const f = fitWindow({ w: 1000, h: 500 }, { want: { w: 480, h: 660 } });
    expect(f.w).toBe(480);
    expect(f.h).toBe(660);
    expect(f.s).toBeCloseTo(500 / 660, 6);
  });
});

describe("placing and keeping the window in bounds", () => {
  it("places a fitted window in its box by alignment", () => {
    const box = { x: 100, y: 50, w: 1000, h: 600 };
    const f = { w: 1440, h: 852, s: 0.5 };
    expect(placeIn(box, f, "end", "end")).toEqual({ x: 100 + 1000 - 720, y: 50 + 600 - 426 });
    expect(placeIn(box, f, "center", "start")).toEqual({ x: 100 + (1000 - 720) / 2, y: 50 });
  });
  it("keeps a dragged window wholly inside the bounds", () => {
    const bounds = { x: 0, y: 52, w: 1280, h: 668 };
    expect(clampInto({ x: -300, y: 10, w: 800, h: 400 }, bounds)).toEqual({ x: 0, y: 52 });
    expect(clampInto({ x: 900, y: 600, w: 800, h: 400 }, bounds)).toEqual({ x: 480, y: 320 });
    expect(clampInto({ x: 100, y: 100, w: 800, h: 400 }, bounds)).toEqual({ x: 100, y: 100 });
  });
  it("puts a window larger than the bounds at their top left", () => {
    expect(clampInto({ x: 50, y: 300, w: 2000, h: 900 }, { x: 0, y: 52, w: 1280, h: 668 })).toEqual({ x: 0, y: 52 });
  });
});
