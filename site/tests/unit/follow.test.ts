import { describe, expect, it } from "vitest";
import { followOffset } from "../../src/demo/follow";

// The app's rule (pages/lyrics.slint, SyncedLines.follow): the line playing is held a third of
// the way down the view, but the list never scrolls above its first line nor past its end.
describe("where the lines rest", () => {
  const view = 600; // the panel's height
  const content = 24 + 30 * 50 + view / 2; // padding, 30 lines of 50 px, half the view below

  it("rests at the top while the line playing is above a third of the way down", () => {
    expect(followOffset({ top: 24, height: 50 }, view, content)).toBe(0);
    expect(followOffset({ top: 24 + 2 * 50, height: 50 }, view, content)).toBe(0);
  });

  it("holds the line playing with its centre a third of the way down", () => {
    const line = { top: 24 + 10 * 50, height: 50 };
    const y = followOffset(line, view, content);
    expect(line.top + line.height / 2 - y).toBeCloseTo(view / 3, 6);
  });

  it("never scrolls past the end of the list", () => {
    const last = { top: 24 + 29 * 50, height: 50 };
    expect(followOffset(last, view, content)).toBe(content - view);
  });

  it("stays at the top when the list is shorter than the view", () => {
    expect(followOffset({ top: 400, height: 50 }, view, 500)).toBe(0);
  });

  it("is at the top with no line playing", () => {
    expect(followOffset(null, view, content)).toBe(0);
  });
});
