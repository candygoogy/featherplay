import { expect, test } from "@playwright/test";
import { wake } from "./helpers";
import { problems, waitSettled, WIDTHS } from "./check";

// The owner's three reports on mockup 10, made impossible: the bar's parts on top of each other
// in a smaller window (1), the narrow window's caption buttons stacked (2, tests/window.spec.ts),
// the window running off the right edge (3). The page is loaded fresh at the widths either side
// of every rule, from a phone to a 4K screen, at a short, a common and a tall height, and
// checked docked and as the capsule (tests/check.ts). Loaded fresh: bar.spec resizes instead.
for (const h of [600, 900, 1440]) {
  test(`loaded at every width from 320 to 3840 and ${h} px tall, the window and the bar fit and the bar's parts are apart`, async ({ page }) => {
    test.setTimeout(180_000);
    const found: string[] = [];
    for (const w of WIDTHS) {
      await page.setViewportSize({ width: w, height: h });
      await page.goto("/");
      await wake(page);
      found.push(...(await problems(page)));
      await page.evaluate(() => scrollTo(0, 3200));
      await expect(page.locator("[data-site-bar]")).toHaveAttribute("data-state", "capsule");
      await waitSettled(page);
      found.push(...(await problems(page)));
    }
    expect(found).toEqual([]);
  });
}

test("the odd ones: half a 2560 screen, a short wide window, a tall narrow one", async ({ page }) => {
  const found: string[] = [];
  for (const [w, h] of [
    [1280, 1270],
    [1270, 1300],
    [2560, 600],
    [3840, 2160],
    [761, 2000],
    [500, 600],
  ]) {
    await page.setViewportSize({ width: w, height: h });
    await page.goto("/");
    await wake(page);
    found.push(...(await problems(page)));
  }
  expect(found).toEqual([]);
});

test.describe("in light", () => {
  test.use({ colorScheme: "light" });
  test("the same sweep once in the light theme, docked and as the capsule", async ({ page }) => {
    test.setTimeout(120_000);
    const found: string[] = [];
    for (const w of WIDTHS) {
      await page.setViewportSize({ width: w, height: 900 });
      await page.goto("/");
      await wake(page);
      found.push(...(await problems(page)));
    }
    expect(found).toEqual([]);
  });
});
