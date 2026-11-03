import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { randomBytes } from "node:crypto";
import { afterEach, describe, expect, it } from "vitest";
// @ts-expect-error: plain ESM script without types
import { measure, check } from "../../scripts/budget.mjs";

const dirs: string[] = [];
function dist(files: Record<string, string | Buffer>) {
  const dir = mkdtempSync(join(tmpdir(), "fp-budget-"));
  dirs.push(dir);
  for (const [name, body] of Object.entries(files)) {
    const path = join(dir, name);
    mkdirSync(join(path, ".."), { recursive: true });
    writeFileSync(path, body);
  }
  return dir;
}
afterEach(() => {
  for (const d of dirs.splice(0)) rmSync(d, { recursive: true, force: true });
});

describe("budget", () => {
  it("counts the page and what it references, following script imports", () => {
    const dir = dist({
      "index.html": `<link rel="stylesheet" href="/_astro/a.css"><script type="module" src="/_astro/m.js"></script><img src="/covers/x.webp">`,
      "_astro/a.css": `@font-face{src:url(/fonts/f.woff2)}`,
      "_astro/m.js": `import "./chunk.js";`,
      "_astro/chunk.js": `export const s = "/covers/y.webp";`,
      "fonts/f.woff2": Buffer.alloc(1000),
      "covers/x.webp": Buffer.alloc(2000),
      "covers/y.webp": Buffer.alloc(3000),
      "unused.png": Buffer.alloc(50_000),
    });
    const m = measure(dir, "index.html");
    expect(m.files.sort()).toEqual(
      ["_astro/a.css", "_astro/chunk.js", "_astro/m.js", "covers/x.webp", "covers/y.webp", "fonts/f.woff2", "index.html"].sort(),
    );
    expect(m.totalBytes).toBeGreaterThan(6000);
    expect(m.totalBytes).toBeLessThan(10_000);
  });

  it("fails when the JavaScript is over 60 KB compressed", () => {
    const dir = dist({
      "index.html": `<script type="module" src="/_astro/big.js"></script>`,
      // Random bytes don't compress, so this stays ~90 KB gzipped.
      "_astro/big.js": `const x = "${randomBytes(70_000).toString("base64")}";`,
    });
    const result = check(dir);
    expect(result.ok).toBe(false);
    expect(result.lines.join("\n")).toMatch(/JS/);
  });

  it("counts inline scripts as JavaScript too", () => {
    const dir = dist({
      "index.html": `<script type="module">const x = "${randomBytes(70_000).toString("base64")}";</script>`,
    });
    const m = measure(dir, "index.html");
    expect(m.jsBytes).toBeGreaterThan(60 * 1024);
    expect(check(dir).ok).toBe(false);
  });

  it("passes a small site", () => {
    const dir = dist({
      "index.html": `<p>hi</p>`,
      "404.html": `<p>not found</p>`,
    });
    expect(check(dir).ok).toBe(true);
  });

  it("fails a 404 page over 100 KB", () => {
    const dir = dist({
      "index.html": `<p>hi</p>`,
      "404.html": `<img src="/big.png">`,
      "big.png": randomBytes(120_000),
    });
    expect(check(dir).ok).toBe(false);
  });
});
