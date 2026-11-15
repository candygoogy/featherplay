import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";

// Every theme colour moves on the one clock: a registered colour that :root doesn't transition
// would snap while the rest ease (tokens.css), and an unregistered one can't ease at all.
const css = readFileSync(new URL("../../src/styles/tokens.css", import.meta.url), "utf8");
const registered = [...css.matchAll(/@property (--[\w-]+) \{ syntax: "<color>"/g)].map((m) => m[1]);
const list = css.match(/transition-property:([^;]+);/)![1];
const eased = new Set(list.split(",").map((s) => s.trim()));
const paired = [...css.matchAll(/^\s+(--[\w-]+): light-dark\(/gm)].map((m) => m[1]);

describe("theme tokens", () => {
  it("eases every registered colour on the theme's clock", () => {
    expect(registered.filter((n) => !eased.has(n))).toEqual([]);
  });
  it("registers every light and dark pair, so it can ease", () => {
    expect(paired.filter((n) => !registered.includes(n))).toEqual([]);
  });
  it("has the page system's desk, inks and glass", () => {
    for (const n of ["--desk", "--ink2", "--ink3", "--mat-dock", "--mat-capsule", "--mat-fly", "--mat-solid"]) {
      expect(registered).toContain(n);
    }
  });
});
