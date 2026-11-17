import { expect, test } from "@playwright/test";
import { statSync } from "node:fs";
import { wranglerPort } from "../playwright.config";

const cf = `http://127.0.0.1:${wranglerPort}`;

test("an unknown path returns status 404 and the 404 page (on Cloudflare's runtime)", async ({ page }) => {
  const res = await page.goto(`${cf}/no/such/page`);
  expect(res?.status()).toBe(404);
  await expect(page.locator("h1")).toHaveText("Not in the queue");
  await expect(page.locator("[data-nf-code]")).toHaveText("404");
  await expect(page.getByText("The page you asked for isn’t here.")).toBeVisible();
  const home = await page.goto(`${cf}/`);
  expect(home?.status()).toBe(200);
});

test("its two ways on: back to featherplay, and the download", async ({ page }) => {
  await page.goto("/404");
  const main = page.locator("main");
  await expect(main.getByRole("link", { name: "Download" })).toHaveAttribute("href", "/#download");
  await main.getByRole("link", { name: "Back to featherplay" }).click();
  await expect(page).toHaveURL(/\/$/);
  await expect(page.locator("h1#hero-title")).toBeVisible();
});

test("the 404 page is light (its HTML, CSS and script under 100 KB; the budget script checks the rest)", async ({ page }) => {
  const bytes: number[] = [];
  page.on("response", async (r) => {
    if (r.url().startsWith("http://localhost")) bytes.push((await r.body().catch(() => Buffer.alloc(0))).length);
  });
  await page.goto("/404");
  await page.waitForLoadState("networkidle");
  const total = bytes.reduce((a, b) => a + b, 0);
  expect(total).toBeLessThan(400 * 1024); // uncompressed, fonts and art included
  expect(statSync("dist/404.html").size).toBeLessThan(100 * 1024);
});

test("on a tall screen the 404's footer ends at the window's foot, no desk under it", async ({ page }) => {
  for (const [width, height] of [[1920, 1080], [2560, 1270]]) {
    await page.setViewportSize({ width, height });
    await page.goto("/404");
    await page.evaluate(() => scrollTo(0, document.documentElement.scrollHeight));
    const under = await page.evaluate(() => innerHeight - document.querySelector("footer.foot")!.getBoundingClientRect().bottom);
    expect(Math.abs(under), `${width}x${height}`).toBeLessThan(1);
  }
});
