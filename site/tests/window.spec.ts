import { expect, test, type Page } from "@playwright/test";
import { wake, windowBox } from "./helpers";

// featherplay's window on the page: the app laid out for the window's size, drawn to fit, and
// a window: dragged, maximised and restored, minimised, closed with the app's own question.
test.use({ viewport: { width: 1440, height: 900 } });

const win = (page: Page) => page.locator("[data-window]");
const replica = (page: Page) => win(page).locator("[data-replica]");
const caption = (page: Page, name: RegExp) => replica(page).getByRole("button", { name });
/** Maximise and restore are a transition of the window; it can be pressed again once it ends. */
const morphed = (page: Page) => expect(page.locator("html")).not.toHaveAttribute("data-win-morph", /.*/);

test("the window is never wider or taller than the viewport, and on a wide screen it lies wholly inside it", async ({ page }) => {
  for (const [w, h] of [
    [320, 600],
    [390, 844],
    [700, 1000],
    [761, 600],
    [1024, 768],
    [1280, 600],
    [1280, 720],
    [1366, 768],
    [1440, 900],
    [1600, 900],
    [1920, 1080],
    [2560, 1270],
    [3840, 2160],
  ]) {
    await page.setViewportSize({ width: w, height: h });
    await page.goto("/");
    await wake(page);
    const b = await windowBox(page);
    const vw = await page.evaluate(() => document.documentElement.clientWidth);
    expect(b.width, `${w}x${h}`).toBeLessThanOrEqual(vw + 0.5);
    expect(b.height, `${w}x${h}`).toBeLessThanOrEqual(h + 0.5);
    expect(b.x, `${w}x${h}`).toBeGreaterThanOrEqual(-0.5);
    expect(b.x + b.width, `${w}x${h}`).toBeLessThanOrEqual(vw + 0.5);
    if (w >= 761) {
      expect(b.y, `${w}x${h}`).toBeGreaterThanOrEqual(-0.5);
      expect(b.y + b.height, `${w}x${h}`).toBeLessThanOrEqual(h + 0.5);
    }
  }
});

test("on a phone the window is the app's compact layout, whole on the first screen with its bar", async ({ page }) => {
  for (const [w, h] of [[390, 844], [360, 740]] as const) {
    await page.setViewportSize({ width: w, height: h });
    await page.goto("/");
    await wake(page);
    const r = replica(page);
    expect(await r.evaluate((el) => el.clientWidth)).toBeLessThan(800);
    await expect(r.locator("[data-queue]")).toBeHidden();
    const cover = (await r.locator(".np-cover").boundingBox())!;
    const lines = (await r.locator("[data-lyrics]").boundingBox())!;
    expect(lines.y).toBeGreaterThan(cover.y + cover.height - 1);
    // The bar and its play button on the first screen, without scrolling.
    await expect(r.locator(".bar .fp-play")).toBeInViewport({ ratio: 1 });
    const box = await windowBox(page);
    expect(box.y + box.height, `${w}x${h}`).toBeLessThanOrEqual(h);
  }
});

test("the caption buttons keep one row inside the title bar at every width, never on top of each other", async ({ page }) => {
  for (const [w, h] of [
    [320, 700],
    [390, 844],
    [761, 700],
    [900, 700],
    [1440, 900],
  ]) {
    await page.setViewportSize({ width: w, height: h });
    await page.goto("/");
    await wake(page);
    const bar = (await replica(page).locator(".titlebar").boundingBox())!;
    const boxes = [];
    for (const name of [/^Minimise/, /^Maximise/, /^Close/]) boxes.push((await caption(page, name).boundingBox())!);
    for (let i = 0; i < 3; i++) {
      expect(Math.abs(boxes[i].y - boxes[0].y), `${w}: one row`).toBeLessThan(1);
      expect(boxes[i].x + boxes[i].width, `${w}: inside the bar`).toBeLessThanOrEqual(bar.x + bar.width + 0.5);
      expect(boxes[i].y, `${w}: inside the bar`).toBeGreaterThanOrEqual(bar.y - 0.5);
      if (i) expect(boxes[i].x, `${w}: apart`).toBeGreaterThanOrEqual(boxes[i - 1].x + boxes[i - 1].width - 0.5);
    }
  }
});

test("dragged by its title bar, the window moves and stays wholly in view", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  const before = await windowBox(page);
  const bar = (await replica(page).locator(".titlebar-name").boundingBox())!;
  await page.mouse.move(bar.x + 5, bar.y + bar.height / 2);
  await page.mouse.down();
  await page.mouse.move(bar.x - 200, bar.y - 120, { steps: 6 });
  await page.mouse.up();
  const moved = await windowBox(page);
  expect(moved.x).toBeLessThan(before.x - 150);
  // Far past the bottom edge: it stops there.
  await page.mouse.move(moved.x + 60, moved.y + 10);
  await page.mouse.down();
  await page.mouse.move(moved.x + 60, 3000, { steps: 6 });
  await page.mouse.up();
  const end = await windowBox(page);
  expect(end.width).toBeCloseTo(before.width, 0);
  expect(end.y + end.height).toBeLessThanOrEqual(900.5);
});

test("pushed against the screen's side the window snaps to that half, uncovering its desk; the shortcut there takes it home", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  const home = await windowBox(page);
  const desk = page.locator("[data-win-gone]");
  const grab = async () => {
    const t = (await replica(page).locator(".titlebar-name").boundingBox())!;
    await page.mouse.move(t.x + 5, t.y + t.height / 2);
    await page.mouse.down();
    return t;
  };
  await expect(desk).toBeHidden();
  const t = await grab();
  await page.mouse.move(-300, t.y + 100, { steps: 8 });
  // The place it would take shows first.
  await expect(page.locator("[data-win-snap]")).toHaveAttribute("data-on", "left");
  await page.mouse.up();
  await expect(win(page)).toHaveAttribute("data-tile", "left");
  await expect.poll(async () => Math.round((await windowBox(page)).width)).toBe(720);
  const half = await windowBox(page);
  expect(half.x).toBeCloseTo(0, 0);
  expect(half.y + half.height).toBeCloseTo(900, 0);
  expect(await replica(page).evaluate((el) => el.clientWidth)).toBe(720);
  await morphed(page);
  // The desk is where the window lives (not where it went), its shortcut the way back.
  const back = desk.getByRole("button", { name: "Move featherplay’s window back" });
  await expect(back).toBeVisible();
  await expect(desk).toContainText("this takes it home");
  const b = (await back.boundingBox())!;
  expect(Math.abs(b.x + b.width / 2 - (home.x + home.width / 2))).toBeLessThan(2);
  // A press on its title bar alone leaves it snapped; a drag brings it back to its size.
  const t2 = await grab();
  await page.mouse.up();
  await expect(win(page)).toHaveAttribute("data-tile", "left");
  await grab();
  await page.mouse.move(t2.x + 60, t2.y + 40, { steps: 6 });
  await page.mouse.up();
  await expect(win(page)).not.toHaveAttribute("data-tile", /.*/);
  expect((await windowBox(page)).width).toBeCloseTo(home.width, 0);
  await back.click();
  await expect.poll(async () => Math.round((await windowBox(page)).x)).toBe(Math.round(home.x));
  await expect(desk).toBeHidden();
  // Snapped again and sent to the tray, it forgets the side: its shortcut opens it at home.
  const t3 = await grab();
  await page.mouse.move(-300, t3.y + 100, { steps: 8 });
  await page.mouse.up();
  await expect(win(page)).toHaveAttribute("data-tile", "left");
  await morphed(page);
  await caption(page, /^Minimise/).click();
  const show = desk.getByRole("button", { name: "Show featherplay’s window" });
  await expect(show).toBeVisible();
  expect(Math.abs((await show.boundingBox())!.x - b.x)).toBeLessThan(2);
  await show.click();
  await expect(win(page)).not.toHaveAttribute("data-minimised", /.*/);
  await expect(win(page)).not.toHaveAttribute("data-tile", /.*/);
  await expect.poll(async () => Math.round((await windowBox(page)).x)).toBe(Math.round(home.x));
  expect((await windowBox(page)).width).toBeCloseTo(home.width, 0);
  await expect(desk).toBeHidden();
});

test("maximise fills the screen under the bar; restore and a double-click on the title bar bring it back", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  const before = await windowBox(page);
  await caption(page, /^Maximise/).click();
  await expect(win(page)).toHaveAttribute("data-max", "");
  // Its name says what it does next, as Windows' does; not also "pressed".
  await expect(caption(page, /^Restore/)).toBeVisible();
  expect(await caption(page, /^Restore/).getAttribute("aria-pressed")).toBeNull();
  await expect.poll(async () => (await windowBox(page)).width).toBeGreaterThan(before.width + 200);
  // The whole screen under the bar, the app laid out for it at its real size.
  const max = await windowBox(page);
  expect(max.x).toBeCloseTo(0, 0);
  expect(max.width).toBeCloseTo(1440, 0);
  expect(max.y + max.height).toBeCloseTo(900, 0);
  expect(await replica(page).evaluate((el) => el.clientWidth)).toBe(1440);
  await morphed(page);
  await caption(page, /^Restore/).click();
  await expect.poll(async () => Math.round((await windowBox(page)).width)).toBe(Math.round(before.width));
  expect(await replica(page).evaluate((el) => el.clientWidth)).toBe(1440);
  await morphed(page);
  await replica(page).locator(".titlebar-name").dblclick();
  await expect(win(page)).toHaveAttribute("data-max", "");
});

test("dragged out of maximised, the window's button says Maximise again", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  await caption(page, /^Maximise/).click();
  await expect(win(page)).toHaveAttribute("data-max", "");
  await page.waitForTimeout(400);
  const t = (await replica(page).locator(".titlebar-name").boundingBox())!;
  await page.mouse.move(t.x + 5, t.y + t.height / 2);
  await page.mouse.down();
  await page.mouse.move(t.x + 200, t.y + 160, { steps: 6 });
  await page.mouse.up();
  await expect(win(page)).not.toHaveAttribute("data-max", /.*/);
  await expect(caption(page, /^Maximise/)).toBeVisible();
});

test("on a phone the window can't be maximised, and its button says so", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/");
  await wake(page);
  await expect(caption(page, /^Maximise/)).toHaveAttribute("aria-disabled", "true");
  await page.setViewportSize({ width: 1440, height: 900 });
  await expect(caption(page, /^Maximise/)).toHaveAttribute("aria-disabled", "false");
});

test("minimised from the keyboard, the keys go to the tray icon the window went into", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  await caption(page, /^Minimise/).focus();
  await page.keyboard.press("Enter");
  await expect(win(page)).toHaveAttribute("data-minimised", "");
  await expect(page.locator("[data-site-bar] [data-tray]")).toBeFocused();
});

test("closed to the tray from the keyboard, the keys go to the tray icon too", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  await caption(page, /^Close/).focus();
  await page.keyboard.press("Enter");
  await page.getByRole("button", { name: "Keep playing in the tray" }).focus();
  await page.keyboard.press("Enter");
  await expect(win(page)).toHaveAttribute("data-minimised", "");
  await expect(page.locator("[data-site-bar] [data-tray]")).toBeFocused();
});

test("restored part-way into the tray, the window glides back from where it had got to, no jump", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  // Minimised, and 60 ms into its glide restored (the tray icon): where it starts back from is
  // where it had got to.
  const { mid, first } = await page.evaluate(
    () =>
      new Promise<{ mid: { x: number; y: number }; first: { x: number; y: number } }>((r) => {
        const el = document.querySelector<HTMLElement>("[data-window]")!;
        const at = () => {
          const m = new DOMMatrixReadOnly(getComputedStyle(el).transform);
          return { x: m.e, y: m.f };
        };
        el.querySelector<HTMLElement>('[data-act="minimise"]')!.click();
        setTimeout(() => {
          const mid = at();
          (document.querySelector("[data-site-bar] [data-tray]") as HTMLElement).click();
          r({ mid, first: at() });
        }, 60);
      }),
  );
  expect(Math.hypot(first.x - mid.x, first.y - mid.y)).toBeLessThan(12);
});

test("laid out again part-way through the story (a resize there), the window is still its size back at the top", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  const home = await windowBox(page);
  await page.evaluate(() => scrollTo(0, 0.12 * 2400));
  await page.waitForTimeout(500);
  await page.setViewportSize({ width: 1441, height: 900 });
  await page.waitForTimeout(200);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.waitForTimeout(200);
  await page.evaluate(() => scrollTo(0, 0));
  await expect.poll(async () => Math.round((await windowBox(page)).height)).toBe(Math.round(home.height));
  expect(Math.round((await windowBox(page)).width)).toBe(Math.round(home.width));
});

test.describe("where the story doesn't run (a short screen)", () => {
  test.use({ viewport: { width: 1280, height: 560 } });
  test("a maximised window stays on the first screen: scrolled down and resized, it never covers the sections after it", async ({ page }) => {
    await page.goto("/");
    await wake(page);
    await caption(page, /^Maximise/).click();
    await expect(win(page)).toHaveAttribute("data-max", "");
    await page.evaluate(() => scrollTo(0, 700));
    await page.setViewportSize({ width: 1260, height: 560 });
    await page.waitForTimeout(400);
    const stage = await page.locator("[data-stage]").evaluate((s) => s.getBoundingClientRect().bottom);
    expect((await windowBox(page)).y + (await windowBox(page)).height).toBeLessThanOrEqual(stage + 1);
  });
});

test("while the window is away its shortcut lies where it stood and brings it back, with the note for how it went", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  const home = await windowBox(page);
  const place = page.locator("[data-win-gone]");
  await expect(place).toBeHidden();
  await caption(page, /^Close/).click();
  await page.getByRole("dialog").getByRole("button", { name: "Close featherplay" }).click();
  await expect(place).toBeVisible();
  await expect(place).toContainText("now it really isn’t running");
  // The bar's tray icon says it is closed, not in the tray.
  await expect(page.locator("html")).toHaveAttribute("data-window-closed", "");
  const trayIcon = page.locator("[data-site-bar] [data-tray]");
  await expect(trayIcon).toHaveAccessibleName("featherplay is closed: open it");
  // No pop-up of the player from a closed app's icon.
  await trayIcon.hover();
  await page.waitForTimeout(500);
  await expect(page.locator("#fly-song")).toBeHidden();
  // Its shortcut in the middle of where the window stood, a listener's other icons about it,
  // each inside that place: pressed, one says what it has to do with featherplay.
  const b = (await place.getByRole("button", { name: "Open featherplay" }).boundingBox())!;
  expect(Math.abs(b.x + b.width / 2 - (home.x + home.width / 2))).toBeLessThan(2);
  expect(Math.abs(b.y + b.height / 2 - (home.y + home.height / 2))).toBeLessThan(2);
  for (const name of ["Discord", "Last.fm", "OBS Studio", "Recycle Bin"]) {
    const i = (await place.getByRole("button", { name }).boundingBox())!;
    expect(i.x, name).toBeGreaterThanOrEqual(home.x);
    expect(i.x + i.width, name).toBeLessThanOrEqual(home.x + home.width);
    expect(i.y, name).toBeGreaterThanOrEqual(home.y);
    expect(i.y + i.height, name).toBeLessThanOrEqual(home.y + home.height);
  }
  await place.getByRole("button", { name: "Last.fm" }).click();
  await expect(place.getByRole("status")).toHaveText("Songs you play past halfway are scrobbled there.");
  await place.getByRole("button", { name: "Open featherplay" }).click();
  await expect(win(page)).not.toHaveAttribute("data-minimised", /.*/);
  await expect(place).toBeHidden();
  await expect(page.locator("html")).not.toHaveAttribute("data-window-closed", /.*/);
  // "opens in 0.24 s" is for the app opening after a close, every time; not for a window
  // shown again from the tray.
  const opens = page.locator("[data-note='opens']");
  await expect(opens).toHaveAttribute("data-on", "");
  await wake(page);
  await caption(page, /^Minimise/).click();
  await expect(place).toBeVisible();
  await trayIcon.click();
  await expect(win(page)).not.toHaveAttribute("data-minimised", /.*/);
  await expect(opens).not.toHaveAttribute("data-on", /.*/);
  await wake(page);
  await caption(page, /^Close/).click();
  await page.getByRole("dialog").getByRole("button", { name: "Close featherplay" }).click();
  await expect(place).toBeVisible();
  await trayIcon.click();
  await expect(win(page)).not.toHaveAttribute("data-minimised", /.*/);
  await expect(opens).toHaveAttribute("data-on", "");
  await wake(page);
  await caption(page, /^Minimise/).click();
  await expect(place.getByRole("button", { name: "Show featherplay’s window" })).toBeVisible();
  await expect(place).toContainText("still running. easy to forget");
});

test("minimise sends the window to the tray: hidden and out of the keyboard's way", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  await caption(page, /^Minimise/).click();
  await expect(win(page)).toHaveAttribute("data-minimised", "");
  expect(await win(page).evaluate((el) => el.inert)).toBe(true);
  await expect(win(page)).toBeHidden();
});

test("close asks first, as the app does; Escape keeps the window and gives the focus back", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  const close = caption(page, /^Close/);
  await close.focus();
  await page.keyboard.press("Enter");
  const dialog = page.getByRole("dialog", { name: "Close featherplay?" });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole("button", { name: "Keep playing in the tray" })).toBeFocused();
  // Modal: Tab stays inside it.
  for (let i = 0; i < 4; i++) await page.keyboard.press("Tab");
  expect(await dialog.evaluate((d) => d.contains(document.activeElement))).toBe(true);
  await page.keyboard.press("Escape");
  await expect(dialog).toBeHidden();
  await expect(close).toBeFocused();
  await expect(win(page)).not.toHaveAttribute("data-minimised", /.*/);
});

test("closing to the tray keeps the song playing; closing featherplay stops it", async ({ page }) => {
  await page.route("**/music/**", (r) => r.abort());
  await page.goto("/");
  await wake(page);
  await replica(page).locator(".bar .fp-play").click();
  await caption(page, /^Close/).click();
  await page.getByRole("button", { name: "Keep playing in the tray" }).click();
  await expect(win(page)).toHaveAttribute("data-minimised", "");
  await expect(page.locator("html")).toHaveAttribute("data-playing", "true");
  await page.reload();
  await wake(page);
  await replica(page).locator(".bar .fp-play").click();
  await caption(page, /^Close/).click();
  await page.getByRole("dialog").getByRole("button", { name: "Close featherplay" }).click();
  await expect(win(page)).toHaveAttribute("data-minimised", "");
  await expect(page.locator("html")).toHaveAttribute("data-playing", "false");
});

test("the wheel over the window scrolls the page until the visitor clicks into it", async ({ page }) => {
  // The window still (no scroll story), so the wheel's effect on the page is plain to see.
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/");
  await wake(page);
  const lines = (await replica(page).locator("[data-lyrics]").boundingBox())!;
  await page.mouse.move(lines.x + lines.width / 2, lines.y + lines.height / 2);
  await page.mouse.wheel(0, 300);
  await expect.poll(() => page.evaluate(() => scrollY)).toBeGreaterThan(100);
  await page.evaluate(() => scrollTo(0, 0));
  await expect.poll(() => page.evaluate(() => scrollY)).toBe(0);
  // Clicked into: the wheel is the app's (the lyrics move), the page stays.
  await page.mouse.click(lines.x + 4, lines.y + lines.height - 20);
  const shift = () => replica(page).locator("[data-lines]").evaluate((el) => getComputedStyle(el).transform);
  const t0 = await shift();
  await page.mouse.wheel(0, 200);
  await expect.poll(shift).not.toBe(t0);
  expect(await page.evaluate(() => scrollY)).toBe(0);
});

test("a selection in the window moves with it: its drawn pieces follow a drag", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  // The song's title in the window, selected.
  const sel = () =>
    page.evaluate(() => {
      const r = document.getSelection()!.getRangeAt(0).getBoundingClientRect();
      const p = document.querySelector(".sel-layer > span")!.getBoundingClientRect();
      return { dx: Math.abs(p.left - r.left), dy: Math.abs(p.top - r.top) };
    });
  await page.evaluate(() => {
    const el = document.querySelector("[data-window] [data-replica] .np-title")!;
    const r = document.createRange();
    r.selectNodeContents(el);
    document.getSelection()!.removeAllRanges();
    document.getSelection()!.addRange(r);
  });
  await expect(page.locator(".sel-layer > span").first()).toBeVisible();
  await page.waitForTimeout(300);
  const at = await sel();
  expect(at.dx + at.dy).toBeLessThan(12);
  const bar = (await replica(page).locator(".titlebar-name").boundingBox())!;
  await page.mouse.move(bar.x + 5, bar.y + bar.height / 2);
  await page.mouse.down();
  await page.mouse.move(bar.x - 200, bar.y + 120, { steps: 6 });
  await page.mouse.up();
  await page.waitForTimeout(300);
  const after = await sel();
  expect(Math.abs(after.dx - at.dx) + Math.abs(after.dy - at.dy)).toBeLessThan(3);
});

for (const [scheme, want] of [["light", "rgba(0, 0, 0, 0.18)"], ["dark", "rgba(0, 0, 0, 0.5)"]] as const) {
  test(`the close question dims the page by the theme's own scrim (${scheme})`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: scheme });
    await page.goto("/");
    await wake(page);
    await caption(page, /^Close/).click();
    const dialog = page.getByRole("dialog", { name: "Close featherplay?" });
    await expect(dialog).toBeVisible();
    expect(await dialog.evaluate((d) => getComputedStyle(d, "::backdrop").backgroundColor)).toBe(want);
  });
}
