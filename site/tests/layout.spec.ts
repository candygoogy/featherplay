import { expect, test, type Page } from "@playwright/test";
import { wake } from "./helpers";

// The owner's three reports on mockup 10, made impossible: the bar's parts on top of each other
// in a smaller window (1), the narrow window's caption buttons stacked (2, tests/window.spec.ts),
// the window running off the right edge (3). The page is loaded fresh at every size of a grid
// from a phone to a 4K screen, odd ones included (half a 2560 screen, a short wide window), and
// checked docked and as the capsule.
const widths = [320, 360, 390, 414, 600, 760, 761, 800, 900, 1024, 1100, 1180, 1280, 1366, 1440, 1536, 1600, 1680, 1920, 2048, 2560, 3200, 3840];
const heights = [600, 720, 900, 1270, 1440, 2160];

async function check(page: Page, w: number, h: number): Promise<string[]> {
  return page.evaluate(
    ({ w, h }) => {
      const out: string[] = [];
      const vw = document.documentElement.clientWidth;
      const at = `${w}x${h} ${document.querySelector<HTMLElement>("[data-site-bar]")!.dataset.state}`;
      // Nothing runs off the side.
      if (document.documentElement.scrollWidth > vw + 1) out.push(`${at}: the page is ${document.documentElement.scrollWidth} px wide`);
      // The window: never wider or taller than the screen, never past its sides; on a wide screen
      // at the top, wholly in view.
      const win = document.querySelector("[data-window]")!.getBoundingClientRect();
      if (win.width > vw + 0.5 || win.height > h + 0.5) out.push(`${at}: the window is larger than the screen`);
      if (win.left < -0.5 || win.right > vw + 0.5) out.push(`${at}: the window runs off a side`);
      if (w >= 761 && scrollY === 0 && (win.top < -0.5 || win.bottom > h + 0.5)) out.push(`${at}: the window runs off the top or bottom`);
      // The bar: its parts apart, whole and on screen (any glide in flight finished first).
      const bar = document.querySelector("[data-site-bar]")!;
      for (const p of bar.querySelectorAll("[data-part]")) for (const a of p.getAnimations()) if (!(a instanceof CSSTransition)) a.finish();
      for (const a of bar.querySelector(".cap-bg")!.getAnimations()) a.finish();
      const parts = [...bar.querySelectorAll<HTMLElement>("[data-part]")]
        .map((el) => ({ el, r: el.getBoundingClientRect(), n: el.getAttribute("aria-label") ?? el.textContent!.trim().slice(0, 12) }))
        .filter((p) => p.r.width > 0);
      for (const p of parts) {
        if (p.r.left < -0.5 || p.r.right > vw + 0.5 || p.r.top < -0.5 || p.r.bottom > 72) out.push(`${at}: ${p.n} off the bar`);
        const lbl = p.el.querySelector<HTMLElement>(".lbl");
        if (lbl && lbl.clientWidth > 1 && lbl.scrollWidth > lbl.clientWidth + 0.5) out.push(`${at}: ${p.n} cut`);
      }
      for (let i = 0; i < parts.length; i++)
        for (let j = i + 1; j < parts.length; j++) {
          const a = parts[i].r;
          const b = parts[j].r;
          if (a.left < b.right - 0.5 && b.left < a.right - 0.5 && a.top < b.bottom - 0.5 && b.top < a.bottom - 0.5) out.push(`${at}: ${parts[i].n} on ${parts[j].n}`);
        }
      return out;
    },
    { w, h },
  );
}

for (const h of heights) {
  test(`loaded at every width from 320 to 3840 and ${h} px tall, the window and the bar fit and the bar's parts are apart`, async ({ page }) => {
    test.setTimeout(180_000);
    const found: string[] = [];
    for (const w of widths) {
      await page.setViewportSize({ width: w, height: h });
      await page.goto("/");
      await wake(page);
      found.push(...(await check(page, w, h)));
      await page.evaluate(() => scrollTo(0, 3200));
      await expect(page.locator("[data-site-bar]")).toHaveAttribute("data-state", "capsule");
      found.push(...(await check(page, w, h)));
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
    [3840, 600],
    [761, 2000],
    [500, 600],
  ]) {
    await page.setViewportSize({ width: w, height: h });
    await page.goto("/");
    await wake(page);
    found.push(...(await check(page, w, h)));
  }
  expect(found).toEqual([]);
});
