// The sharing card, public/og.jpg (1200 x 630): the site's own first screen, as a visitor sees
// it (run by hand when the first screen or the logo changes, then commit what it writes):
//   pnpm build && pnpm exec astro preview --port 4321   (in another terminal)
//   node scripts/og.mjs http://127.0.0.1:4321/ [out.jpg] [preview.png]
//
// The first screen lays itself out for any window (src/scripts/first-math.ts), so it is shot in
// a window of the card's own shape: 2400 x 1260, the dark theme, the opening song, settled (no
// arrival), its notes written, then scaled down by half with the browser's own image scaler,
// so every edge and letter is sharp. A JPEG: the frosted desk is photographic.
import { writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const url = process.argv[2] ?? "http://127.0.0.1:4321/";
const out = process.argv[3] ?? fileURLToPath(new URL("../public/og.jpg", import.meta.url));
const preview = process.argv[4];
const W = 1200;
const H = 630;

const browser = await chromium.launch();
const site = await browser.newPage({ viewport: { width: W * 2, height: H * 2 }, colorScheme: "dark", reducedMotion: "reduce" });
// No sound to fetch: the card is a still.
await site.route("**/music/**", (r) => r.abort());
await site.goto(url, { waitUntil: "load" });
await site.locator("[data-first][data-shown]").waitFor();
await site.evaluate(async () => {
  await document.fonts.ready;
  // (The images on screen; lazy ones further down never load here.)
  await Promise.all([...document.images].filter((i) => i.complete).map((i) => i.decode().catch(() => {})));
});
// The notes write themselves in once (instantly with reduced motion); the desk's fade settles.
await site.waitForTimeout(1500);
const big = await site.screenshot();

const small = await browser.newPage({ viewport: { width: W, height: H } });
await small.setContent(
  `<style>html,body{margin:0;background:#0f0f0f}img{display:block;width:${W}px;height:${H}px}</style>` +
    `<img src="data:image/png;base64,${big.toString("base64")}">`,
);
await small.locator("img").evaluate((img) => img.decode());
writeFileSync(out, await small.screenshot({ type: "jpeg", quality: 88 }));
if (preview) writeFileSync(preview, await small.screenshot());
await browser.close();
console.log(`wrote ${out}`);
