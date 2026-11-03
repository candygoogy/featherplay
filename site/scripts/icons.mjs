// The site's icons, made from the logo's own drawings (run by hand when the logo changes, then
// commit what it writes to public/; the sharing card is scripts/og.mjs):
//   node scripts/icons.mjs
//
// - favicon.svg: the Tri-vane master; on light browser chrome it carries the light variant's
//   crimson ring round the play (the logo's own light drawing), on dark chrome it doesn't.
// - favicon.ico: 16, 32 and 48 px for anything that asks for /favicon.ico, the 16 px entry
//   from the logo's own 16 px drawing (the light variant, which reads on any ground).
// - apple-touch-icon.png, 180 x 180: the dark master on the app's ground (#0f0f0f), opaque,
//   with room round it for the system's rounded mask.
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";

const pub = (f) => fileURLToPath(new URL(`../public/${f}`, import.meta.url));
const logo = (f) => fileURLToPath(new URL(`../../apps/featherplay/ui/logo/${f}`, import.meta.url));
const dataUrl = (svg) => `data:image/svg+xml;base64,${Buffer.from(svg).toString("base64")}`;

const browser = await chromium.launch();

/** The SVG drawn at `size` px on a transparent (or `ground`) square, as PNG bytes. */
async function raster(svg, size, ground = "transparent", mark = size) {
  const page = await browser.newPage({ viewport: { width: size, height: size } });
  await page.setContent(
    `<style>html,body{margin:0;background:${ground}}body{display:grid;place-items:center;width:${size}px;height:${size}px}</style>` +
      `<img src="${dataUrl(svg)}" width="${mark}" height="${mark}">`,
  );
  await page.locator("img").evaluate((img) => img.decode());
  const png = await page.screenshot({ omitBackground: ground === "transparent" });
  await page.close();
  return png;
}

/** An ICO with one PNG entry per size (read by everything since Vista). */
function ico(entries) {
  const head = Buffer.alloc(6 + 16 * entries.length);
  head.writeUInt16LE(1, 2);
  head.writeUInt16LE(entries.length, 4);
  let offset = head.length;
  entries.forEach(([size, png], i) => {
    const at = 6 + 16 * i;
    head.writeUInt8(size % 256, at);
    head.writeUInt8(size % 256, at + 1);
    head.writeUInt16LE(1, at + 4);
    head.writeUInt16LE(32, at + 6);
    head.writeUInt32LE(png.length, at + 8);
    head.writeUInt32LE(offset, at + 12);
    offset += png.length;
  });
  return Buffer.concat([head, ...entries.map(([, png]) => png)]);
}

// Icons.
const master = readFileSync(logo("tri-vane.svg"), "utf8");
const light = readFileSync(logo("tri-vane-light.svg"), "utf8");
const light16 = readFileSync(logo("tri-vane-light-16.svg"), "utf8");
writeFileSync(
  pub("favicon.svg"),
  light.replace(
    /(<svg[^>]*>)/,
    `$1\n  <style>@media (prefers-color-scheme: dark) { #ring { display: none } }</style>`,
  ),
);
writeFileSync(
  pub("favicon.ico"),
  ico([
    [16, await raster(light16, 16)],
    [32, await raster(light, 32)],
    [48, await raster(light, 48)],
  ]),
);
writeFileSync(pub("apple-touch-icon.png"), await raster(master, 180, "#0f0f0f", 132));

await browser.close();
console.log("wrote favicon.svg, favicon.ico, apple-touch-icon.png");
