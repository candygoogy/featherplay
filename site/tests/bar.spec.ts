import { expect, test, type Page } from "@playwright/test";
import { wake } from "./helpers";
import { problems, settled, waitSettled, WIDTHS } from "./check";

// The bar: docked at the top, one floating capsule once the page moves; two states and nothing
// in between, its parts never on top of each other at any width (the owner's report on
// mockup 10: the docked bar and the capsule half-drawn at once, "Questions" cut, parts stacked).
test.use({ viewport: { width: 1440, height: 900 } });

const bar = (page: Page) => page.locator("[data-site-bar]");

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
      for (const w of WIDTHS) {
        await page.setViewportSize({ width: w, height });
        await waitSettled(page);
        found.push(...(await problems(page)));
      }
      expect(found, state).toEqual([]);
    }
  });
}

test("the change can't be caught half-way: a resize in the middle of the glide ends it at once, and a reload part-way down starts as the capsule", async ({ page }) => {
  await page.goto("/");
  await page.waitForTimeout(600);
  // The glide is under way (the page moved, the parts are still travelling)...
  const gliding = await page.evaluate(
    () =>
      new Promise<boolean>((r) => {
        document.querySelector("[data-site-bar]")!.addEventListener("barstate", () => r([...document.querySelectorAll("[data-site-bar] [data-part]")].some((p) => p.getAnimations().some((a) => !(a instanceof CSSTransition) && a.playState === "running"))), { once: true });
        scrollTo(0, 400);
      }),
  );
  expect(gliding).toBe(true);
  // ...and a resize lands every part where its CSS puts it, within a frame, not 300 ms later.
  await page.setViewportSize({ width: 1000, height: 900 });
  expect(await settled(page)).toBe(true);
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
  // Landed (on its way back from the tray its buttons pass under the bar, where a press on
  // one would have the test scroll the page to reach it).
  await wake(page);
  const close = win.getByRole("button", { name: /^Close featherplay/ });
  await close.click();
  const dialog = page.getByRole("dialog");
  await dialog.getByLabel("Remember my choice").check();
  await dialog.getByRole("button", { name: "Keep playing in the tray" }).click();
  await expect(win).toHaveAttribute("data-minimised", "");
  await tray.click();
  await expect(win).toBeVisible();
  await wake(page);
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

test("an open flyout follows a resize: opened at 1280, the screen turned to 375, it is still on it", async ({ page }) => {
  await page.route("**/music/**", (r) => r.abort());
  // No story either side (reduced motion): the bar stays the capsule, only the size changes.
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.setViewportSize({ width: 1280, height: 800 });
  await page.goto("/");
  await page.evaluate(() => scrollTo(0, 3200));
  await expect(bar(page)).toHaveAttribute("data-state", "capsule");
  await bar(page).getByRole("button", { name: /the song and the player$/ }).click();
  const fly = page.locator("#fly-song");
  await expect(fly).toBeVisible();
  // Placed under the capsule's song, right of the middle.
  await expect.poll(async () => (await fly.boundingBox())!.x).toBeGreaterThan(400);
  await page.setViewportSize({ width: 375, height: 800 });
  await expect
    .poll(async () => {
      const f = (await fly.boundingBox())!;
      return f.x >= 8 - 0.5 && f.x + f.width <= 375 - 8 + 0.5;
    })
    .toBe(true);
});

test("opened from the capsule and back at the top, the song's flyout goes under the tray icon, not to the corner", async ({ page }) => {
  await page.route("**/music/**", (r) => r.abort());
  await page.goto("/");
  await page.evaluate(() => scrollTo(0, 3200));
  await expect(bar(page)).toHaveAttribute("data-state", "capsule");
  await bar(page).getByRole("button", { name: /the song and the player$/ }).click();
  const fly = page.locator("#fly-song");
  await expect(fly).toBeVisible();
  await page.evaluate(() => scrollTo(0, 0));
  await expect(bar(page)).toHaveAttribute("data-state", "docked");
  await waitSettled(page);
  const tray = (await bar(page).locator("[data-tray]").boundingBox())!;
  await expect
    .poll(async () => {
      const f = (await fly.boundingBox())!;
      return Math.abs(f.x + f.width / 2 - (tray.x + tray.width / 2)) < 2 || f.x + f.width >= 1440 - 8 - 16;
    })
    .toBe(true);
  expect((await fly.boundingBox())!.x).toBeGreaterThan(100);
});

test("the tray icon opens no flyout of its own to the keyboard: it never says it is expanded", async ({ page }) => {
  await page.route("**/music/**", (r) => r.abort());
  await page.goto("/");
  const tray = bar(page).locator("[data-tray]");
  await tray.hover();
  await expect(page.locator("#fly-song")).toBeVisible();
  expect(await tray.getAttribute("aria-expanded")).toBeNull();
});

test("on a narrow phone the song's flyout stays inside the screen, its last row above the bottom", async ({ page }) => {
  await page.route("**/music/**", (r) => r.abort());
  await page.setViewportSize({ width: 360, height: 640 });
  await page.goto("/");
  await bar(page).getByRole("button", { name: /the song and the player$/ }).click();
  const fly = page.locator("#fly-song");
  await expect(fly).toBeVisible();
  const f = (await fly.boundingBox())!;
  expect(f.x).toBeGreaterThanOrEqual(7.5);
  expect(f.x + f.width).toBeLessThanOrEqual(360 - 7.5);
  expect(f.y + f.height).toBeLessThanOrEqual(640);
});

test("a press on Download opens its files (its chevron turned), a second press puts them away", async ({ page }) => {
  await page.goto("/");
  const dl = bar(page).getByRole("link", { name: "Download" });
  const fly = page.locator("#fly-dl");
  await dl.click();
  await expect(fly).toBeVisible();
  await expect(dl).toHaveAttribute("aria-expanded", "true");
  // Held open: the pointer leaving doesn't close it.
  await page.mouse.move(10, 500);
  await page.waitForTimeout(400);
  await expect(fly).toBeVisible();
  await dl.click();
  await expect(fly).toBeHidden();
  await expect(dl).toHaveAttribute("aria-expanded", "false");
  expect(new URL(page.url()).hash).toBe("");
});

test("Download's flyout opens for the keyboard and Tab goes on into it", async ({ page }) => {
  await page.goto("/");
  const dl = bar(page).getByRole("link", { name: "Download" });
  await dl.focus();
  await expect(page.locator("#fly-dl")).toBeVisible();
  await page.keyboard.press("Tab");
  await expect(page.locator("#fly-dl a").first()).toBeFocused();
  await expect(page.locator("#fly-dl a").first()).toHaveAttribute("href", /releases\/download\/v0\.8\.0\/featherplay-setup-0\.8\.0\.exe$/);
});

/** A point on the first screen's empty desk (no word, picture, control or window under it). */
async function emptyDesk(page: Page) {
  await expect(page.locator("[data-first]")).toHaveAttribute("data-shown", "");
  return page.evaluate(() => {
    const NOT = "a, button, input, p, h1, h2, img, svg, [data-window], [data-site-bar], [popover], [data-shelf], .hand, [role]";
    for (let y = innerHeight - 30; y > 80; y -= 20)
      for (let x = 30; x < innerWidth - 30; x += 20) {
        const e = document.elementFromPoint(x, y);
        if (e?.closest("[data-stage]") && !e.closest(NOT)) return { x, y };
      }
    throw new Error("no empty desk");
  });
}

test("the menu works with the mouse: a click on one of its items does it", async ({ page }) => {
  await page.goto("/");
  const spot = await emptyDesk(page);
  await page.mouse.click(spot.x, spot.y, { button: "right" });
  const menu = page.getByRole("menu", { name: "Desktop" });
  await expect(menu).toBeVisible();
  const item = menu.getByRole("menuitem", { name: "Switch to light" });
  const b = (await item.boundingBox())!;
  // A real pointer click (not the locator's, which would find its way to the button anyway).
  await page.mouse.click(b.x + b.width / 2, b.y + b.height / 2);
  await expect(menu).toBeHidden();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
});

test("the menu's focused item shows the focus ring, in both themes", async ({ page }) => {
  for (const scheme of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.goto("/");
    await page.keyboard.press("Shift+F10");
    const menu = page.getByRole("menu", { name: "Desktop" });
    await expect(menu.getByRole("menuitem").first()).toBeFocused();
    const ring = await menu.getByRole("menuitem").first().evaluate((el) => {
      const cs = getComputedStyle(el);
      return { style: cs.outlineStyle, width: parseFloat(cs.outlineWidth), colour: cs.outlineColor, ground: getComputedStyle(el.closest("[role=menu]")!).backgroundColor };
    });
    expect(ring.style, scheme).toBe("solid");
    expect(ring.width, scheme).toBeGreaterThanOrEqual(2);
    expect(ring.colour, scheme).not.toBe(ring.ground);
    await page.keyboard.press("Escape");
  }
});

test("the browser's own menu stays everywhere but the first screen's empty desk", async ({ page }) => {
  await page.goto("/");
  await page.evaluate(() => document.getElementById("questions")!.scrollIntoView());
  await page.waitForTimeout(300);
  // Beside the questions, on the plain ground of a section.
  const prevented = await page.evaluate(() => {
    const sec = document.getElementById("questions")!;
    const e = new MouseEvent("contextmenu", { bubbles: true, cancelable: true, clientX: 5, clientY: 300 });
    sec.dispatchEvent(e);
    return e.defaultPrevented;
  });
  expect(prevented).toBe(false);
  await expect(page.getByRole("menu", { name: "Desktop" })).toBeHidden();
});

test("a right-click on the empty desk opens the app's menu: the arrow keys move in it, Escape closes it", async ({ page }) => {
  await page.route("**/music/**", (r) => r.abort());
  await page.goto("/");
  const spot = await emptyDesk(page);
  await page.mouse.click(spot.x, spot.y, { button: "right" });
  const menu = page.getByRole("menu", { name: "Desktop" });
  await expect(menu).toBeVisible();
  await expect(menu.getByRole("menuitem").first()).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(menu.getByRole("menuitem", { name: "Next song" })).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(menu).toBeHidden();
  await expect(page.locator("[data-window] [data-bar-title]")).toHaveText("Invisible Light");
  await page.mouse.click(spot.x, spot.y, { button: "right" });
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

test("the theme toggle changes the theme under an open flyout, never closing it", async ({ page }) => {
  await page.goto("/");
  await page.evaluate(() => scrollTo(0, 800));
  await expect(bar(page)).toHaveAttribute("data-state", "capsule");
  const fly = page.locator("#fly-song");
  await bar(page).locator("[data-np]").click();
  await expect(fly).toBeVisible();
  await waitSettled(page);
  // Any closing on the way would show here (beforetoggle fires at once, a toggle is coalesced).
  await fly.evaluate((el) => el.addEventListener("beforetoggle", (e) => (e as ToggleEvent).newState === "closed" && ((window as unknown as { shut: number }).shut = 1 + ((window as unknown as { shut?: number }).shut ?? 0))));
  const toggle = bar(page).locator("[data-theme-toggle]");
  const before = await toggle.getAttribute("data-choice");
  await toggle.click();
  await expect(toggle).not.toHaveAttribute("data-choice", before ?? "");
  await expect(fly).toBeVisible();
  const shut = await page.evaluate(() => (window as unknown as { shut?: number }).shut ?? 0);
  // Closed by the press and shown again in the same task at most: never left closed.
  expect(shut).toBeLessThanOrEqual(1);
  expect(await fly.evaluate((el) => el.matches(":popover-open"))).toBe(true);
});

test("the theme toggle stays in the capsule", async ({ page }) => {
  await page.goto("/");
  await expect(bar(page).locator("[data-theme-toggle]")).toBeVisible();
  await page.evaluate(() => scrollTo(0, 800));
  await expect(bar(page)).toHaveAttribute("data-state", "capsule");
  await expect(bar(page).locator("[data-theme-toggle]")).toBeVisible();
  await waitSettled(page);
  expect(await problems(page)).toEqual([]);
});

test.describe("on a phone", () => {
  test.use({ viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true });
  test("the sections' part wears the section on screen and opens the five; a pick goes there", async ({ page }) => {
    await page.goto("/");
    const part = bar(page).locator("[data-secs]");
    await expect(part).toBeVisible();
    await expect(bar(page).locator(".b-nav")).toBeHidden();
    // (Well into the section: the touch back up below stays in it.)
    await page.evaluate(() => scrollTo(0, document.getElementById("lyrics")!.getBoundingClientRect().top + scrollY + 240));
    await expect(part.locator("svg[data-on]")).toHaveAttribute("data-for", "lyrics");
    // A touch back up, as a hand does, brings the tucked capsule back.
    for (let i = 0; i < 3; i++) {
      await page.evaluate(() => scrollBy(0, -30));
      await page.waitForTimeout(32);
    }
    await expect(bar(page)).not.toHaveAttribute("data-tucked", /.*/);
    await waitSettled(page);
    await part.tap();
    const fly = page.locator("#fly-secs");
    await expect(fly).toBeVisible();
    await expect(part).toHaveAttribute("aria-expanded", "true");
    await expect(fly.locator('[aria-current="true"]')).toHaveAttribute("data-sec", "lyrics");
    await fly.getByRole("link", { name: /Questions/ }).tap();
    await expect(fly).toBeHidden();
    await expect.poll(() => page.evaluate(() => Math.abs(document.getElementById("questions")!.getBoundingClientRect().top) < 120)).toBe(true);
  });
  test("the capsule steps up out of the reading while the page moves down, and is back as soon as it moves up", async ({ page }) => {
    await page.goto("/");
    const bar = page.locator("[data-site-bar]");
    for (let i = 0; i < 20; i++) {
      await page.evaluate(() => scrollBy(0, 60));
      await page.waitForTimeout(32);
    }
    await expect(bar).toHaveAttribute("data-state", "capsule");
    await expect(bar).toHaveAttribute("data-tucked", "");
    for (let i = 0; i < 3; i++) {
      await page.evaluate(() => scrollBy(0, -30));
      await page.waitForTimeout(32);
    }
    await expect(bar).not.toHaveAttribute("data-tucked");
  });
});

test("on a desktop the capsule stays while the page moves down", async ({ page }) => {
  await page.goto("/");
  for (let i = 0; i < 20; i++) {
    await page.mouse.wheel(0, 120);
    await page.waitForTimeout(32);
  }
  await expect(page.locator("[data-site-bar]")).toHaveAttribute("data-state", "capsule");
  await expect(page.locator("[data-site-bar]")).not.toHaveAttribute("data-tucked");
});
