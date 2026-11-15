import { describe, expect, it } from "vitest";
import { inkIsLight, mergeLines } from "../../src/scripts/selection";

// The drawn selection (src/scripts/selection.ts): its pieces are blended so the text keeps
// its colour, by the text's own ink: screened under light text, multiplied under dark text.

describe("which way a piece blends", () => {
  it("takes white and the dark theme's greys for light ink", () => {
    expect(inkIsLight("rgb(255, 255, 255)")).toBe(true);
    expect(inkIsLight("rgb(170, 170, 170)")).toBe(true);
    expect(inkIsLight("rgba(255, 255, 255, 0.62)")).toBe(true);
  });
  it("takes near-black and the light theme's warm greys for dark ink", () => {
    expect(inkIsLight("rgb(15, 15, 15)")).toBe(false);
    expect(inkIsLight("rgb(74, 69, 64)")).toBe(false);
    expect(inkIsLight("rgb(100, 94, 87)")).toBe(false);
  });
  it("reads colours as Chrome and Firefox write them", () => {
    expect(inkIsLight("color(srgb 1 1 1)")).toBe(true);
    expect(inkIsLight("color(srgb 0.06 0.06 0.06 / 0.9)")).toBe(false);
    expect(inkIsLight("oklch(0.98 0 0)")).toBe(true);
    expect(inkIsLight("oklch(0.2 0.01 60)")).toBe(false);
  });
});

describe("merging line boxes", () => {
  it("never merges a run of light ink with a run of dark ink on the same line", () => {
    const merged = mergeLines([
      { left: 0, top: 0, right: 50, bottom: 20, light: true },
      { left: 52, top: 0, right: 120, bottom: 20, light: false },
      { left: 122, top: 0, right: 160, bottom: 20, light: false },
    ]);
    expect(merged).toEqual([
      { left: 0, top: 0, right: 50, bottom: 20, light: true },
      { left: 52, top: 0, right: 160, bottom: 20, light: false },
    ]);
  });
});
