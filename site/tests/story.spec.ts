import { expect, test, type Page } from "@playwright/test";
import { wake, windowBox } from "./helpers";
import { HOLDS } from "../src/scripts/story-math";

// The scroll story: the stage pinned, the window taking three holds (whole, narrow, in the
// tray) the page settles into; plain blocks where it doesn't run.
test.use({ viewport: { width: 1440, height: 900 } });

const LEN = 2400;
const at = async (page: Page, p: number) => {
  await page.evaluate((y) => scrollTo(0, y), Math.round(p * LEN));
  // The story's smoothing arrives, and the page has nothing to settle.
  await page.waitForTimeout(700);
};
const appWidth = (page: Page) => page.locator("[data-window] [data-replica]").evaluate((r) => r.clientWidth);
const capOpacity = (page: Page, i: number) => page.locator(`[data-cap="${i}"]`).evaluate((c) => Number(getComputedStyle(c).opacity));
const inViewport = async (page: Page, label: string) => {
  const b = await windowBox(page);
  const vp = page.viewportSize()!;
  expect(b.x, label).toBeGreaterThanOrEqual(-0.5);
  expect(b.y, label).toBeGreaterThanOrEqual(-0.5);
  expect(b.x + b.width, label).toBeLessThanOrEqual(vp.width + 0.5);
  expect(b.y + b.height, label).toBeLessThanOrEqual(vp.height + 0.5);
};

test("the window takes the three holds: whole and large, the app's real narrow layout, then into the tray with the song still playing", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  await expect(page.locator("html")).toHaveAttribute("data-story", "");
  const home = await windowBox(page);

  await at(page, HOLDS[0]);
  const whole = await windowBox(page);
  expect(whole.width).toBeGreaterThan(home.width);
  expect(Math.abs(whole.x + whole.width / 2 - 720)).toBeLessThan(2);
  expect(await appWidth(page)).toBeGreaterThanOrEqual(1100);
  expect(await capOpacity(page, 0)).toBe(1);
  await inViewport(page, "whole");

  await at(page, HOLDS[1]);
  expect(await appWidth(page)).toBe(480);
  await expect(page.locator("[data-window] [data-queue]")).toBeHidden();
  expect(await capOpacity(page, 1)).toBe(1);
  expect(await capOpacity(page, 0)).toBe(0);
  expect(await page.locator("[data-snap]").evaluate((s) => Number(getComputedStyle(s).opacity))).toBe(0);
  await inViewport(page, "narrow");
  // At rest in a hold the window is the app, there to use.
  expect(await page.locator("[data-window]").evaluate((w) => w.inert)).toBe(false);

  await at(page, HOLDS[2]);
  expect(await page.locator("[data-window]").evaluate((w) => Number(getComputedStyle(w).opacity))).toBe(0);
  expect(await page.locator("[data-window]").evaluate((w) => w.inert)).toBe(true);
  await expect(page.locator("#fly-song")).toBeVisible();
  expect(await capOpacity(page, 2)).toBe(1);

  await at(page, 0);
  const back = await windowBox(page);
  expect(Math.round(back.width)).toBe(Math.round(home.width));
  await expect(page.locator("#fly-song")).toBeHidden();
});

test("a scroll that stops part-way through a move is finished the way it was going", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  await page.mouse.move(700, 600);
  await page.evaluate(() => scrollTo(0, 0.21 * 2400));
  await page.waitForTimeout(400);
  await page.evaluate(() => scrollTo(0, 0.36 * 2400));
  await expect.poll(() => page.evaluate(() => scrollY), { timeout: 3000 }).toBe(HOLDS[1] * LEN);
  // The page's settling glide has ended before the next scroll; then one stop going up.
  await page.waitForTimeout(700);
  await page.evaluate(() => scrollTo(0, 0.38 * 2400));
  await expect.poll(() => page.evaluate(() => scrollY), { timeout: 3000 }).toBeLessThan(HOLDS[0] * LEN + 4);
});

for (const [w, h] of [
  [761, 600],
  [1280, 600],
  [1280, 720],
  [1920, 1080],
  [2560, 1270],
  [3840, 2160],
]) {
  test(`at ${w} x ${h} every hold's window lies wholly inside the viewport`, async ({ page }) => {
    await page.setViewportSize({ width: w, height: h });
    await page.goto("/");
    await wake(page);
    for (const p of [0, HOLDS[0], HOLDS[1]]) {
      await at(page, p);
      await inViewport(page, `${w}x${h} at ${p}`);
    }
  });
}

test("a dragged window joins the story from where it was put, and is home again back at the top", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  const home = await windowBox(page);
  const t = (await page.locator("[data-window] .titlebar-name").boundingBox())!;
  await page.mouse.move(t.x + 5, t.y + 5);
  await page.mouse.down();
  await page.mouse.move(t.x - 300, t.y + 40, { steps: 5 });
  await page.mouse.up();
  expect((await windowBox(page)).x).toBeLessThan(home.x - 200);
  await at(page, HOLDS[0]);
  const whole = await windowBox(page);
  expect(Math.abs(whole.x + whole.width / 2 - 720)).toBeLessThan(2);
  await at(page, 0);
  expect(Math.round((await windowBox(page)).x)).toBe(Math.round(home.x));
});

test("a window sent to the tray by the visitor comes back as the page moves on", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  await page.locator("[data-window]").getByRole("button", { name: /^Minimise/ }).click();
  await expect(page.locator("[data-window]")).toHaveAttribute("data-minimised", "");
  await at(page, HOLDS[0]);
  await expect(page.locator("[data-window]")).not.toHaveAttribute("data-minimised", /.*/);
  await expect(page.locator("[data-window]")).toBeVisible();
});

test('"The app" in the bar goes to the whole window', async ({ page }) => {
  await page.goto("/");
  await page.locator("[data-site-bar]").getByRole("link", { name: "The app" }).click();
  await expect.poll(() => page.evaluate(() => scrollY), { timeout: 3000 }).toBeGreaterThan(HOLDS[0] * LEN - 4);
});

for (const [label, use] of [
  ["a phone", { viewport: { width: 390, height: 844 } }],
  ["a short window", { viewport: { width: 1280, height: 560 } }],
  ["reduced motion", { reducedMotion: "reduce" as const }],
] as const) {
  test.describe(label, () => {
    test.use(use);
    test("no pinned story: the three steps are plain blocks to read, the window stays put", async ({ page }) => {
      await page.goto("/");
      await wake(page);
      await expect(page.locator("html")).not.toHaveAttribute("data-story", /.*/);
      for (const h of ["A whole player when you want one.", "Narrow, beside your work.", "Or nothing at all."]) {
        await expect(page.getByRole("heading", { level: 2, name: h })).toBeVisible();
      }
      const before = await windowBox(page);
      await page.evaluate(() => scrollTo(0, 300));
      await page.waitForTimeout(300);
      const after = await windowBox(page);
      expect(Math.round(after.y)).toBe(Math.round(before.y - 300));
    });
  });
}
