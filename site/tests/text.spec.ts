import { expect, test } from "@playwright/test";

// Text the visitor reads must never show an escape that didn't turn into its character
// ("YouTube00a0Music", " ", "&nbsp;").
const escapes = /\\u[0-9a-f]{4}|00a0|&(?:nbsp|#160|#xa0);/i;

for (const path of ["/", "/method"]) {
  test(`no visible text on ${path} carries a raw escape`, async ({ page }) => {
    await page.goto(path);
    const text = await page.evaluate(() => document.body.innerText);
    expect(text.length).toBeGreaterThan(200);
    expect(text).not.toMatch(escapes);
    const title = await page.title();
    expect(title).not.toMatch(escapes);
  });
}

test("the hero line keeps \"YouTube Music\" together on one line", async ({ page }) => {
  for (const width of [390, 1024, 1440]) {
    await page.setViewportSize({ width, height: 900 });
    await page.goto("/");
    const line = page.locator(".hero-line");
    await expect(line).toContainText("for YouTube Music, light enough");
    const rects = await line.locator(".rise", { hasText: "YouTube Music" }).evaluate((el) => el.getClientRects().length);
    expect(rects, `at ${width} px`).toBe(1);
  }
});
