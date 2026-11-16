import { expect, test, type Page } from "@playwright/test";
import { wake } from "./helpers";

// The page for the keyboard and the ear: the ways past things, what is said aloud, the
// capsule's words over the moving page, the taskbar clock.
test.use({ viewport: { width: 1440, height: 900 } });

/** Tabs until `name` has the focus (at most 40 presses). */
async function tabTo(page: Page, name: string) {
  for (let i = 0; i < 40; i++) {
    await page.keyboard.press("Tab");
    if ((await page.evaluate(() => document.activeElement?.textContent?.trim())) === name) return;
  }
  throw new Error(`never reached ${name}`);
}

test("Skip to the page takes the keys past the bar to the page", async ({ page }) => {
  await page.goto("/");
  await page.keyboard.press("Tab");
  const skip = page.getByRole("link", { name: "Skip to the page" });
  await expect(skip).toBeFocused();
  await expect(skip).toBeInViewport();
  await page.keyboard.press("Enter");
  await expect(page.locator("main")).toBeFocused();
});

test("Skip past the app shows where it can be seen when it has the focus, not under the bar", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  await tabTo(page, "Skip past the app");
  const hit = await page.evaluate(() => {
    const a = document.activeElement as HTMLElement;
    const r = a.getBoundingClientRect();
    return document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2) === a;
  });
  expect(hit).toBe(true);
});

test("what is said aloud: the song once as it plays, not the scrobble row's every change", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("[data-announce][role=status]")).toHaveCount(1);
  // The scrobble row changes on every play, every song and part-way through each: it is shown,
  // not said.
  await expect(page.locator("[data-piece=scrobble] [role=status], [data-piece=scrobble] [aria-live]")).toHaveCount(0);
});

// The capsule's section names over the page: measured on the real pixels in glass.spec.ts.

test("the taskbar clock shows the visitor's time and moves on with the minute", async ({ page }) => {
  await page.clock.install({ time: new Date(2026, 10, 15, 22, 9, 30) });
  await page.goto("/");
  const clock = page.locator("[data-site-bar] [data-clock-time]");
  const want = (h: number, m: number) => page.evaluate(([h, m]) => new Date(2026, 10, 15, h, m).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" }), [h, m]);
  await expect(clock).toHaveText(await want(22, 9));
  await page.clock.runFor(31_000);
  await expect(clock).toHaveText(await want(22, 10));
});
