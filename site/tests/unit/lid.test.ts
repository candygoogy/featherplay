import { describe, expect, it } from "vitest";
import { FLOURISH_EASE, FLOURISH_MS, LEAN, heroMoment, hinge, lidFlourish, startScale } from "../../src/scripts/lid";

describe("the hero's scroll moment", () => {
  it("starts small, low and leaning back, with the name in full", () => {
    const m = heroMoment(0);
    expect(m.zoom).toBe(0);
    expect(m.lean).toBe(LEAN);
    expect(m.lean).toBeGreaterThan(6);
    expect(m.head).toBe(0);
    expect(m.settled).toBe(false);
  });

  it("ends settled: full size, upright, the name gone", () => {
    const m = heroMoment(1);
    expect(m).toEqual({ zoom: 1, lean: 0, head: 1, settled: true });
  });

  it("rises, straightens and blurs the name away without a jump or a step back", () => {
    let prev = heroMoment(0);
    for (let i = 1; i <= 400; i++) {
      const m = heroMoment(i / 400);
      expect(m.zoom).toBeGreaterThanOrEqual(prev.zoom);
      expect(m.lean).toBeLessThanOrEqual(prev.lean);
      expect(m.head).toBeGreaterThanOrEqual(prev.head);
      expect(m.zoom - prev.zoom).toBeLessThan(0.02);
      expect(m.head - prev.head).toBeLessThan(0.03);
      prev = m;
    }
  });

  it("the name has gone before the laptop has finished settling", () => {
    const settle = Array.from({ length: 101 }, (_, i) => i / 100).find((p) => heroMoment(p).settled)!;
    const gone = Array.from({ length: 101 }, (_, i) => i / 100).find((p) => heroMoment(p).head >= 1)!;
    expect(gone).toBeLessThan(settle);
  });
});

describe("the first-load opening", () => {
  it("is short (900 ms at most) and ends where the lid rests", () => {
    expect(FLOURISH_MS).toBeLessThanOrEqual(900);
    expect(lidFlourish(1)).toBe(0);
  });

  it("starts where the hero draws the lid before its script runs (Hero.astro: rotateX(-70deg))", () => {
    expect(LEAN + lidFlourish(0)).toBe(-70);
  });

  it("opens only, and keeps the screen towards the viewer the whole way (never dark)", () => {
    // The eye sits about 5.5° above the hinge: under that the lid would show its back.
    for (let i = 0; i <= 100; i++) {
      const lean = LEAN + lidFlourish(i / 100); // degrees back from upright
      expect(90 + lean).toBeGreaterThan(5.5);
      if (i) expect(lidFlourish(i / 100)).toBeGreaterThanOrEqual(lidFlourish((i - 1) / 100));
    }
  });
});

describe("the opening's hinge", () => {
  it("accelerates out of closed: no jump at the start, the lid picks up speed", () => {
    expect(hinge(0)).toBe(0);
    const speed = (a: number, b: number) => (hinge(b) - hinge(a)) / (b - a);
    expect(speed(0, 0.02)).toBeLessThan(speed(0.04, 0.06));
    expect(speed(0.04, 0.06)).toBeLessThan(speed(0.08, 0.1));
  });

  it("settles softly, with no step at the end and no swing past where it rests", () => {
    expect(hinge(1)).toBe(1);
    expect(1 - hinge(0.7)).toBeLessThan(0.015);
    expect(1 - hinge(0.98)).toBeLessThan(0.001);
    for (let i = 1; i <= 200; i++) expect(hinge(i / 200)).toBeGreaterThanOrEqual(hinge((i - 1) / 200));
    for (let i = 0; i <= 200; i++) expect(hinge(i / 200)).toBeLessThanOrEqual(1);
  });

  it("is played as one fine-grained linear() curve, so no straight stretch shows", () => {
    expect(FLOURISH_EASE.startsWith("linear(0,")).toBe(true);
    expect(FLOURISH_EASE.endsWith(", 1)")).toBe(true);
    const stops = FLOURISH_EASE.slice(7, -1).split(",").map(Number);
    // A stop at least every 25 ms of the opening.
    expect(stops.length - 1).toBeGreaterThanOrEqual(FLOURISH_MS / 25);
    for (let i = 1; i < stops.length; i++) expect(stops[i]).toBeGreaterThanOrEqual(stops[i - 1]);
  });
});

describe("the start scale", () => {
  it("fits the room under the name, within 0.6 to 0.8", () => {
    expect(startScale(500, 700)).toBeCloseTo(500 / 700);
    expect(startScale(2000, 700)).toBe(0.8);
    expect(startScale(100, 700)).toBe(0.6);
  });
});
