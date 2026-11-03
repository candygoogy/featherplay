import { expect, test, type Page } from "@playwright/test";
import { readdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { skipTour, wake } from "./helpers";

// The Content-Security-Policy in public/_headers, as Cloudflare's runtime serves it (wrangler
// dev, scripts/wrangler-dev.mjs): every page carries it and loads with no violation, and no
// built page has an inline script, style element or event-handler attribute for it to block.
const cf = "http://127.0.0.1:8788";
const site = fileURLToPath(new URL("..", import.meta.url));

/** The site's pages, from src/pages (each `.astro` file is one page). */
const pages = readdirSync(`${site}/src/pages`)
  .filter((f) => f.endsWith(".astro"))
  .map((f) => f.replace(/\.astro$/, ""))
  .map((name) => (name === "index" ? "/" : `/${name}`))
  .sort();

/** A header's value as public/_headers sets it for every path (the `/*` block). */
function headerRule(name: string) {
  const text = readFileSync(`${site}/public/_headers`, "utf8");
  const block = text.split(/\r?\n(?=\S)/).find((b) => b.startsWith("/*"));
  const line = block?.split(/\r?\n/).find((l) => l.trim().toLowerCase().startsWith(`${name.toLowerCase()}:`));
  if (!line) throw new Error(`public/_headers sets no ${name} for /*`);
  return line.slice(line.indexOf(":") + 1).trim();
}

const htmlPath = (path: string) => `${site}/dist/${path === "/" ? "index" : path.slice(1)}.html`;

test("the built pages are exactly the pages in src/pages", () => {
  const built = readdirSync(`${site}/dist`)
    .filter((f) => f.endsWith(".html"))
    .map((f) => (f === "index.html" ? "/" : `/${f.replace(/\.html$/, "")}`))
    .sort();
  expect(built).toEqual(pages);
});

for (const path of pages) {
  test(`${path} has no inline script, style element or handler attribute`, () => {
    const html = readFileSync(htmlPath(path), "utf8");
    // Every <script> either loads a file or is JSON-LD (data, which CSP doesn't block).
    const inline = [...html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script>/gi)]
      .filter(([, attrs]) => !/\bsrc=/i.test(attrs) && !/type=["']?application\/ld\+json/i.test(attrs))
      .map(([tag]) => tag.slice(0, 120));
    expect(inline, "inline scripts: script-src allows files from 'self' only").toEqual([]);
    expect(html.match(/<style\b[^>]*>/gi) ?? [], "style elements: style-src-elem allows files only").toEqual([]);
    expect(html.match(/\son[a-z]+\s*=/gi) ?? [], "event-handler attributes").toEqual([]);
    expect(html.match(/(?:src|href)=["']?(?:javascript|data|blob):/gi) ?? [], "javascript:, data: or blob: URLs").toEqual([]);
  });

  test(`${path} is served with the CSP and HSTS headers`, async ({ request }) => {
    const res = await request.get(`${cf}${path}`);
    expect(res.headers()["content-security-policy"]).toBe(headerRule("Content-Security-Policy"));
    expect(res.headers()["strict-transport-security"]).toBe("max-age=31536000; includeSubDomains");
    expect(res.headers()["x-content-type-options"]).toBe("nosniff");
    expect(res.headers()["x-frame-options"]).toBe("DENY");
  });

  test(`${path} loads with no CSP violation`, async ({ page }) => {
    const problems = await watch(page);
    const res = await page.goto(`${cf}${path}`);
    expect(res?.headers()["content-security-policy"]).toBe(headerRule("Content-Security-Policy"));
    await page.waitForLoadState("networkidle");
    // Down the whole page, so everything that loads on sight (covers, sections' scripts) loads.
    await page.evaluate(async () => {
      for (let y = 0; y < document.documentElement.scrollHeight; y += innerHeight / 2) {
        scrollTo({ top: y, behavior: "instant" });
        await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
      }
    });
    await page.waitForLoadState("networkidle");
    expect(await problems()).toEqual([]);
  });
}

test("an unknown path's 404 page carries the CSP too", async ({ page }) => {
  const problems = await watch(page, "/no/such/page");
  const res = await page.goto(`${cf}/no/such/page`);
  expect(res?.status()).toBe(404);
  expect(res?.headers()["content-security-policy"]).toBe(headerRule("Content-Security-Policy"));
  await page.waitForLoadState("networkidle");
  expect(await problems()).toEqual([]);
});

test("pressing play loads the demo's clip under the CSP (media-src)", async ({ page }) => {
  const problems = await watch(page);
  await page.goto(`${cf}/`);
  await wake(page);
  await skipTour(page);
  const clip = page.waitForResponse((r) => new URL(r.url()).pathname.startsWith("/music/"));
  await page.locator("[data-hero] .bar .fp-play").first().click();
  expect((await clip).ok()).toBe(true);
  expect(await problems()).toEqual([]);
});

/** Collects CSP violations, console errors and uncaught errors from now on. `missing` is a
 *  path expected to answer 404, whose own "Failed to load resource" isn't a problem. */
async function watch(page: Page, missing?: string) {
  const seen: string[] = [];
  page.on("console", (m) => {
    if (m.type() !== "error") return;
    const at = m.location().url;
    if (missing && at && new URL(at).pathname === missing && m.text().includes("status of 404")) return;
    seen.push(`console: ${m.text()} (${at})`);
  });
  page.on("pageerror", (e) => seen.push(`error: ${e.message}`));
  await page.addInitScript(() => {
    const w = window as unknown as { __csp: string[] };
    w.__csp = [];
    document.addEventListener("securitypolicyviolation", (e) =>
      w.__csp.push(`csp: ${e.effectiveDirective} blocked ${e.blockedURI || "inline"} (${e.sourceFile}:${e.lineNumber})`),
    );
  });
  return async () => [...seen, ...(await page.evaluate(() => (window as unknown as { __csp: string[] }).__csp))];
}
