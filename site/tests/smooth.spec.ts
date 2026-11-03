import { expect, test, type Page } from "@playwright/test";

// A mouse wheel glides the page (src/scripts/smooth.ts): each notch eases in over a few
// frames instead of jumping. Everything else keeps the browser's own scrolling: a trackpad's
// fine deltas, touch, the keyboard, reduced motion. The wheel over a lyrics panel is still the
// panel's, the hero's moment follows the glide frame by frame, a link within the page still
// lands where it points, and a reload part-way down still comes back without a replay.

test.use({ viewport: { width: 1440, height: 900 } });

/** Starts recording scrollY and whether the page is gliding, every frame. */
async function sample(page: Page) {
  await page.evaluate(() => {
    const w = window as any;
    w.__ys = [] as number[];
    w.__glided = false;
    const tick = () => {
      w.__ys.push(scrollY);
      if (document.documentElement.hasAttribute("data-gliding")) w.__glided = true;
      w.__raf = requestAnimationFrame(tick);
    };
    w.__raf = requestAnimationFrame(tick);
  });
}
async function samples(page: Page): Promise<{ ys: number[]; glided: boolean }> {
  return page.evaluate(() => {
    const w = window as any;
    cancelAnimationFrame(w.__raf);
    return { ys: w.__ys, glided: w.__glided };
  });
}
/** Positions strictly between where it started and where it ended. */
const between = (ys: number[]) => {
  const a = ys[0];
  const b = ys.at(-1)!;
  const [lo, hi] = a < b ? [a, b] : [b, a];
  return new Set(ys.filter((y) => y > lo + 0.5 && y < hi - 0.5).map((y) => Math.round(y))).size;
};

/** Below the hero, over plain page (the comparison), the pointer in the middle. */
async function midPage(page: Page) {
  await page.goto("/");
  await page.evaluate(() => {
    const light = document.getElementById("light")!;
    scrollTo({ top: light.getBoundingClientRect().top + scrollY, behavior: "instant" });
  });
  await page.waitForTimeout(300);
  await page.mouse.move(720, 450);
}

test("a mouse wheel notch glides the page there over several frames", async ({ page }) => {
  await midPage(page);
  await sample(page);
  const y0 = await page.evaluate(() => scrollY);
  await page.mouse.wheel(0, 200);
  await page.waitForTimeout(900);
  const { ys, glided } = await samples(page);
  expect(glided).toBe(true);
  expect(ys.at(-1)!).toBeCloseTo(y0 + 200, 0);
  expect(between(ys)).toBeGreaterThanOrEqual(5);
  // Always on the way, never back.
  for (let i = 1; i < ys.length; i++) expect(ys[i]).toBeGreaterThanOrEqual(ys[i - 1] - 0.5);
  await expect(page.locator("html")).not.toHaveAttribute("data-gliding");
});

test("notches in a row add up and keep gliding the same way", async ({ page }) => {
  await midPage(page);
  const y0 = await page.evaluate(() => scrollY);
  for (let i = 0; i < 3; i++) await page.mouse.wheel(0, 100);
  await expect.poll(() => page.evaluate(() => scrollY)).toBeCloseTo(y0 + 300, 0);
});

test("a trackpad's fine deltas keep the browser's own scrolling", async ({ page }) => {
  await midPage(page);
  await sample(page);
  for (let i = 0; i < 6; i++) await page.mouse.wheel(0, 7.5);
  await page.waitForTimeout(300);
  const { glided } = await samples(page);
  expect(glided).toBe(false);
});

test.describe("on a touch screen", () => {
  test.use({ hasTouch: true });
  test("a swipe keeps the browser's own scrolling", async ({ page }) => {
  await midPage(page);
  await sample(page);
  const y0 = await page.evaluate(() => scrollY);
  const cdp = await page.context().newCDPSession(page);
  const touch = (type: string, y?: number) =>
    cdp.send("Input.dispatchTouchEvent", { type, touchPoints: y === undefined ? [] : [{ x: 720, y }] });
  await touch("touchStart", 700);
  for (let y = 680; y >= 380; y -= 20) await touch("touchMove", y);
  await touch("touchEnd");
  await page.waitForTimeout(300);
  const { glided } = await samples(page);
  expect(glided).toBe(false);
  expect(await page.evaluate(() => scrollY)).toBeGreaterThan(y0 + 100);
});
});

test.describe("reduced motion", () => {
  test.use({ reducedMotion: "reduce" });
  test("the wheel jumps as the browser's own does", async ({ page }) => {
    await midPage(page);
    await sample(page);
    const y0 = await page.evaluate(() => scrollY);
    await page.mouse.wheel(0, 200);
    await page.waitForTimeout(400);
    const { ys, glided } = await samples(page);
    expect(glided).toBe(false);
    expect(ys.at(-1)!).toBeCloseTo(y0 + 200, 0);
    expect(between(ys)).toBe(0);
  });
});

test("the wheel over a lyrics panel moves its lines, and the page stays", async ({ page }) => {
  await page.goto("/");
  const panel = page.locator("#lyrics [data-showcase-panel]").first();
  await panel.evaluate((el) => el.scrollIntoView({ block: "center", behavior: "instant" }));
  await page.waitForTimeout(700);
  const box = (await panel.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  const y0 = await page.evaluate(() => scrollY);
  await sample(page);
  await page.mouse.wheel(0, 120);
  await page.waitForTimeout(600);
  const { glided } = await samples(page);
  expect(glided).toBe(false);
  expect(await page.evaluate(() => scrollY)).toBe(y0);
});

test("the hero's moment follows the glide: the laptop only grows on the way down", async ({ page }) => {
  await page.goto("/");
  await page.waitForTimeout(1200); // the lid's opening
  await page.mouse.move(720, 450);
  await page.evaluate(() => {
    const w = window as any;
    const stage = document.querySelector<HTMLElement>("[data-hero-stage]")!;
    w.__scales = [] as number[];
    const tick = () => {
      w.__scales.push(new DOMMatrixReadOnly(getComputedStyle(stage).transform).a);
      w.__raf = requestAnimationFrame(tick);
    };
    w.__raf = requestAnimationFrame(tick);
  });
  for (let i = 0; i < 4; i++) {
    await page.mouse.wheel(0, 100);
    await page.waitForTimeout(60);
  }
  await page.waitForTimeout(900);
  const scales: number[] = await page.evaluate(() => {
    cancelAnimationFrame((window as any).__raf);
    return (window as any).__scales;
  });
  expect(scales.at(-1)!).toBeGreaterThan(scales[0]);
  expect(new Set(scales.map((s) => s.toFixed(3))).size).toBeGreaterThan(6);
  for (let i = 1; i < scales.length; i++) expect(scales[i]).toBeGreaterThanOrEqual(scales[i - 1] - 1e-4);
});

test("a link within the page, clicked mid-glide, still lands where it points", async ({ page }) => {
  await midPage(page);
  await page.mouse.wheel(0, 400);
  await page.locator("[data-site-header] a[data-section='questions']").first().click();
  await expect
    .poll(() => page.evaluate(() => Math.round(document.getElementById("questions")!.getBoundingClientRect().top)), { timeout: 4000 })
    .toBeLessThan(4);
  await page.waitForTimeout(600);
  const top = await page.evaluate(() => Math.round(document.getElementById("questions")!.getBoundingClientRect().top));
  expect(Math.abs(top)).toBeLessThan(4);
});

test("reloaded part-way down after gliding there, it comes back where it was, with no replay", async ({ page }) => {
  await page.goto("/");
  await page.waitForTimeout(1200);
  await page.mouse.move(720, 450);
  for (let i = 0; i < 8; i++) await page.mouse.wheel(0, 100);
  await expect(page.locator("html")).not.toHaveAttribute("data-gliding", { timeout: 3000 });
  await page.waitForTimeout(500);
  const y = Math.round(await page.evaluate(() => scrollY));
  expect(y).toBeGreaterThan(600);
  await page.addInitScript(() => {
    const w = window as any;
    w.__frames = [] as { y: number; intro: number }[];
    const tick = () => {
      const hero = document.querySelector<HTMLElement>("[data-hero]");
      const lid = hero?.querySelector("[data-laptop-body]");
      if (hero && lid)
        w.__frames.push({
          y: Math.round(scrollY),
          intro: hero.getAnimations({ subtree: true }).filter((a) => a.playState === "running" && (a.effect as KeyframeEffect).target === lid).length,
        });
      if (w.__frames.length < 120) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  });
  await page.reload();
  await page.waitForTimeout(1500);
  expect(await page.evaluate(() => document.documentElement.dataset.land)).toBe("restore");
  const frames: { y: number; intro: number }[] = await page.evaluate(() => (window as any).__frames);
  expect(frames.at(-1)!.y).toBe(y);
  expect(Math.max(...frames.map((f) => f.intro))).toBe(0);
});
