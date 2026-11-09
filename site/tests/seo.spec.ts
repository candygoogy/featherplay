import { expect, test } from "@playwright/test";

test("index has title, description, canonical, og:image and SoftwareApplication JSON-LD", async ({ page }) => {
  await page.goto("/");
  await expect(page).toHaveTitle(/featherplay/);
  expect(await page.title()).not.toMatch(/YouTube|YTM|Google/);
  await expect(page.locator('meta[name="description"]')).toHaveAttribute("content", /.{60,}/);
  await expect(page.locator('link[rel="canonical"]')).toHaveAttribute("href", "https://featherplay.candygoogy.dev/");
  const og = await page.locator('meta[property="og:image"]').getAttribute("content");
  expect(og).toBe("https://featherplay.candygoogy.dev/og.jpg");
  const img = await page.request.get("/og.jpg");
  expect(img.ok()).toBe(true);
  expect(img.headers()["content-type"]).toBe("image/jpeg");
  const ld = JSON.parse((await page.locator('script[type="application/ld+json"]').textContent())!);
  expect(ld["@type"]).toBe("SoftwareApplication");
  expect(ld.name).toBe("featherplay");
  expect(ld.operatingSystem).toMatch(/Windows/);
  expect(ld.offers.price).toBe("0");
  expect(ld.softwareVersion).toBe("0.2.0");
  expect(ld.fileSize).toBe("10.68 MB");
  expect(ld.aggregateRating).toBeUndefined();
});

test("/method has its own canonical; the 404 is not indexed", async ({ page }) => {
  await page.goto("/method");
  await expect(page.locator('link[rel="canonical"]')).toHaveAttribute("href", "https://featherplay.candygoogy.dev/method");
  await page.goto("/404");
  await expect(page.locator('meta[name="robots"]')).toHaveAttribute("content", "noindex");
});

test("sitemap.xml lists /, /method and /credits, robots.txt points to it", async ({ request }) => {
  const xml = await (await request.get("/sitemap.xml")).text();
  expect(xml).toContain("<loc>https://featherplay.candygoogy.dev/</loc>");
  expect(xml).toContain("<loc>https://featherplay.candygoogy.dev/method</loc>");
  expect(xml).toContain("<loc>https://featherplay.candygoogy.dev/credits</loc>");
  expect(xml).not.toContain("404");
  const robots = await (await request.get("/robots.txt")).text();
  expect(robots).toContain("Sitemap: https://featherplay.candygoogy.dev/sitemap.xml");
});

test("the sharing image is 1200 x 630, and every icon is the Tri-vane at its size", async ({ page, request }) => {
  const png = async (path: string) => {
    const res = await request.get(path);
    expect(res.ok()).toBe(true);
    const buf = await res.body();
    expect(buf.subarray(1, 4).toString()).toBe("PNG");
    return { w: buf.readUInt32BE(16), h: buf.readUInt32BE(20) };
  };
  // The card is a JPEG (a photographic frame): its size, as the browser decodes it.
  const og = await request.get("/og.jpg");
  expect(og.ok()).toBe(true);
  await page.goto("/");
  expect(
    await page.evaluate(async () => {
      const img = new Image();
      img.src = "/og.jpg";
      await img.decode();
      return { w: img.naturalWidth, h: img.naturalHeight };
    }),
  ).toEqual({ w: 1200, h: 630 });
  expect(await png("/apple-touch-icon.png")).toEqual({ w: 180, h: 180 });

  await page.goto("/");
  await expect(page.locator('link[rel="icon"][type="image/svg+xml"]')).toHaveAttribute("href", "/favicon.svg");
  await expect(page.locator('link[rel="icon"][sizes]')).toHaveAttribute("href", "/favicon.ico");
  await expect(page.locator('link[rel="apple-touch-icon"]')).toHaveAttribute("href", "/apple-touch-icon.png");
  const svg = await (await request.get("/favicon.svg")).text();
  expect(svg).toContain("Tri-vane");
  expect(svg).toContain("prefers-color-scheme: dark");
  // The ICO holds 16, 32 and 48 px entries.
  const ico = await (await request.get("/favicon.ico")).body();
  expect(ico.readUInt16LE(2)).toBe(1);
  const sizes = Array.from({ length: ico.readUInt16LE(4) }, (_, i) => ico.readUInt8(6 + 16 * i));
  expect(sizes).toEqual([16, 32, 48]);
});
