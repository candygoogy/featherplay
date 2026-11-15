import { expect, test } from "@playwright/test";

// Review Focus 1: with JavaScript off (or failing to load) the page still says what it is,
// still offers the download, and every section is there to read.
test.use({ javaScriptEnabled: false });

const sectionHeadings = [
  "Light, and it shows",
  "Lyrics, up close",
  "Make it yours",
  "Everything you’d miss, kept",
  "Questions",
  "Download for Windows",
];

test("the headline, the download link and every section heading are there", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("h1")).toHaveText("Light enough to forget it’s running.");
  await expect(page.locator("[data-stage] .col .t-lede")).toContainText("featherplay is a free, native player");
  await expect(page.getByRole("link", { name: /download/i }).first()).toBeVisible();
  for (const name of sectionHeadings) {
    await expect(page.getByRole("heading", { level: 2, name, exact: true })).toBeVisible();
  }
});

test("the theme still follows the system without JavaScript", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "light" });
  await page.goto("/");
  const bg = await page.evaluate(() => getComputedStyle(document.body).backgroundColor);
  expect(bg).toBe("rgb(255, 255, 255)");
});

test("the app's window is a still to look at, not controls that do nothing; the bar offers nothing that needs script", async ({ page }) => {
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 844 });
    await page.goto("/");
    // Nothing in the window or the pieces takes the keys.
    const focusable = await page.evaluate(
      () =>
        [...document.querySelectorAll<HTMLElement>("[data-replica] :is(a, button, input), [data-piece] :is(a, button, input)")].filter((el) => !el.closest("[inert]") && el.getClientRects().length).length,
    );
    expect(focusable, `${width}`).toBe(0);
    await expect(page.locator("[data-site-bar] [data-np]")).toBeHidden();
    await expect(page.locator("[data-site-bar] [data-tray]")).toBeHidden();
  }
});

test.describe("with the page's scripts failing to load (boot.js ran)", () => {
  test.use({ javaScriptEnabled: true });
  test("the first screen is the page without script: no pinned stage, the window shown, the steps to read", async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    // The page's module scripts fail; the small boot script in the head still runs.
    await page.route(/\/_astro\/.*\.js$/, (r) => (/\/boot\./.test(r.request().url()) ? r.continue() : r.abort()));
    await page.goto("/", { waitUntil: "load" });
    await expect(page.locator("html")).not.toHaveAttribute("data-story", /.*/);
    const win = page.locator("[data-window]");
    await expect(win).toBeVisible();
    expect((await win.boundingBox())!.width).toBeGreaterThan(400);
    for (const h of ["A whole player when you want one.", "Narrow, beside your work.", "Or nothing at all."]) {
      await expect(page.getByRole("heading", { level: 2, name: h })).toBeVisible();
    }
    // No pinned stretch of empty page: the stage is about a screen tall.
    expect(await page.locator("[data-stage]").evaluate((s) => s.getBoundingClientRect().height)).toBeLessThan(1400);
  });
});
