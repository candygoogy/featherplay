import { expect, type Page } from "@playwright/test";

/** Waits until the first screen's window is sized and placed (src/demo/window.ts). */
export async function wake(page: Page) {
  await expect(page.locator("[data-window]")).toHaveClass(/\bplaced\b/);
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

/** The window's visible box on screen, in CSS px. */
export async function windowBox(page: Page) {
  return (await page.locator("[data-window]").boundingBox())!;
}
