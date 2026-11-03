import { expect, test } from "@playwright/test";
import { skipTour, wake } from "./helpers";

test.describe("clock", () => {
  test.use({ locale: "en-GB", timezoneId: "UTC" });

  test("the clock shows the visitor's time (HH:MM) and moves on the minute", async ({ page }) => {
    await page.clock.install({ time: new Date("2026-10-31T21:07:30Z") });
    await page.goto("/");
    const clock = page.locator("[data-clock]").first();
    await expect(clock).toContainText("21:07");
    await page.clock.runFor(31_000);
    await expect(clock).toContainText("21:08");
  });
});

test("hovering the tray icon shows the two-line tooltip", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  await skipTour(page);
  const icon = page.locator("[data-tray-icon]").first();
  await icon.scrollIntoViewIfNeeded();
  await icon.hover();
  const tip = page.getByRole("tooltip").first();
  await expect(tip).toBeVisible();
  // The app's hover text: the title, then the artist and where the song is.
  const lines = (await tip.innerText()).split("\n").map((l) => l.trim()).filter(Boolean);
  expect(lines).toHaveLength(2);
  expect(lines[0]).toBe("California Lullabye");
  expect(lines[1]).toMatch(/^Josh Woodward, paused at 0:20$/);
});

test("focusing the tray icon shows the tooltip too", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  await skipTour(page);
  await page.locator("[data-tray-icon]").first().focus();
  await expect(page.getByRole("tooltip").first()).toBeVisible();
});

for (const width of [320, 390, 1440, 3840]) {
  test(`no horizontal overflow at ${width} px wide`, async ({ page }) => {
    await page.setViewportSize({ width, height: width < 800 ? 800 : Math.round(width * 0.6) });
    await page.goto("/");
    const { scroll, client } = await page.evaluate(() => ({
      scroll: document.documentElement.scrollWidth,
      client: document.documentElement.clientWidth,
    }));
    expect(scroll).toBeLessThanOrEqual(client);
    // The laptop fits inside the viewport too.
    const box = await page.locator("[data-laptop]").first().boundingBox();
    expect(box).not.toBeNull();
    expect(box!.x).toBeGreaterThanOrEqual(0);
    expect(box!.x + box!.width).toBeLessThanOrEqual(width);
  });
}

test("the screen is laid out at 1440x900 and scaled to fit", async ({ page }) => {
  await page.goto("/");
  // Settled, upright: at the top the lid leans back, which foreshortens the screen.
  await wake(page);
  const desk = page.locator("[data-desktop]").first();
  const size = await desk.evaluate((el) => ({ w: el.offsetWidth, h: el.offsetHeight }));
  expect(size).toEqual({ w: 1440, h: 900 });
  const screen = await page.locator("[data-screen]").first().boundingBox();
  const shown = await desk.boundingBox();
  expect(Math.abs(shown!.width - screen!.width)).toBeLessThan(1.5);
  expect(Math.abs(shown!.height - screen!.height)).toBeLessThan(1.5);
});

test("the laptop body colour differs between themes", async ({ page }) => {
  await page.goto("/");
  const body = page.locator("[data-laptop-body]").first();
  const dark = await body.evaluate((el) => getComputedStyle(el).backgroundColor);
  await page.getByRole("button", { name: "Theme: System. Switch to Light" }).click();
  await expect.poll(() => body.evaluate((el) => getComputedStyle(el).backgroundColor)).not.toBe(dark);
});

// The desktop behind the replica has our own wallpaper (light and shade over the song's
// colour), dark or light with the page (each on its own layer, so they cross-fade), loaded
// once the page is idle.
for (const theme of ["dark", "light"] as const) {
  test(`the desktop's wallpaper: its ${theme} folds over the song's colour, behind the window`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: theme });
    await page.goto("/");
    const desk = page.locator("[data-hero] [data-desktop]");
    await expect(desk).toHaveAttribute("data-wall", "", { timeout: 5000 });
    // The two themes' folds lie on two layers that cross-fade; the shown one is opaque.
    await expect
      .poll(() => desk.locator(".wallpaper").evaluate((el) => Math.max(...["::before", "::after"].map((p) => Number(getComputedStyle(el, p).opacity)))))
      .toBe(1);
    const wall = await desk.locator(".wallpaper").evaluate((el) => {
      const shown = ["::before", "::after"].map((p) => getComputedStyle(el, p)).find((s) => s.opacity === "1")!;
      return { image: shown.backgroundImage, blend: shown.mixBlendMode, ground: getComputedStyle(el).backgroundColor };
    });
    expect(wall.image).toContain(`wallpaper-${theme}`);
    expect(wall.blend).toBe("overlay");
    expect(wall.ground).not.toBe("rgba(0, 0, 0, 0)");
    // Behind the windows: the window layer comes after it.
    expect(
      await desk.evaluate((d) => {
        const w = d.querySelector(".wallpaper")!;
        const win = d.querySelector(".windows")!;
        return !!(w.compareDocumentPosition(win) & Node.DOCUMENT_POSITION_FOLLOWING);
      }),
    ).toBe(true);
  });
}

// The wallpaper takes the playing song's colour on every desktop (the hero's and the
// features'), easing with the page's glow: it reads the root's --glow, which no section
// shadows. California Lullabye starts (peach: red over blue); Watching Seabirds Soar is teal.
for (const theme of ["dark", "light"] as const) {
  test(`changing the song changes every wallpaper's tint (${theme})`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: theme });
    await page.goto("/");
    const tints = () =>
      page.evaluate(() =>
        [...document.querySelectorAll(".wallpaper")].map((w) => {
          const [r, , b] = getComputedStyle(w).backgroundColor.match(/[\d.]+/g)!.map(Number);
          return r > b ? "warm" : "cool";
        }),
      );
    expect(await tints()).toEqual(["warm", "warm"]);
    // Nothing eases while the page boots (base.css), which can outlast the load event by a
    // frame or two: read the wallpaper's easing once it's done.
    await expect(page.locator("html")).not.toHaveClass(/\bbooting\b/);
    expect(await page.locator(".wallpaper").first().evaluate((w) => getComputedStyle(w).transitionDuration)).toBe(
      "0.9s",
    );
    await page.evaluate(() =>
      document.querySelector<HTMLElement>('[data-hero] [data-play="watching-seabirds-soar"]')!.click(),
    );
    await expect.poll(tints, { timeout: 3000 }).toEqual(["cool", "cool"]);
  });
}
