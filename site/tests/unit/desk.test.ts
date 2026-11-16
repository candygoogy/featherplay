import { describe, expect, it } from "vitest";
import { deskTones, oklchOf, warmth } from "../../src/data/desk";
import { songs, songById } from "../../src/data/songs";

// The desk's frosted glass (src/data/desk.ts): each song's four tones per theme, by rule, so a
// seventh song behaves like the six. The study's recipe ("K2, clean"), with the owner's change:
// in the dark theme a warm song must not turn the whole screen brown.

const L = (css: string) => Number(/oklch\(([\d.]+)/.exec(css)![1]);
const C = (css: string) => Number(/oklch\([\d.]+ ([\d.]+)/.exec(css)![1]);

describe("reading a colour", () => {
  it("gives OKLCH for a hex colour", () => {
    const [l, c, h] = oklchOf("#ffffff");
    expect(l).toBeCloseTo(1, 3);
    expect(c).toBeLessThan(0.001);
    const [, , hue] = oklchOf("#E8894A");
    expect(hue).toBeGreaterThan(45);
    expect(hue).toBeLessThan(60);
  });
});

describe("warmth", () => {
  it("is full for the warm covers and nothing for the blue and teal ones", () => {
    for (const id of ["california-lullabye", "origami", "in-this-moment"]) expect(warmth(songById(id)!.colour), id).toBeGreaterThan(0.9);
    for (const id of ["invisible-light", "she-dreams-in-blue", "watching-seabirds-soar"]) expect(warmth(songById(id)!.colour), id).toBe(0);
  });
  it("follows the hue, and fades with the chroma (a grey has none)", () => {
    expect(warmth("#c0392b")).toBeGreaterThan(0.6); // a red
    expect(warmth("#2e8b57")).toBeLessThan(0.3); // a green
    expect(warmth("#8a8580")).toBeLessThan(0.3); // a warm grey
  });
});

describe("the dark theme's tones", () => {
  it("are the study's for the cool songs", () => {
    const t = deskTones(songById("invisible-light")!.colour, "dark");
    expect(L(t.qg)).toBeCloseTo(0.15, 3);
    expect(L(t.qs)).toBeCloseTo(0.07, 3);
    expect(L(t.ql)).toBeCloseTo(0.56, 3);
    expect(L(t.qh)).toBeCloseTo(0.74, 3);
    expect(t.lit).toBe(1);
  });
  it("fall nearer to black for the warm songs, the colour kept in the lit corner", () => {
    const warm = deskTones(songById("california-lullabye")!.colour, "dark");
    const cool = deskTones(songById("invisible-light")!.colour, "dark");
    expect(L(warm.qg)).toBeLessThan(L(cool.qg));
    expect(C(warm.qg)).toBeLessThan(C(cool.qg));
    expect(L(warm.qs)).toBeLessThan(L(cool.qs));
    // Less of the screen carries the colour, but never none of it; the corner holds it.
    expect(warm.lit).toBeLessThan(cool.lit);
    expect(warm.lit).toBeGreaterThanOrEqual(0.5);
    expect(warm.hi).toBeGreaterThan(cool.hi);
    expect(L(warm.ql)).toBeCloseTo(L(cool.ql), 3);
  });
  it("caps the chroma, yellows to 72 % of it", () => {
    for (const s of songs) {
      const t = deskTones(s.colour, "dark");
      expect(C(t.ql)).toBeLessThanOrEqual(0.13 + 1e-9);
      expect(C(t.qg)).toBeLessThanOrEqual(0.04 + 1e-9);
    }
    const gold = deskTones(songById("origami")!.colour, "dark");
    expect(C(gold.ql)).toBeLessThanOrEqual(0.13 * 0.72 + 1e-3);
  });
});

describe("the light theme's tones", () => {
  it("are the study's, the same for warm and cool songs", () => {
    for (const s of songs) {
      const t = deskTones(s.colour, "light");
      expect(L(t.qg)).toBeCloseTo(0.95, 3);
      expect(L(t.qs)).toBeCloseTo(0.72, 3);
      expect(t.ql).toBe("oklch(0.995 0.004 90)");
      expect(t.qh).toBe("oklch(1 0 0)");
      expect(t.lit).toBe(1);
      expect(t.hi).toBeCloseTo(0.6, 5);
    }
  });
});
