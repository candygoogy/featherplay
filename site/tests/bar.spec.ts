import { expect, test, type Page } from "@playwright/test";
import { wake } from "./helpers";

// The bar: docked at the top, one floating capsule once the page moves; two states and nothing
// in between, its parts never on top of each other at any width (the owner's report on
// mockup 10: the docked bar and the capsule half-drawn at once, "Questions" cut, parts stacked).
test.use({ viewport: { width: 1440, height: 900 } });

const bar = (page: Page) => page.locator("[data-site-bar]");
/** At rest: two frames for any observer to answer, then no glide running. */
const settled = (page: Page) =>
  page.evaluate(
    () =>
      new Promise<boolean>((r) =>
        requestAnimationFrame(() =>
          requestAnimationFrame(() =>
            r([...document.querySelectorAll("[data-site-bar] [data-part]")].every((p) => p.getAnimations().every((a) => a instanceof CSSTransition || a.playState === "finished"))),
          ),
        ),
      ),
  );

/** The visible parts' boxes at rest (a glide in flight is finished first: the layout is what
 *  is judged), and what is wrong with them: overlaps, cut words, off screen. */
const problems = (page: Page) =>
  page.evaluate(() => {
    for (const p of document.querySelectorAll("[data-site-bar] [data-part]")) for (const a of p.getAnimations()) if (!(a instanceof CSSTransition)) a.finish();
    for (const a of document.querySelector("[data-site-bar] .cap-bg")!.getAnimations()) a.finish();
    const vw = document.documentElement.clientWidth;
    const out: string[] = [];
    const parts = [...document.querySelectorAll<HTMLElement>("[data-site-bar] [data-part]")]
      .map((el) => ({ el, r: el.getBoundingClientRect(), name: el.getAttribute("aria-label") ?? el.textContent!.trim().slice(0, 12) }))
      .filter((p) => p.r.width > 0 && getComputedStyle(p.el).visibility !== "hidden");
    for (const p of parts) {
      if (p.r.left < -0.5 || p.r.right > vw + 0.5) out.push(`${p.name} off screen`);
      const lbl = p.el.querySelector<HTMLElement>(".lbl");
      if (lbl && lbl.clientWidth > 1 && lbl.scrollWidth > lbl.clientWidth + 0.5) out.push(`${p.name} cut`);
    }
    for (let i = 0; i < parts.length; i++)
      for (let j = i + 1; j < parts.length; j++) {
        const a = parts[i].r;
        const b = parts[j].r;
        if (a.left < b.right - 0.5 && b.left < a.right - 0.5 && a.top < b.bottom - 0.5 && b.top < a.bottom - 0.5) out.push(`${parts[i].name} on ${parts[j].name}`);
      }
    const state = document.querySelector<HTMLElement>("[data-site-bar]")!.dataset.state;
    if (state === "capsule") {
      const cap = document.querySelector("[data-site-bar] .cap-bg")!.getBoundingClientRect();
      for (const p of parts) if (p.r.left < cap.left - 0.5 || p.r.right > cap.right + 0.5) out.push(`${p.name} outside the capsule`);
    }
    return out;
  });

test("docked at the top, a capsule once the page moves, docked again back at the top", async ({ page }) => {
  await page.goto("/");
  await expect(bar(page)).toHaveAttribute("data-state", "docked");
  await page.mouse.wheel(0, 400);
  await expect(bar(page)).toHaveAttribute("data-state", "capsule");
  await expect.poll(() => settled(page)).toBe(true);
  await page.evaluate(() => scrollTo(0, 0));
  await expect(bar(page)).toHaveAttribute("data-state", "docked");
});

for (const height of [600, 1270]) {
  test(`at every width from 320 to 3840 (height ${height}), docked and as the capsule, no part overlaps another, is cut or leaves the screen`, async ({ page }) => {
    test.setTimeout(120_000);
    await page.setViewportSize({ width: 1440, height });
    await page.goto("/");
    for (const state of ["docked", "capsule"]) {
      if (state === "capsule") await page.evaluate(() => scrollTo(0, 800));
      await expect(bar(page)).toHaveAttribute("data-state", state);
      const found: string[] = [];
      for (let w = 320; w <= 3840; w += w < 1400 ? 10 : 40) {
        await page.setViewportSize({ width: w, height });
        await expect.poll(() => settled(page)).toBe(true);
        const p = await problems(page);
        if (p.length) found.push(`${w}: ${p.join(", ")}`);
      }
      expect(found, state).toEqual([]);
    }
  });
}

test("the change can't be caught half-way: it ends on its own, a resize ends it at once, and a reload part-way down starts as the capsule", async ({ page }) => {
  await page.goto("/");
  await page.mouse.wheel(0, 400);
  await expect(bar(page)).toHaveAttribute("data-state", "capsule");
  // Mid-glide, a resize: every part is at its place at once.
  await page.setViewportSize({ width: 1000, height: 900 });
  await expect.poll(() => settled(page)).toBe(true);
  expect(await problems(page)).toEqual([]);
  await page.evaluate(() => scrollTo(0, 1500));
  await page.reload();
  await expect(bar(page)).toHaveAttribute("data-state", "capsule");
  expect(await settled(page)).toBe(true);
  expect(await problems(page)).toEqual([]);
});

test.describe("reduced motion", () => {
  test.use({ reducedMotion: "reduce" });
  test("the bar changes state at once, nothing glides", async ({ page }) => {
    await page.goto("/");
    await page.evaluate(() => scrollTo(0, 600));
    await expect(bar(page)).toHaveAttribute("data-state", "capsule");
    expect(await page.evaluate(() => [...document.querySelectorAll("[data-site-bar] [data-part]")].flatMap((p) => p.getAnimations()).filter((a) => !(a instanceof CSSTransition)).length)).toBe(0);
    expect(await problems(page)).toEqual([]);
  });
});

test("the tray icon hides and shows the window; the close dialog's remembered choice doesn't ask again", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  const tray = page.getByRole("button", { name: /^featherplay in the tray/ });
  const win = page.locator("[data-window]");
  await tray.click();
  await expect(win).toHaveAttribute("data-minimised", "");
  await tray.click();
  await expect(win).not.toHaveAttribute("data-minimised", /.*/);
  await expect(win).toBeVisible();
  const close = win.getByRole("button", { name: /^Close featherplay/ });
  await close.click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Remember my choice").check();
  await dialog.getByRole("button", { name: "Keep playing in the tray" }).click();
  await expect(win).toHaveAttribute("data-minimised", "");
  await tray.click();
  await expect(win).toBeVisible();
  await close.click();
  await expect(dialog).toBeHidden();
  await expect(win).toHaveAttribute("data-minimised", "");
});

test("the capsule's song opens its flyout: the media keys and the six songs; Escape closes it and the focus comes back", async ({ page }) => {
  await page.route("**/music/**", (r) => r.abort());
  await page.goto("/");
  await page.evaluate(() => scrollTo(0, 800));
  await expect(bar(page)).toHaveAttribute("data-state", "capsule");
  const np = bar(page).getByRole("button", { name: /the song and the player$/ });
  await np.click();
  const fly = page.locator("#fly-song");
  await expect(fly).toBeVisible();
  await expect(np).toHaveAttribute("aria-expanded", "true");
  // Inside the viewport, under the capsule.
  const f = (await fly.boundingBox())!;
  expect(f.x).toBeGreaterThanOrEqual(0);
  expect(f.x + f.width).toBeLessThanOrEqual(1440);
  expect(f.y).toBeGreaterThanOrEqual(60);
  await fly.getByRole("button", { name: /Origami/ }).click();
  await expect(page.locator("[data-window] [data-bar-title]")).toHaveText("Origami");
  await expect(page.locator("html")).toHaveAttribute("data-playing", "true");
  await fly.getByRole("button", { name: "Pause" }).focus();
  await page.keyboard.press("Escape");
  await expect(fly).toBeHidden();
  await expect(np).toBeFocused();
});

test("Download's flyout opens for the keyboard and Tab goes on into it", async ({ page }) => {
  await page.goto("/");
  const dl = bar(page).getByRole("link", { name: "Download" });
  await dl.focus();
  await expect(page.locator("#fly-dl")).toBeVisible();
  await page.keyboard.press("Tab");
  await expect(page.locator("#fly-dl a").first()).toBeFocused();
  await expect(page.locator("#fly-dl a").first()).toHaveAttribute("href", /releases\/download\/v0\.4\.1\/featherplay-setup-0\.4\.1\.exe$/);
});

test("a right-click on the empty desk opens the app's menu: the arrow keys move in it, Escape closes it", async ({ page }) => {
  await page.route("**/music/**", (r) => r.abort());
  await page.goto("/");
  await page.mouse.click(300, 800, { button: "right" });
  const menu = page.getByRole("menu", { name: "Desktop" });
  await expect(menu).toBeVisible();
  await expect(menu.getByRole("menuitem").first()).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(menu.getByRole("menuitem", { name: "Next song" })).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(menu).toBeHidden();
  await expect(page.locator("[data-window] [data-bar-title]")).toHaveText("Invisible Light");
  await page.mouse.click(300, 800, { button: "right" });
  await expect(menu).toBeVisible();
  await page.keyboard.press("Escape");
  await expect(menu).toBeHidden();
});

test("the sections' icons say their names in Windows' tooltip after a rest; Escape puts it away", async ({ page }) => {
  await page.setViewportSize({ width: 1000, height: 800 });
  await page.goto("/");
  const lyrics = bar(page).getByRole("link", { name: "Lyrics" });
  await lyrics.hover();
  const tip = page.locator("[data-bar-tip]");
  await expect(tip).toHaveAttribute("data-on", "");
  await expect(tip).toHaveText("Lyrics, up close");
  await page.keyboard.press("Escape");
  await expect(tip).not.toHaveAttribute("data-on", /.*/);
});

test("the keyboard brings the theme toggle into the capsule", async ({ page }) => {
  await page.goto("/");
  await page.evaluate(() => scrollTo(0, 800));
  await expect(bar(page)).toHaveAttribute("data-state", "capsule");
  await expect(bar(page).locator("[data-theme-toggle]")).toBeHidden();
  await page.keyboard.press("Tab"); // the skip link
  await page.keyboard.press("Tab"); // the bar's mark
  await expect(bar(page)).toHaveAttribute("data-kbd", "");
  await expect(bar(page).locator("[data-theme-toggle]")).toBeVisible();
  await expect.poll(() => settled(page)).toBe(true);
  expect(await problems(page)).toEqual([]);
});
