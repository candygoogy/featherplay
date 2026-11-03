import { expect, test, type Page } from "@playwright/test";

// A page that loads part-way down (a reload, back or forward, a link to a #section) shows the
// hero as it is for that scroll position from the first frame it's seen: past the hero,
// settled, upright and full size; inside the pinned stretch, that moment. No opening of the
// lid, no rising name, no trip through the stretch from the top. The opening plays only when
// the page loads at the top.

test.use({ viewport: { width: 1440, height: 900 } });

type Frame = { y: number; shown: boolean; stage: string; lid: string; intro: number; settled: string | null };

/** Records every frame of the next page load in this tab. */
async function record(page: Page) {
  await page.addInitScript(() => {
    const frames: Frame[] = [];
    (window as any).__frames = frames;
    const tick = () => {
      const pin = document.querySelector<HTMLElement>("[data-hero-pin]");
      const stage = document.querySelector<HTMLElement>("[data-hero-stage]");
      const lid = document.querySelector<HTMLElement>("[data-hero] [data-laptop-body]");
      const hero = document.querySelector<HTMLElement>("[data-hero]");
      if (pin && stage && lid && hero) {
        frames.push({
          y: Math.round(scrollY),
          shown: getComputedStyle(pin).visibility === "visible",
          stage: getComputedStyle(stage).transform,
          lid: getComputedStyle(lid).transform,
          // The first-load motion: the lid's opening and the name's words rising in.
          intro: hero
            .getAnimations({ subtree: true })
            .filter(
              (a) =>
                a.playState === "running" &&
                ((a.effect as KeyframeEffect).target === lid || /rise/.test((a as CSSAnimation).animationName ?? "")),
            ).length,
          settled: document.querySelector<HTMLElement>("[data-hero] [data-laptop]")!.dataset.settled ?? null,
        });
      }
      if (frames.length < 240) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  });
}
const frames = (page: Page): Promise<Frame[]> => page.evaluate(() => (window as any).__frames);

/** At 4x CPU, so the start-up's frames are as long as on a slow laptop. */
async function slow(page: Page) {
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Emulation.setCPUThrottlingRate", { rate: 4 });
}

async function scrollAndWait(page: Page, vh: number) {
  await page.goto("/");
  await page.waitForTimeout(1200);
  await page.evaluate((vh) => scrollTo({ top: Math.round(innerHeight * vh), behavior: "instant" }), vh);
  await page.waitForTimeout(400);
  return page.evaluate(() => scrollY);
}

/** Every frame that showed the hero showed it as it ends up, with no first-load motion. */
function expectSteady(all: Frame[], y: number) {
  const last = all.at(-1)!;
  expect(last.y).toBe(y);
  expect(last.shown).toBe(true);
  const shown = all.filter((f) => f.shown);
  expect(shown.length).toBeGreaterThan(5);
  for (const f of shown) {
    expect(f.y).toBe(y);
    expect(f.stage).toBe(last.stage);
    expect(f.lid).toBe(last.lid);
  }
  expect(Math.max(...all.map((f) => f.intro))).toBe(0);
}

test("reloaded past the hero, it shows settled from its first frame, with no opening", async ({ page }) => {
  const y = await scrollAndWait(page, 1.6);
  await record(page);
  await slow(page);
  await page.reload();
  await page.waitForTimeout(2000);
  const all = await frames(page);
  expectSteady(all, y);
  const last = all.at(-1)!;
  expect(last.lid).toBe("none");
  expect(last.stage).toMatch(/^matrix\(1, 0, 0, 1, 0, -?[\d.]+\)$/);
  expect(last.settled).toBe("true");
});

test("reloaded inside the pinned stretch, it shows that moment at once", async ({ page }) => {
  const y = await scrollAndWait(page, 0.4);
  await record(page);
  await slow(page);
  await page.reload();
  await page.waitForTimeout(2000);
  const all = await frames(page);
  expectSteady(all, y);
  // Part way: risen a little, still scaled down.
  expect(all.at(-1)!.stage).not.toMatch(/^matrix\(1, 0, 0, 1,/);
});

test("back to the page part-way down, it shows that moment at once", async ({ page }) => {
  const y = await scrollAndWait(page, 0.4);
  await page.goto("/method");
  await page.waitForTimeout(300);
  await record(page);
  await slow(page);
  await page.goBack();
  await page.waitForTimeout(2000);
  expectSteady(await frames(page), y);
});

test("opening a link to a section lands on it at once, not by way of the hero", async ({ page }) => {
  await record(page);
  await page.goto("/#questions");
  await page.waitForTimeout(2000);
  const all = await frames(page);
  const target = await page.evaluate(() => Math.round(document.getElementById("questions")!.getBoundingClientRect().top));
  expect(Math.abs(target)).toBeLessThan(4);
  const y = all.at(-1)!.y;
  expectSteady(all, y);
  expect(all.at(-1)!.settled).toBe("true");
});

test("loaded or reloaded at the top, the lid opens and the name rises in", async ({ page }) => {
  await record(page);
  await page.goto("/");
  await page.waitForTimeout(300);
  expect(Math.max(...(await frames(page)).map((f) => f.intro))).toBeGreaterThan(0);
  await page.waitForTimeout(1200);
  await page.reload();
  await page.waitForTimeout(300);
  const all = await frames(page);
  expect(all.every((f) => f.y === 0)).toBe(true);
  expect(all.every((f) => f.shown)).toBe(true);
  expect(Math.max(...all.map((f) => f.intro))).toBeGreaterThan(0);
});

test("a link within the page still glides there", async ({ page }) => {
  await page.goto("/");
  await page.waitForTimeout(1200);
  const ys = await page.evaluate(async () => {
    document.querySelector<HTMLAnchorElement>('header a[href="/#questions"]')!.click();
    const ys = new Set<number>();
    for (let i = 0; i < 40; i++) {
      await new Promise((r) => requestAnimationFrame(r));
      ys.add(Math.round(scrollY));
    }
    return [...ys];
  });
  expect(ys.length).toBeGreaterThan(5);
});

// Reloading again and again, each within a moment of the last load (seen in a screen
// recording). Whatever the timing, the page ends where it was left, the hero shown as it is
// there, nothing stuck hidden, no opening replayed, and no frame where the part of the page
// on screen is still skipped (content-visibility) and shows as an empty ground.
type Land = { y: number; skipped: number; shown: boolean; intro: number; land: string | null };

async function recordLanding(page: Page) {
  await page.addInitScript(() => {
    const all: Land[] = [];
    (window as any).__land = all;
    const tick = () => {
      const pin = document.querySelector<HTMLElement>("[data-hero-pin]");
      const lid = document.querySelector<HTMLElement>("[data-hero] [data-laptop-body]");
      if (pin && lid && document.querySelector("body > footer")) {
        // Blocks crossing the viewport whose contents are skipped.
        const skipped = [...document.querySelectorAll<HTMLElement>("main > section, body > footer")].filter((b) => {
          const r = b.getBoundingClientRect();
          const inner = b.firstElementChild;
          return r.bottom > 0 && r.top < innerHeight && inner && !inner.checkVisibility({ contentVisibilityAuto: true });
        }).length;
        all.push({
          y: Math.round(scrollY),
          skipped,
          shown: getComputedStyle(pin).visibility === "visible",
          intro: lid.getAnimations().filter((a) => a.playState === "running").length,
          land: document.documentElement.dataset.land ?? null,
        });
      }
      if (all.length < 150) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  });
}

for (const [where, vh] of [
  ["at the bottom", 99],
  ["past the hero", 1.6],
  ["inside the pinned stretch", 0.4],
] as const) {
  test(`reloaded quickly over and over ${where}, it ends where it was left, shown and still`, async ({ page }) => {
    await page.goto("/");
    await page.waitForTimeout(1200);
    await page.evaluate((vh) => scrollTo({ top: Math.round(innerHeight * vh), behavior: "instant" }), vh);
    await page.waitForTimeout(500);
    const y = await page.evaluate(() => Math.round(scrollY));
    await recordLanding(page);
    await slow(page);
    for (const gap of [40, 120, 250, 90]) {
      await page.reload({ waitUntil: "commit" });
      await page.waitForTimeout(gap);
    }
    await page.reload({ waitUntil: "commit" });
    await page.waitForTimeout(3000);
    const all: Land[] = await page.evaluate(() => (window as any).__land);
    const last = all.at(-1)!;
    expect(Math.abs(last.y - y)).toBeLessThan(4);
    expect(last.shown).toBe(true);
    expect(last.land).toBe("restore");
    expect(Math.max(...all.map((f) => f.intro))).toBe(0);
    expect(all.filter((f) => f.skipped > 0)).toEqual([]);
  });
}

test("a style read before the stylesheets arrive doesn't make the page's colours ease in", async ({ page }) => {
  // What an extension's content script can do on a reload, while the CSS is still on its way.
  await page.route("**/*.css", async (route) => {
    await new Promise((r) => setTimeout(r, 300));
    await route.continue();
  });
  await page.addInitScript(() => {
    const easing: string[] = [];
    (window as any).__easing = easing;
    document.addEventListener("readystatechange", () => {
      for (const a of document.querySelectorAll("a")) getComputedStyle(a).color;
      const band = document.querySelector(".site-footer .band");
      if (band) getComputedStyle(band).backgroundColor;
    });
    let n = 0;
    const tick = () => {
      for (const a of document.getAnimations())
        if (a instanceof CSSTransition && a.playState === "running" && /color/.test(a.transitionProperty)) easing.push(a.transitionProperty + " " + ((a.effect as KeyframeEffect).target as Element).className);
      if (++n < 90) requestAnimationFrame(tick);
    };
    requestAnimationFrame(tick);
  });
  await page.goto("/");
  await page.waitForTimeout(1600);
  expect([...new Set(await page.evaluate(() => (window as any).__easing))]).toEqual([]);
});
