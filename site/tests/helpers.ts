import { expect, type Page } from "@playwright/test";

/** Waits until the first screen's window is sized and placed (src/demo/window.ts). */
export async function wake(page: Page) {
  await expect(page.locator("[data-window]")).toHaveClass(/\bplaced\b/);
}

/** The window's visible box on screen, in CSS px. */
export async function windowBox(page: Page) {
  return (await page.locator("[data-window]").boundingBox())!;
}
