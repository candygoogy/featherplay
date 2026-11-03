import { expect, test } from "@playwright/test";
import { statSync } from "node:fs";

const cf = "http://127.0.0.1:8788";

test("an unknown path returns status 404 and the 404 page (on Cloudflare's runtime)", async ({ page }) => {
  const res = await page.goto(`${cf}/no/such/page`);
  expect(res?.status()).toBe(404);
  await expect(page.locator("h1")).toHaveText("Not in the queue");
  await expect(page.locator("[data-nf-title]")).toHaveText("404");
  await expect(page.locator("[data-nf-artist]")).toHaveText("page not found");
  await expect(page.locator("[data-nf-time]")).toHaveText("0:00");
  await expect(page.locator("[data-nf-length]")).toHaveText("4:04");
  await expect(page.getByText("This page isn't in the queue.")).toBeVisible();
  const home = await page.goto(`${cf}/`);
  expect(home?.status()).toBe(200);
});

test("Play from the start goes home", async ({ page }) => {
  await page.goto("/404");
  await page.getByRole("link", { name: "Play from the start" }).click();
  await expect(page).toHaveURL(/\/$/);
  await expect(page.locator("h1")).toContainText("featherplay");
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
