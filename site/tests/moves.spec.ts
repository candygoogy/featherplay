import { expect, test, type Page } from "@playwright/test";
import { laidOut } from "./helpers";

// The pieces mockup 10 played with Lottie, rebuilt as Web Animations with the files' timings
// (src/scripts/moves.ts): the mark at load and when a song starts, three of the bar's icons,
// the download arrow. No player is loaded, nothing loops, nothing runs at rest.

test.use({ viewport: { width: 1440, height: 900 } });

const running = (page: Page, sel: string) =>
  page.locator(sel).first().evaluate((el) => el.getAnimations({ subtree: true }).filter((a) => !(a instanceof CSSTransition) && a.playState === "running").length);

test("the mark comes together once at load, then rests as the drawing", async ({ page }) => {
  await page.route("**/music/**", (r) => r.abort());
  const scripts: string[] = [];
  page.on("request", (r) => r.resourceType() === "script" && scripts.push(r.url()));
  await page.goto("/");
  await expect.poll(() => running(page, "[data-site-bar] [data-trivane]")).toBeGreaterThan(0);
  await expect.poll(() => running(page, "[data-site-bar] [data-trivane]"), { timeout: 3000 }).toBe(0);
  const rest = await page.locator("[data-site-bar] [data-tv]:not([data-tv=ring])").evaluateAll((ls) => ls.map((l) => getComputedStyle(l)).map((c) => `${c.rotate} ${c.scale} ${c.translate} ${c.opacity}`));
  for (const r of rest) expect(r).toBe("none none none 1");
  expect(scripts.some((u) => /lottie/i.test(u))).toBe(false);
});

test("a song starting presses the mark's triangle; a pause doesn't", async ({ page }) => {
  await page.route("**/music/**", (r) => r.abort());
  await page.goto("/");
  await expect.poll(() => running(page, "[data-site-bar] [data-trivane]"), { timeout: 3000 }).toBe(0);
  await page.locator("[data-window] .bar .fp-play").click();
  await expect.poll(() => running(page, "[data-site-bar] [data-tv='play']")).toBeGreaterThan(0);
  await expect.poll(() => running(page, "[data-site-bar] [data-trivane]"), { timeout: 3000 }).toBe(0);
  await page.locator("[data-window] .bar .fp-play").click();
  await page.waitForTimeout(100);
  expect(await running(page, "[data-site-bar] [data-trivane]")).toBe(0);
});

test("the bar's icons move when the pointer comes to them, the question mark stays still", async ({ page }) => {
  await page.goto("/");
  for (const id of ["app", "light", "lyrics", "kept"]) {
    await page.mouse.move(5, 600);
    await page.locator(`[data-site-bar] a[data-sec="${id}"]`).hover();
    expect(await running(page, `[data-site-bar] svg[data-icon="${id}"]`), id).toBeGreaterThan(0);
  }
  await page.mouse.move(5, 600);
  await page.locator('[data-site-bar] a[data-sec="questions"]').hover();
  expect(await running(page, '[data-site-bar] svg[data-icon="questions"]')).toBe(0);
});

test("an icon moves once when its section first becomes the current one", async ({ page }) => {
  await page.goto("/");
  // (Once the sections above have their true heights: scrolled to before that, the target
  // lands wherever they then push it.)
  await laidOut(page);
  await page.evaluate(() => document.getElementById("lyrics")!.scrollIntoView());
  await expect(page.locator('[data-site-bar] a[data-sec="lyrics"]')).toHaveAttribute("aria-current", "true");
  expect(await running(page, '[data-site-bar] svg[data-icon="lyrics"]')).toBeGreaterThan(0);
  await page.evaluate(() => document.getElementById("questions")!.scrollIntoView());
  await page.waitForTimeout(900);
  await page.evaluate(() => document.getElementById("lyrics")!.scrollIntoView());
  await expect(page.locator('[data-site-bar] a[data-sec="lyrics"]')).toHaveAttribute("aria-current", "true");
  expect(await running(page, '[data-site-bar] svg[data-icon="lyrics"]')).toBe(0);
});

test("the download arrow drops into its tray on hover and on press, and rests drawn whole", async ({ page }) => {
  await page.goto("/");
  const dl = page.locator("[data-site-bar] [data-dl]");
  await dl.hover();
  expect(await running(page, "[data-site-bar] [data-dl] [data-dl-glyph]")).toBeGreaterThan(0);
  await expect.poll(() => running(page, "[data-site-bar] [data-dl] [data-dl-glyph]"), { timeout: 3000 }).toBe(0);
  const arrow = page.locator("[data-site-bar] [data-dl] .dl-arrow-part");
  expect(await arrow.evaluate((a) => [getComputedStyle(a).transform, getComputedStyle(a).opacity].join(" "))).toBe("none 1");
  // The first screen's and the Download section's buttons have it too.
  const hero = page.locator("[data-stage] .btn.red");
  await page.mouse.move(5, 600);
  await hero.dispatchEvent("pointerdown");
  expect(await running(page, "[data-stage] .btn.red [data-dl-glyph]")).toBeGreaterThan(0);
  expect(await page.locator("#download [data-dl-glyph]").count()).toBeGreaterThanOrEqual(3);
});

test("the theme toggle's half turns over as System comes round", async ({ page }) => {
  await page.goto("/");
  const half = page.locator("[data-site-bar] .b-theme .half");
  const toggle = page.locator("[data-site-bar] [data-theme-toggle]");
  await expect(toggle).toHaveAttribute("data-choice", "system");
  expect(await half.evaluate((h) => getComputedStyle(h).rotate)).toBe("none");
  await toggle.click();
  await expect.poll(() => half.evaluate((h) => getComputedStyle(h).rotate)).toBe("-180deg");
});

test.describe("reduced motion", () => {
  test.use({ reducedMotion: "reduce" });
  test("none of them plays", async ({ page }) => {
    await page.route("**/music/**", (r) => r.abort());
    await page.goto("/");
    await page.waitForTimeout(300);
    expect(await running(page, "[data-site-bar] [data-trivane]")).toBe(0);
    await page.locator('[data-site-bar] a[data-sec="light"]').hover();
    await page.locator("[data-site-bar] [data-dl]").hover();
    await page.locator("[data-window] .bar .fp-play").click();
    expect(await running(page, "[data-site-bar]")).toBe(0);
  });
});
