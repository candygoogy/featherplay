import { expect, test, type Page } from "@playwright/test";
import { wake } from "./helpers";

// The first screen (Stage.astro, src/scripts/first.ts): the study's composition, the shelf of
// sleeves, its two notes, the arrival, and the desk's frosted glass behind it (Desk.astro).

const box = (page: Page, sel: string) => page.locator(sel).first().evaluate((e) => e.getBoundingClientRect().toJSON() as DOMRect);
const shown = (page: Page) => expect(page.locator("[data-first]")).toHaveAttribute("data-shown", "");
const apart = (a: DOMRect, b: DOMRect) => a.right <= b.left + 0.5 || b.right <= a.left + 0.5 || a.bottom <= b.top + 0.5 || b.bottom <= a.top + 0.5;

test.describe("at 2560 x 1270", () => {
  test.use({ viewport: { width: 2560, height: 1270 } });
  test("the window opens from the middle of the headline's second line, on the frame's right edge, whole; the shelf under the words, level with its foot", async ({ page }) => {
    await page.goto("/");
    await shown(page);
    await wake(page);
    await page.waitForTimeout(1400);
    const [l2, win, shelf, col, brand, dl, head] = await Promise.all([
      box(page, "[data-head] .line:nth-child(2)"),
      box(page, "[data-window]"),
      box(page, "[data-shelf]"),
      box(page, "[data-col]"),
      box(page, "[data-site-bar] .brand"),
      box(page, "[data-site-bar] [data-dl]"),
      box(page, "[data-head]"),
    ]);
    expect(Math.abs(win.top - (l2.top + l2.height / 2))).toBeLessThan(8);
    expect(win.right).toBeLessThanOrEqual(2560);
    expect(win.bottom).toBeLessThanOrEqual(1270);
    expect(win.width).toBeGreaterThan(1150);
    // The docked bar's edges are the composition's: its mark over the headline, Download over the window's right edge.
    expect(Math.abs(brand.left + 9 - head.left)).toBeLessThan(12);
    expect(Math.abs(dl.right - win.right)).toBeLessThan(4);
    // The shelf: three by two, under the words, its foot level with the window's.
    expect(shelf.top).toBeGreaterThan(col.bottom);
    expect(Math.abs(shelf.bottom - win.bottom)).toBeLessThan(6);
    expect(Math.abs(shelf.left - head.left)).toBeLessThan(2);
    const tops = await page.locator("[data-shelf] .sleeve").evaluateAll((s) => new Set(s.map((e) => Math.round(e.getBoundingClientRect().top / 40))).size);
    expect(tops).toBe(2);
    // "not a screenshot" over the window, in free ground; "drop one on the player" over the shelf.
    for (const id of ["screenshot", "sleeves"]) {
      const n = page.locator(`[data-note='${id}']`);
      await expect(n).toHaveAttribute("data-on", "");
      const words = await n.locator("span").evaluate((e) => e.getBoundingClientRect().toJSON() as DOMRect);
      for (const b of [win, shelf, col, l2]) expect(apart(words, b), id).toBe(true);
    }
  });
});

test("in a tall room (half a 2560 screen) the words and the shelf sit side by side, the window beneath", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 1270 });
  await page.goto("/");
  await shown(page);
  await expect(page.locator("[data-first]")).toHaveAttribute("data-laid", "tall");
  const [col, shelf, win] = await Promise.all([box(page, "[data-col]"), box(page, "[data-shelf]"), box(page, "[data-window]")]);
  expect(shelf.left).toBeGreaterThan(col.right);
  expect(win.top).toBeGreaterThanOrEqual(shelf.bottom);
  expect(win.bottom).toBeLessThanOrEqual(1270);
});

test("at 1280 x 720, where the wide app's lyrics would fall under 13 px, the window is the app's narrow layout", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 720 });
  await page.goto("/");
  await shown(page);
  await wake(page);
  const app = await page.locator("[data-window]").evaluate((w) => ({ w: w.offsetWidth, s: w.getBoundingClientRect().width / w.offsetWidth }));
  expect(app.w).toBeLessThanOrEqual(720);
  // Its lyrics (20 or 22 app px) at 13 px or more on the page.
  expect((app.w < 600 ? 20 : 22) * app.s).toBeGreaterThanOrEqual(13);
  const [col, shelf, win] = await Promise.all([box(page, "[data-col]"), box(page, "[data-shelf]"), box(page, "[data-window]")]);
  expect(apart(col, shelf)).toBe(true);
  expect(apart(shelf, win)).toBe(true);
});

test("on a phone: one column, the words, the window, then the shelf", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/");
  await shown(page);
  await expect(page.locator("[data-first]")).toHaveAttribute("data-laid", "stack");
  const [l1, col, win, shelf] = await Promise.all([box(page, "[data-head] .line"), box(page, "[data-col]"), box(page, "[data-window]"), box(page, "[data-shelf]")]);
  // The headline's first line fits the width.
  expect(l1.right).toBeLessThanOrEqual(390);
  expect(win.top).toBeGreaterThan(col.bottom);
  expect(shelf.top).toBeGreaterThan(win.bottom);
  expect(shelf.right).toBeLessThanOrEqual(390);
});

test.describe("the sleeves", () => {
  test.use({ viewport: { width: 1920, height: 1080 } });
  test.beforeEach(async ({ page }) => {
    await page.route("**/music/**", (r) => r.abort());
    await page.goto("/");
    await shown(page);
    await wake(page);
    await page.waitForTimeout(1300);
  });
  const title = (page: Page) => page.locator("[data-window] [data-bar-title]");

  test("Enter on one plays it, and it is marked as the one playing; its title shows on focus", async ({ page }) => {
    const s = page.locator("[data-sleeve='origami']");
    await s.focus();
    await expect(s.locator(".sl-tip")).toHaveCSS("opacity", "1");
    await page.keyboard.press("Enter");
    await expect(title(page)).toHaveText("Origami");
    await expect(page.locator("html")).toHaveAttribute("data-playing", "true");
    await expect(s).toHaveAttribute("data-current", "");
    await expect(page.locator("[data-sleeve][data-current]")).toHaveCount(1);
  });

  test("a single click only picks it up; a double-click plays it", async ({ page }) => {
    const s = page.locator("[data-sleeve='invisible-light']");
    await s.click();
    await page.waitForTimeout(300);
    await expect(page.locator("html")).not.toHaveAttribute("data-playing", "true");
    await s.dblclick();
    await expect(title(page)).toHaveText("Invisible Light");
  });

  test("dropped on the player it plays; the window rings while it is over it; dropped elsewhere it goes back", async ({ page }) => {
    const s = page.locator("[data-sleeve='watching-seabirds-soar']");
    const from = (await s.boundingBox())!;
    const win = (await page.locator("[data-window]").boundingBox())!;
    // Dropped on the empty desk first: nothing.
    await page.mouse.move(from.x + 20, from.y + 20);
    await page.mouse.down();
    await page.mouse.move(from.x + 60, from.y - 40, { steps: 5 });
    await page.mouse.up();
    await page.waitForTimeout(400);
    await expect(page.locator("html")).not.toHaveAttribute("data-playing", "true");
    // Then onto the window.
    await page.mouse.move(from.x + 20, from.y + 20);
    await page.mouse.down();
    await page.mouse.move(win.x + win.width / 2, win.y + win.height / 2, { steps: 8 });
    await expect(page.locator("[data-window]")).toHaveAttribute("data-drop", "");
    await page.mouse.up();
    await expect(page.locator("[data-window]")).not.toHaveAttribute("data-drop", "");
    await expect(title(page)).toHaveText("Watching Seabirds Soar (Harmony)");
    await expect(page.locator(".carry")).toHaveCount(0);
  });
});

test("the notes step aside while the window is away from home (dragged)", async ({ page }) => {
  await page.setViewportSize({ width: 2560, height: 1270 });
  await page.goto("/");
  await shown(page);
  await wake(page);
  await page.waitForTimeout(1400);
  const note = page.locator("[data-note='screenshot']");
  await expect(note).toHaveCSS("opacity", "1");
  const t = (await page.locator("[data-window] .titlebar-name").boundingBox())!;
  await page.mouse.move(t.x + 5, t.y + 5);
  await page.mouse.down();
  await page.mouse.move(t.x - 200, t.y + 40, { steps: 6 });
  await page.mouse.up();
  await expect(note).toHaveCSS("opacity", "0");
});

test("the arrival plays once at load: the headline line by line, the window opening, the sleeves landing", async ({ page }) => {
  await page.setViewportSize({ width: 1920, height: 1080 });
  await page.goto("/");
  await shown(page);
  const running = await page.evaluate(() => ({
    lines: [...document.querySelectorAll("[data-head] .line")].filter((l) => l.getAnimations().length).length,
    window: document.querySelector("[data-window]")!.getAnimations().length,
    sleeves: [...document.querySelectorAll("[data-shelf] .sleeve")].filter((s) => s.getAnimations().length).length,
  }));
  expect(running.lines).toBe(2);
  expect(running.window).toBeGreaterThan(0);
  expect(running.sleeves).toBe(6);
  // And then the page is still.
  await expect.poll(() => page.evaluate(() => document.querySelector("[data-stage]")!.getAnimations({ subtree: true }).filter((a) => !(a instanceof CSSTransition) && a.playState === "running").length), { timeout: 4000 }).toBe(0);
});

test.describe("reduced motion", () => {
  test.use({ reducedMotion: "reduce", viewport: { width: 1920, height: 1080 } });
  test("the first screen is simply there: no arrival (the app's own short fades inside the window aside)", async ({ page }) => {
    await page.goto("/");
    await shown(page);
    const n = await page.evaluate(() => document.querySelector("[data-stage]")!.getAnimations({ subtree: true }).filter((a) => !(a instanceof CSSTransition) && !(a.effect as KeyframeEffect).target!.closest("[data-replica]")).map((a) => `${(a as CSSAnimation).animationName ?? "script"} ${(a.effect as KeyframeEffect).target!.className}`));
    expect(n).toEqual([]);
    await expect(page.locator("[data-note='screenshot']")).toHaveAttribute("data-on", "");
  });
});

test.describe("the desk", () => {
  test.use({ viewport: { width: 1440, height: 900 } });
  test("a song's colour comes in by fading the second slot over the first, 900 ms, nothing animating inside the masked layers", async ({ page }) => {
    await page.route("**/music/**", (r) => r.abort());
    await page.goto("/");
    await shown(page);
    const front = () => page.locator("[data-desk] [data-slot].front").getAttribute("data-key");
    expect(await front()).toBe("dark:#E8894A");
    await page.evaluate(() => document.querySelector<HTMLElement>('#fly-song [data-song="invisible-light"]')!.click());
    // The new tones are painted out of sight first, then the fade starts.
    await expect(page.locator("[data-desk] [data-slot].fading.in")).toHaveCount(1);
    const fade = await page.evaluate(() => {
      const f = document.querySelector("[data-desk] [data-slot].fading")!;
      const anims = document.querySelector("[data-desk]")!.getAnimations({ subtree: true }).filter((a) => a instanceof CSSTransition);
      return { duration: getComputedStyle(f).transitionDuration, props: [...new Set(anims.map((a) => (a as CSSTransition).transitionProperty))] };
    });
    expect(fade.duration).toBe("0.9s");
    expect(fade.props).toEqual(["opacity"]);
    await expect.poll(front, { timeout: 2000 }).toBe("dark:#3E4FA8");
  });
  test("the warm opening song's dark desk falls nearer to black away from its lit corner than the study's did", async ({ page }) => {
    await page.goto("/");
    await shown(page);
    const ground = await page.locator("[data-desk] [data-slot].front .k-ground").evaluate((g) => getComputedStyle(g).backgroundColor);
    // oklch(L C h): the study's ground was L 0.15; warmth takes it down to 0.09.
    const L = Number(/oklch\(([\d.]+)/.exec(ground)![1]);
    expect(L).toBeLessThan(0.11);
  });
  test("the theme switch fades between the themes' own tones", async ({ page }) => {
    await page.goto("/");
    await shown(page);
    await page.locator("[data-site-bar] [data-theme-toggle]").click();
    await expect(page.locator("[data-desk] [data-slot].fading")).toHaveCount(1);
    await expect.poll(() => page.locator("[data-desk] [data-slot].front").getAttribute("data-tones"), { timeout: 2000 }).toBe("light");
  });
});
