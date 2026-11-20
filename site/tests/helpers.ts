import { expect, type Page } from "@playwright/test";

/** Waits until the first screen's window is sized and placed (src/demo/window.ts). */
export async function wake(page: Page) {
  await expect(page.locator("[data-window]")).toHaveClass(/\bplaced\b/);
  // Landed: no opening from the tray (the first screen's arrival) still under way.
  await expect.poll(() => page.locator("[data-window]").evaluate((w) => w.getAnimations().length), { timeout: 3000 }).toBe(0);
}

/** Scrolls something below the pinned story into view: its section first (a section that
 *  waits to render, content-visibility, gives its insides no true place until it has), then,
 *  once the sections on the way have rendered and settled, the thing itself. */
export async function reach(page: Page, selector: string) {
  const el = page.locator(selector).first();
  await el.evaluate((e) => e.closest("main > section")?.scrollIntoView());
  await page.waitForTimeout(150);
  await el.scrollIntoViewIfNeeded();
  await page.waitForTimeout(250);
  await el.scrollIntoViewIfNeeded();
  await page.waitForTimeout(100);
}

/** Waits until no section is left waiting to be laid out (just after a fresh load they are
 *  released one at a time, src/scripts/release.ts). */
export async function laidOut(page: Page) {
  await expect
    .poll(() => page.evaluate(() => [...document.querySelectorAll("main > section, body > footer")].filter((s) => getComputedStyle(s).contentVisibility === "auto").length), { timeout: 4000 })
    .toBe(0);
}

/** A spot in the window's title bar row that drags it: 96 of the app's px in from its left
 *  (clear of Back on Now playing, on the name on the browse pages), 24 down (the row's middle),
 *  as a small box. */
export async function grip(page: Page) {
  return page.locator("[data-window] [data-replica]").evaluate((e: HTMLElement) => {
    const b = e.getBoundingClientRect();
    const k = b.width / e.offsetWidth;
    return { x: b.left + 96 * k, y: b.top + 18 * k, width: 10 * k, height: 12 * k };
  });
}

/** The window's visible box on screen, in CSS px. */
export async function windowBox(page: Page) {
  return (await page.locator("[data-window]").boundingBox())!;
}
