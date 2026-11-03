import { expect, test } from "@playwright/test";
import { skipTour, wake } from "./helpers";

// The privacy rule: no trackers, no cookies, nothing fetched from anywhere else.
for (const path of ["/", "/method", "/no-such-page"]) {
  test(`${path} makes no requests to other origins and sets no cookies`, async ({ page, context, baseURL }) => {
    const origin = new URL(baseURL!).origin;
    const foreign: string[] = [];
    page.on("request", (r) => {
      const u = new URL(r.url());
      if (u.protocol.startsWith("http") && u.origin !== origin) foreign.push(r.url());
    });
    await page.goto(path);
    await page.waitForLoadState("networkidle");
    if (path === "/") {
      // Play with the demo a little: still nothing leaves the page.
      await wake(page);
      await skipTour(page);
      await page.locator("[data-hero] .bar .fp-play").click();
      await page.evaluate(() => scrollTo({ top: document.body.scrollHeight, behavior: "instant" }));
      await page.waitForLoadState("networkidle");
    }
    expect(foreign).toEqual([]);
    expect(await context.cookies()).toEqual([]);
    expect(await page.evaluate(() => document.cookie)).toBe("");
  });
}
