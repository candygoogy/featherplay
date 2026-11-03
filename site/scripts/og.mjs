// The sharing card, public/og.jpg (1200 x 630), composed from the real laptop (run by hand when
// the hero or the logo changes, then commit what it writes):
//   pnpm build && pnpm exec astro preview --port 4321   (in another terminal)
//   node scripts/og.mjs http://127.0.0.1:4321/ [out.jpg] [preview.png]
//
// The hero's own first screen is laid out for a browser window, not a 1.9:1 card: shot as it
// was, the laptop came out small and low with empty black round it. So the card is set on its
// own: the Tri-vane and the name top left, the line top right, and the laptop (the page's own,
// shot alone at twice the size on a clear ground, lid open, the replica on the lyrics) filling
// the lower two thirds, with the cover's colour behind it fading out to the ground well inside
// the frame. Drawn at 2400 x 1260 and scaled down by half, so every edge and letter is sharp.
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const url = process.argv[2] ?? "http://127.0.0.1:4321/";
const pub = (f) => fileURLToPath(new URL(`../public/${f}`, import.meta.url));
const out = process.argv[3] ?? pub("og.jpg");
const preview = process.argv[4];
const b64 = (buf) => Buffer.from(buf).toString("base64");

// The card's geometry, in its own 1200 x 630 pixels.
const W = 1200;
const H = 630;
const MARGIN = 56;
const LAPTOP_W = 712; // the laptop's outline, whole, from just under the name to the bottom
const LAPTOP_TOP = 144;
const GROUND = "#0f0f0f"; // the app's dark ground
const COLOUR = "#E8894A"; // California Lullabye, the song the hero starts on

const browser = await chromium.launch();

// 1. The laptop alone: the page in the dark theme, still (reduced motion: the lid open and
//    straight, no scroll moment), everything but the laptop hidden and the ground made clear.
const site = await browser.newPage({
  viewport: { width: 1600, height: 1300 },
  deviceScaleFactor: 2,
  colorScheme: "dark",
  reducedMotion: "reduce",
});
await site.addInitScript(() => localStorage.setItem("fp-theme", "dark"));
await site.goto(url, { waitUntil: "networkidle" });
await site.addStyleTag({
  content: `
    html, body, main, .hero, .hero-pin, .hero-stage, .laptop-wrap { background: transparent !important; }
    body > header, .hero-head, [data-glow], [data-tour], .hero-audio { visibility: hidden !important; }
  `,
});
// The wallpaper comes in once the page is idle; the fonts and covers with the load.
await site.waitForTimeout(2500);
const lap = site.locator("[data-hero] [data-laptop]");
const box = await lap.boundingBox();
if (!box) throw new Error("no laptop in the hero");
// Room round the outline for its contact shadow.
const pad = { x: Math.round(box.width * 0.04), top: 4, bottom: Math.round(box.height * 0.08) };
const laptopPng = await site.screenshot({
  omitBackground: true,
  clip: { x: box.x - pad.x, y: box.y - pad.top, width: box.width + 2 * pad.x, height: box.height + pad.top + pad.bottom },
});
await site.close();

// 2. The card, laid out at 1200 x 630 and drawn at twice that.
const k = LAPTOP_W / box.width; // the laptop's scale on the card
const shot = { w: (box.width + 2 * pad.x) * k, h: (box.height + pad.top + pad.bottom) * k };
const shotLeft = (W - shot.w) / 2;
const shotTop = LAPTOP_TOP - pad.top * k;
// The glow is centred a little above the screen's middle, wide enough to fill the sides, and
// has faded to the ground before every edge (as on the page, it's masked out towards the top,
// so the name and the line stand on the plain ground).
const glow = { x: W / 2, y: LAPTOP_TOP + box.height * k * 0.3, rx: 630, ry: 400 };
const mix = (p) => `color-mix(in srgb, ${COLOUR} ${p}%, ${GROUND})`;
const font = b64(readFileSync(pub("fonts/manrope-latin-400-800.woff2")));
const mark = b64(readFileSync(pub("tri-vane.svg")));
const card = await browser.newPage({ viewport: { width: W, height: H }, deviceScaleFactor: 2 });
await card.setContent(`<!doctype html>
<style>
  @font-face { font-family: Manrope; src: url(data:font/woff2;base64,${font}) format("woff2"); font-weight: 400 800; }
  html, body { margin: 0; width: ${W}px; height: ${H}px; overflow: hidden; background: ${GROUND}; }
  body { position: relative; font-family: Manrope, sans-serif; color: #fff; -webkit-font-smoothing: antialiased; }
  .glow {
    position: absolute; left: ${glow.x - glow.rx}px; top: ${glow.y - glow.ry}px;
    width: ${2 * glow.rx}px; height: ${2 * glow.ry}px;
    background: radial-gradient(closest-side,
      ${mix(60)} 0%, ${mix(56)} 30%, ${mix(45)} 48%, ${mix(32)} 62%, ${mix(19)} 74%,
      ${mix(9)} 84%, ${mix(3)} 92%, ${GROUND} 100%);
    mask-image: linear-gradient(to bottom, transparent ${((LAPTOP_TOP - 150 - (glow.y - glow.ry)) / (2 * glow.ry)) * 100}%, #000 ${((LAPTOP_TOP + 40 - (glow.y - glow.ry)) / (2 * glow.ry)) * 100}%);
  }
  .laptop { position: absolute; left: ${shotLeft}px; top: ${shotTop}px; width: ${shot.w}px; height: ${shot.h}px; }
  .name { position: absolute; left: ${MARGIN}px; top: 44px; display: flex; align-items: center; gap: 16px; }
  .name img { width: 64px; height: 64px; display: block; }
  .name span { font-size: 76px; font-weight: 800; letter-spacing: -0.055em; line-height: 1; padding-bottom: 10px; }
  .line {
    position: absolute; right: ${MARGIN}px; top: 54px; margin: 0; white-space: nowrap;
    font-size: 24px; font-weight: 600; line-height: 1.25; letter-spacing: -0.02em;
  }
</style>
<div class="glow"></div>
<img class="laptop" src="data:image/png;base64,${b64(laptopPng)}" alt="">
<div class="name"><img src="data:image/svg+xml;base64,${mark}" alt=""><span>featherplay</span></div>
<p class="line">A free, native Windows player for<br>YouTube Music, light enough to forget it’s running.</p>`);
await card.evaluate(async () => {
  await document.fonts.ready;
  await Promise.all([...document.images].map((i) => i.decode()));
});
const big = await card.screenshot();
await card.close();

// 3. Scaled down by half with the browser's own image scaler, then a JPEG (the glow and the
//    screen are photographic: a fraction of a PNG's bytes).
const small = await browser.newPage({ viewport: { width: W, height: H } });
await small.setContent(
  `<style>html,body{margin:0;background:${GROUND}}img{display:block;width:${W}px;height:${H}px}</style>` +
    `<img src="data:image/png;base64,${b64(big)}">`,
);
await small.locator("img").evaluate((img) => img.decode());
writeFileSync(out, await small.screenshot({ type: "jpeg", quality: 90 }));
if (preview) writeFileSync(preview, await small.screenshot());
await browser.close();
console.log(`wrote ${out}`);
