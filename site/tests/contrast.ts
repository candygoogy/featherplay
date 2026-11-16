import type { Page } from "@playwright/test";

// Text contrast over what is really behind it (the desk's frosted glass, a pane over the page):
// each text made transparent, the ground behind its line boxes captured, and its colour set
// against the worst pixel of that ground (the 95th percentile toward the text's own lightness,
// so a stray speck of grain doesn't decide it). The study's method (mockups/11-study).

type Rgb = [number, number, number];
const lum = ([r, g, b]: Rgb) => {
  const f = (v: number) => ((v /= 255) <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
};
const ratio = (a: number, b: number) => (Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05);

/** The contrast of each element's text (the first `max` matches of `selector`) over its ground. */
export async function textContrast(page: Page, selector: string, max = 8): Promise<{ text: string; ratio: number }[]> {
  const items = await page.evaluate(
    ([sel, max]) => {
      const out: { text: string; color: string; box: { x: number; y: number; width: number; height: number } }[] = [];
      for (const el of [...document.querySelectorAll<HTMLElement>(sel)].slice(0, max)) {
        if (!el.getClientRects().length || getComputedStyle(el).visibility === "hidden") continue;
        const r = document.createRange();
        r.selectNodeContents(el);
        const rs = [...r.getClientRects()].filter((x) => x.width > 1 && x.height > 1);
        if (!rs.length) continue;
        const x = Math.max(0, Math.min(...rs.map((q) => q.left)));
        const y = Math.max(0, Math.min(...rs.map((q) => q.top)));
        const w = Math.min(innerWidth, Math.max(...rs.map((q) => q.right))) - x;
        const h = Math.min(innerHeight, Math.max(...rs.map((q) => q.bottom))) - y;
        if (w < 2 || h < 2) continue;
        out.push({ text: (el.textContent ?? "").trim().slice(0, 30), color: getComputedStyle(el).color, box: { x, y, width: w, height: h } });
      }
      return out;
    },
    [selector, max] as const,
  );
  if (!items.length) return [];
  // The text made transparent (and anything drawn in its colour inside it), the ground captured.
  const tag = await page.addStyleTag({ content: `${selector}, ${selector} * { color: transparent !important; -webkit-text-fill-color: transparent !important; text-decoration-color: transparent !important; } ${selector} svg { visibility: hidden !important; }` });
  await page.waitForTimeout(80);
  const shots = [];
  for (const it of items) shots.push((await page.screenshot({ clip: it.box, scale: "css" })).toString("base64"));
  await tag.evaluate((t) => t.remove());
  // Decoded on a blank page of the same context (the site's own policy allows no data: URLs).
  const blank = await page.context().newPage();
  const res: { text: string; ratio: number }[] = [];
  for (let i = 0; i < items.length; i++) {
    const px: Rgb[] = await blank.evaluate(async (b64) => {
      const img = new Image();
      img.src = "data:image/png;base64," + b64;
      await img.decode();
      const c = new OffscreenCanvas(img.width, img.height);
      const x = c.getContext("2d")!;
      x.drawImage(img, 0, 0);
      const d = x.getImageData(0, 0, c.width, c.height).data;
      const out: [number, number, number][] = [];
      for (let k = 0; k < d.length; k += 4 * 3) out.push([d[k], d[k + 1], d[k + 2]]);
      return out;
    }, shots[i]);
    // rgb() in 0-255, or color(srgb ...) in 0-1.
    const c = items[i].color;
    const m = c.replace(/^color\(srgb/, "").match(/[\d.]+/g)!.map(Number);
    if (c.startsWith("color(")) for (let j = 0; j < 3; j++) m[j] *= 255;
    const ground = px.map(lum).sort((a, b) => a - b);
    const median = ground[Math.floor(ground.length / 2)];
    // The text's colour over the ground (its alpha, if any, over the median).
    const a = m[3] ?? 1;
    const medianRgb = px[Math.floor(px.length / 2)];
    const text: Rgb = [0, 1, 2].map((j) => m[j] * a + medianRgb[j] * (1 - a)) as Rgb;
    const tl = lum(text);
    // The worst ground: the 95th percentile toward the text's own lightness.
    const worst = tl > median ? ground[Math.floor(ground.length * 0.95)] : ground[Math.floor(ground.length * 0.05)];
    res.push({ text: items[i].text, ratio: +ratio(tl, worst).toFixed(2) });
  }
  await blank.close();
  return res;
}
