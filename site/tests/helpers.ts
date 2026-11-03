import { expect, type Page } from "@playwright/test";

/** Scrolls through the hero's scroll moment until the laptop has settled for the tour. */
export async function wake(page: Page) {
  await page.evaluate(() => {
    const hero = document.querySelector<HTMLElement>("[data-hero]")!;
    scrollTo({ top: hero.offsetTop + innerHeight * 1.05, behavior: "instant" });
  });
  await expect(page.locator("[data-laptop]").first()).toHaveAttribute("data-settled", "true");
}

/** The lid's lean in degrees, read off its computed transform (0 when upright). */
export async function lidAngle(page: Page) {
  return page.locator("[data-laptop-body]").first().evaluate((el) => {
    const t = getComputedStyle(el).transform;
    if (!t || t === "none") return 0;
    const m = new DOMMatrixReadOnly(t);
    // rotateX(a): m22 = cos a, m23 = sin a
    return Math.round((Math.atan2(m.m23, m.m22) * 180) / Math.PI * 10) / 10;
  });
}

/** Closes the guided tour, for tests about the replica itself. */
export async function skipTour(page: Page) {
  const skip = page.locator("[data-tour-skip]");
  if (await skip.isVisible()) await skip.click();
}
