import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { describe, expect, it } from "vitest";
// @ts-expect-error: plain ESM script without types
import { readPins, readKept, check, MAX_BYTES } from "../../scripts/helpers.mjs";

const sidecars = readFileSync(new URL("../../../crates/streams/src/sidecars.rs", import.meta.url), "utf8");

describe("helpers", () => {
  it("reads both architectures' yt-dlp pins from the app's code", () => {
    const pins = readPins(sidecars);
    expect(pins.map((p: { path: string }) => p.path.split("/").pop())).toEqual([
      "yt-dlp.exe",
      "yt-dlp_arm64.exe",
    ]);
    for (const pin of pins) {
      // The site's path holds the GitHub release's version and file name.
      const [, release] = pin.url.split("/releases/download/");
      expect(pin.path).toBe(`helpers/yt-dlp/${release}`);
      expect(pin.sha256).toMatch(/^[0-9a-f]{64}$/);
    }
  });

  it("accepts only the pinned bytes, under the asset size limit", () => {
    const bytes = Buffer.from("not really yt-dlp");
    const pin = { path: "helpers/yt-dlp/1/yt-dlp.exe", sha256: createHash("sha256").update(bytes).digest("hex") };
    expect(() => check(bytes, pin)).not.toThrow();
    expect(() => check(Buffer.from("another file"), pin)).toThrow(/SHA256/);
    expect(() => check(Buffer.alloc(MAX_BYTES + 1), pin)).toThrow(/limit/);
  });

  it("keeps serving the earlier pins listed in helpers-kept.json", () => {
    const kept = readFileSync(new URL("../../scripts/helpers-kept.json", import.meta.url), "utf8");
    expect(Array.isArray(readKept(kept))).toBe(true);
    const old = {
      url: "https://github.com/yt-dlp/yt-dlp/releases/download/2026.01.01/yt-dlp.exe",
      sha256: "a".repeat(64),
      path: "helpers/yt-dlp/2026.01.01/yt-dlp.exe",
    };
    expect(readKept(JSON.stringify([old]))).toEqual([old]);
    expect(() => readKept(JSON.stringify([{ ...old, path: "helpers/yt-dlp/../../x.exe" }]))).toThrow(/helpers path/);
  });

  it("refuses a pin that would write outside the helpers folder", () => {
    const evil = sidecars.replace(
      /ytdlp_mirror: "[^"]+yt-dlp\.exe"/,
      'ytdlp_mirror: "https://featherplay.candygoogy.dev/helpers/yt-dlp/../../../x.exe"',
    );
    expect(() => readPins(evil)).toThrow(/helpers path/);
  });
});
