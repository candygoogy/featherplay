import { expect, test, type Page } from "@playwright/test";
import { wake, windowBox } from "./helpers";
import { HOLDS } from "../src/scripts/story-math";

// The scroll story: the stage pinned, the window taking three holds (whole, narrow, in the
// tray) the page settles into; plain blocks where it doesn't run.
test.use({ viewport: { width: 1440, height: 900 } });

const LEN = 2400;
/** Scrolls to `p` and waits until the story has arrived there: the page where it was sent and
 *  the window drawn at the same place a quarter of a second apart, with no glide running (no
 *  fixed wait: polled). */
const at = async (page: Page, p: number) => {
  const y = Math.round(p * LEN);
  await page.evaluate((y) => scrollTo(0, y), y);
  await expect.poll(() => page.evaluate(() => scrollY)).toBe(y);
  await expect
    .poll(() =>
      page.evaluate(
        () =>
          new Promise<boolean>((r) => {
            const el = document.querySelector<HTMLElement>("[data-window]")!;
            const read = () => `${el.style.transform}|${el.style.opacity}|${el.style.width}`;
            const a = read();
            setTimeout(() => r(read() === a && !el.getAnimations().length), 250);
          }),
      ),
    )
    .toBe(true);
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

test("where the first screen shows the app's narrow layout (1280 x 720), hold 1 shows the whole wide app, by the snap, and the narrow one is home again at the top", async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 720 });
  await page.goto("/");
  await wake(page);
  const home = await appWidth(page);
  expect(home).toBeLessThanOrEqual(720);
  // Part-way through the first move: the app faded, its ground carrying its icon.
  await page.evaluate((y) => scrollTo(0, y), Math.round(0.08 * LEN));
  await expect.poll(() => page.locator("[data-ghost]").evaluate((g) => Number(getComputedStyle(g).opacity))).toBeGreaterThan(0.5);
  await at(page, HOLDS[0]);
  expect(await appWidth(page)).toBe(1440);
  await inViewport(page, "hold 1 wide at 1280 x 720");
  // Its lyrics at 13 px or more.
  const s = await page.locator("[data-window]").evaluate((w) => w.getBoundingClientRect().width / w.offsetWidth);
  expect(24 * s).toBeGreaterThanOrEqual(13);
  await at(page, 0);
  await expect.poll(() => appWidth(page)).toBe(home);
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

test('"The app" in the bar goes to the whole window, and the keys go with it', async ({ page }) => {
  await page.goto("/");
  await page.locator("[data-site-bar]").getByRole("link", { name: "The app" }).click();
  await expect.poll(() => page.evaluate(() => scrollY), { timeout: 3000 }).toBeGreaterThan(HOLDS[0] * LEN - 4);
  await expect(page.locator("[data-stage]")).toBeFocused();
});

test("the story's flyout in the tray goes when the story stops (the screen made narrow there)", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  await at(page, HOLDS[2]);
  await expect(page.locator("#fly-song")).toBeVisible();
  await page.setViewportSize({ width: 700, height: 900 });
  await expect(page.locator("html")).not.toHaveAttribute("data-story", /.*/);
  await expect(page.locator("#fly-song")).toBeHidden();
});

test("the tray's flyout stays through its beat, though the pointer left the tray icon as the bar became the capsule", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  // The window sent to the tray and back by the tray icon, the pointer left resting on it.
  const tray = page.locator("[data-tray]");
  await tray.click();
  await page.mouse.move(10, 500);
  await tray.click();
  await at(page, HOLDS[2]);
  await page.waitForTimeout(600);
  await expect(page.locator("#fly-song")).toBeVisible();
  // And "still playing" beside it, wholly on the screen and clear of it.
  const note = page.locator("[data-note='playing']");
  await expect(note).toHaveAttribute("data-on", "");
  const [n, f] = [(await note.locator("span").boundingBox())!, (await page.locator("#fly-song").boundingBox())!];
  expect(n.x).toBeGreaterThan(f.x + f.width);
  expect(n.x + n.width).toBeLessThanOrEqual(1440);
});

test("the keys in the first screen's words, as they leave, go on to the steps rather than to nowhere", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  await page.locator("[data-stage] .actions a").first().focus();
  await at(page, 0.1);
  expect(await page.evaluate(() => document.activeElement?.closest("[data-steps]") !== null)).toBe(true);
});

test("a drag the page scrolls under is let go: the story draws the window", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  const t = (await page.locator("[data-window] .titlebar-name").boundingBox())!;
  await page.mouse.move(t.x + 5, t.y + 5);
  await page.mouse.down();
  await page.mouse.move(t.x - 40, t.y + 20, { steps: 3 });
  await at(page, HOLDS[0]);
  await page.mouse.move(t.x - 300, t.y + 200, { steps: 4 });
  await expect(page.locator("[data-window]")).not.toHaveAttribute("data-dragging", /.*/);
  const whole = await windowBox(page);
  expect(Math.abs(whole.x + whole.width / 2 - 720)).toBeLessThan(2);
  await page.mouse.up();
});

test("a reload part-way through a move is finished the way it was going", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  await page.evaluate(() => scrollTo(0, Math.round(0.4 * 2400)));
  await page.waitForTimeout(100);
  await page.reload();
  await expect.poll(() => page.evaluate(() => scrollY), { timeout: 5000 }).toBe(HOLDS[1] * LEN);
});

for (const [w, h] of [
  [1000, 620],
  [1280, 600],
  [761, 650],
]) {
  test(`at ${w} x ${h} the whole window's words sit under it, inside the screen`, async ({ page }) => {
    await page.setViewportSize({ width: w, height: h });
    await page.goto("/");
    await wake(page);
    await at(page, HOLDS[0]);
    const win = await windowBox(page);
    const cap = (await page.locator('[data-cap="0"]').boundingBox())!;
    expect(cap.y).toBeGreaterThanOrEqual(win.y + win.height - 0.5);
    expect(cap.y + cap.height).toBeLessThanOrEqual(h + 0.5);
  });
}

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

for (const [w, h] of [[761, 600], [800, 600], [900, 800], [1000, 1024]] as const) {
  test(`in the tray's hold the song's flyout leaves the hold's words clear (${w}x${h})`, async ({ page }) => {
    await page.setViewportSize({ width: w, height: h });
    await page.goto("/");
    await wake(page);
    // Straight there from the top, the bar still docked as the story starts to move.
    await page.waitForTimeout(800);
    await at(page, HOLDS[2]);
    await expect(page.locator("#fly-song")).toBeVisible();
    const f = (await page.locator("#fly-song").boundingBox())!;
    // Every line of the hold's words, as drawn.
    const boxes = await page.locator('[data-cap="2"]').evaluate((cap) =>
      [...cap.querySelectorAll("h2, p")].flatMap((el) => {
        const r = document.createRange();
        r.selectNodeContents(el);
        return [...r.getClientRects()].map((b) => ({ x: b.x, y: b.y, w: b.width, h: b.height }));
      }),
    );
    expect(boxes.length).toBeGreaterThan(1);
    for (const b of boxes) {
      const meet = b.x < f.x + f.width && b.x + b.w > f.x && b.y < f.y + f.height && b.y + b.h > f.y;
      expect(meet, `${JSON.stringify(b)} under the flyout ${JSON.stringify(f)}`).toBe(false);
    }
  });
}
