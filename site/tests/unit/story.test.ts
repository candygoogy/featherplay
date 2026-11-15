import { describe, expect, it } from "vitest";
import { HOLDS, ZONES, bezier, captionOpacity, phases, settleTarget, storyProgress } from "../../src/scripts/story-math";

describe("the story's progress", () => {
  it("runs 0 to 1 over the pinned distance, and stays at its ends", () => {
    expect(storyProgress(-50, 0, 2400)).toBe(0);
    expect(storyProgress(1200, 0, 2400)).toBe(0.5);
    expect(storyProgress(9000, 0, 2400)).toBe(1);
    expect(storyProgress(500, 100, 0)).toBe(0);
  });
});

describe("the three holds", () => {
  it("are still: nothing moves while the page rests in a hold's zone", () => {
    for (const [a, b] of ZONES) {
      const lo = phases(a);
      for (let p = a; p <= b; p += 0.01) expect(phases(p)).toEqual(lo);
    }
  });
  it("each show one finished picture: whole, narrow, in the tray", () => {
    expect(phases(HOLDS[0])).toEqual({ a: 1, b: 0, c: 0 });
    expect(phases(HOLDS[1])).toEqual({ a: 1, b: 1, c: 0 });
    expect(phases(HOLDS[2])).toEqual({ a: 1, b: 1, c: 1 });
    expect(phases(0)).toEqual({ a: 0, b: 0, c: 0 });
  });
  it("move forward only, each move from 0 to 1", () => {
    let prev = phases(0);
    for (let p = 0; p <= 1; p += 0.005) {
      const now = phases(p);
      for (const k of ["a", "b", "c"] as const) {
        expect(now[k]).toBeGreaterThanOrEqual(prev[k] - 1e-9);
        expect(now[k]).toBeLessThanOrEqual(1);
      }
      prev = now;
    }
  });
});

describe("settling", () => {
  it("finishes a move the way the page was going", () => {
    expect(settleTarget(0.4, 1)).toBe(HOLDS[1]);
    expect(settleTarget(0.4, -1)).toBe(HOLDS[0]);
    expect(settleTarget(0.66, 1)).toBe(HOLDS[2]);
  });
  it("goes back from a nudge just past a hold, forward from one just before the next", () => {
    expect(settleTarget(0.275, 1)).toBe(HOLDS[0]);
    expect(settleTarget(0.495, -1)).toBe(HOLDS[1]);
  });
  it("leaves the page alone in a hold's zone and at the story's ends", () => {
    expect(settleTarget(0.2, 1)).toBeNull();
    expect(settleTarget(0.55, -1)).toBeNull();
    expect(settleTarget(0, 1)).toBeNull();
    expect(settleTarget(1, -1)).toBeNull();
    expect(settleTarget(0.9, 1)).toBeNull();
  });
});

describe("the captions", () => {
  it("show one at a time, each fully in its own hold", () => {
    for (const [i, h] of HOLDS.entries()) {
      for (let j = 0; j < 3; j++) expect(captionOpacity(j, h)).toBe(i === j ? 1 : 0);
    }
    expect(captionOpacity(0, 0)).toBe(0);
  });
});

describe("the motion curves", () => {
  it("run from 0 to 1 through their middle", () => {
    const e = bezier(0.37, 0, 0.63, 1);
    expect(e(0)).toBe(0);
    expect(e(1)).toBe(1);
    expect(e(0.5)).toBeCloseTo(0.5, 3);
    expect(e(0.25)).toBeLessThan(0.25);
  });
});
