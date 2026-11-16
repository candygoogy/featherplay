import { describe, expect, it } from "vitest";
import { existsSync, statSync } from "node:fs";
import { notes, PEN_CHARS } from "../../src/data/notes";

// The handwritten notes (src/data/notes.ts) are set in a subset of Reenie Beanie cut to their
// own characters (public/fonts): a note needing a character the subset lacks would fall back
// to another face, and a character no note uses is weight for nothing.
describe("the hand's font", () => {
  const used = new Set([...Object.values(notes).join("")]);
  it("has every character the notes use", () => {
    for (const c of used) expect(PEN_CHARS, `"${c}"`).toContain(c);
  });
  it("has no character they don't", () => {
    for (const c of PEN_CHARS) expect(used.has(c), `"${c}"`).toBe(true);
  });
  it("is the subset file, small, with its licence beside it", () => {
    const font = "public/fonts/reenie-beanie-subset.woff2";
    expect(existsSync(font)).toBe(true);
    expect(statSync(font).size).toBeLessThan(12_000);
    expect(existsSync("public/fonts/reenie-beanie-OFL.txt")).toBe(true);
  });
  it("keeps to eight to ten notes on the page (and four only a visitor who moves the window or puts it away sees), in the maker's plain lowercase voice", () => {
    const all = Object.values(notes);
    const { closed, tray, moved, pick, ...onThePage } = notes;
    expect(closed && tray && moved && pick).toBeTruthy();
    expect(Object.keys(onThePage).length).toBeGreaterThanOrEqual(8);
    expect(Object.keys(onThePage).length).toBeLessThanOrEqual(10);
    for (const n of all) expect(n).toBe(n.toLowerCase());
  });
});
