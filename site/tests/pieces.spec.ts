import { expect, test, type Page } from "@playwright/test";

// The small live pieces all follow the page's one player: one song, one clock.
test.use({ viewport: { width: 1440, height: 900 } });

// No clip loads, so the clock runs on its own (the sound would hold it to real time).
test.beforeEach(async ({ page }) => {
  await page.route("**/music/**", (r) => r.abort());
});

test("pressing play anywhere plays the one song everywhere: the media keys, the tray's tooltip, Discord, the system's controls", async ({ page }) => {
  await page.goto("/");
  const kept = page.locator("#kept");
  // The pieces in view (out of view they hold still).
  await kept.locator("[data-piece='sponsor']").scrollIntoViewIfNeeded();
  await kept.locator(".m-media [data-do='toggle']").click();
  await expect(page.locator("html")).toHaveAttribute("data-playing", "true");
  await expect(page.locator("[data-play-label]").first()).toHaveAttribute("aria-label", "Pause");
  await expect(kept.locator("[data-tray-2]")).toHaveText(/^Josh Woodward, 0:2\d of 0:54$/);
  await expect(kept.locator("[data-piece='discord']")).toHaveAttribute("data-on", "");
  expect(await page.evaluate(() => navigator.mediaSession.metadata?.title)).toBe("California Lullabye");
  expect(await page.evaluate(() => navigator.mediaSession.playbackState)).toBe("playing");
  // Its progress line runs on the compositor: one animation, not a style per frame.
  expect(await kept.locator(".m-media [data-fill]").evaluate((el) => el.getAnimations().length)).toBe(1);
  // Next, from the media keys: every piece shows the next song. (Each hop the sponsor piece
  // makes is noted, so a slow step here can't miss it: it lasts some 1.9 s.)
  await kept.locator("[data-piece='sponsor']").evaluate((s) => {
    (window as unknown as { hops: string[] }).hops = [];
    new MutationObserver(() => s.dataset.hop && (window as unknown as { hops: string[] }).hops.push(s.dataset.hop)).observe(s, { attributes: true, attributeFilter: ["data-hop"] });
  });
  // (Clicked where it is: the sponsor piece stays in view, as a visitor's would below it.)
  await kept.locator("[data-piece='sponsor']").scrollIntoViewIfNeeded();
  await kept.locator(".m-media [data-do='next']").evaluate((b: HTMLElement) => b.click());
  for (const piece of ["media", "discord", "scrobble", "sponsor"]) {
    await expect(kept.locator(`[data-piece='${piece}'] [data-bind='title']`).first()).toHaveText("Invisible Light");
  }
  await expect(kept.locator("[data-tray-1]")).toHaveText("Invisible Light");
  // SponsorBlock hops the new song's opening talk.
  await expect.poll(() => page.evaluate(() => (window as unknown as { hops: string[] }).hops)).toContain("a");
  await kept.locator(".m-media [data-do='toggle']").click();
  await expect(kept.locator("[data-tray-2]")).toHaveText(/^Josh Woodward, paused at 0:\d\d$/);
});

/** The page's Media Session handlers, kept where a test can call them as the system would when
 *  the visitor presses a media key. */
const catchKeys = (page: Page) =>
  page.addInitScript(() => {
    const ms = navigator.mediaSession;
    const handlers: Record<string, MediaSessionActionHandler | null> = {};
    const set = ms.setActionHandler.bind(ms);
    ms.setActionHandler = (action, fn) => {
      handlers[action] = fn;
      set(action, fn);
    };
    (window as unknown as { keys: typeof handlers }).keys = handlers;
  });
const key = (page: Page, action: string, seekTime?: number) =>
  page.evaluate(([action, seekTime]) => (window as unknown as { keys: Record<string, (d: object) => void> }).keys[action as string]({ action, seekTime }), [action, seekTime] as const);

test("the key caps press for the visitor's own media keys and for the overlay's buttons", async ({ page }) => {
  await catchKeys(page);
  await page.goto("/");
  const stage = page.locator("#kept [data-keys-stage]");
  await stage.scrollIntoViewIfNeeded();
  await stage.evaluate((s) => {
    const w = window as unknown as { pressed: string[] };
    w.pressed = [];
    // Each time a cap goes down (the attribute added).
    new MutationObserver((rs) => rs.forEach((r) => r.oldValue === null && w.pressed.push((r.target as HTMLElement).dataset.key!))).observe(s, { attributes: true, subtree: true, attributeFilter: ["data-pressed"], attributeOldValue: true });
  });
  await stage.locator(".m-media [data-do='toggle']").click();
  await expect(page.locator("html")).toHaveAttribute("data-playing", "true");
  await key(page, "nexttrack");
  await expect(stage.locator(".m-media [data-bind='title']")).toHaveText("Invisible Light");
  await key(page, "pause");
  await expect(page.locator("html")).toHaveAttribute("data-playing", "false");
  await stage.locator(".m-media [data-do='previous']").click();
  expect(await page.evaluate(() => (window as unknown as { pressed: string[] }).pressed)).toEqual(["toggle", "next", "toggle", "previous"]);
  // The caps are the keyboard's way in: real buttons, each named.
  await expect(stage.getByRole("button", { name: "Next" })).toHaveCount(2);
  // Stop is a key too: it pauses, as the app answers it.
  await stage.locator(".m-media [data-do='toggle']").click();
  await expect(page.locator("html")).toHaveAttribute("data-playing", "true");
  await key(page, "stop");
  await expect(page.locator("html")).toHaveAttribute("data-playing", "false");
  await expect(stage.getByRole("button", { name: "Stop" })).toHaveCount(1);
});

test("scrobbling counts the time heard, not a seek, and at half the song drops it into the list", async ({ page }) => {
  await catchKeys(page);
  await page.clock.install();
  await page.goto("/");
  const sc = page.locator("[data-piece='scrobble']");
  await expect(sc.locator("[data-sc-label]")).toHaveText("Paused");
  // The page's sections laid out first (release.ts runs on the page's idle time, which the clock
  // holds), the piece in view.
  await page.clock.runFor(2000);
  await sc.scrollIntoViewIfNeeded();
  await expect(sc.locator(".sc-empty")).toBeVisible();
  await page.locator("#kept .m-media [data-do='toggle']").click();
  // Back to the piece (the press may have scrolled to the overlay): out of view it holds still.
  await sc.scrollIntoViewIfNeeded();
  await page.clock.runFor(10_000);
  await expect(sc.locator("[data-sc-label]")).toHaveText("Scrobbling now");
  await expect(sc.locator("[data-sc-heard]")).toHaveText(/^0:(09|10)$/);
  // A seek forward: the song plays out with less than half of it heard, so it isn't scrobbled.
  await key(page, "seekto", 40);
  await page.clock.runFor(16_000);
  await expect(sc.locator("[data-bind='title']")).toHaveText("Invisible Light");
  await expect(sc.locator("[data-sc-past] li")).toHaveCount(0);
  // The next one heard to its half (29.95 s of 59.9): scrobbled, and in the list at once.
  await page.clock.runFor(31_000);
  await expect(sc.locator("[data-sc-label]")).toHaveText("Scrobbled");
  await expect(sc).toHaveAttribute("data-scrobbled", "");
  await expect(sc.locator("[data-sc-past] li")).toHaveCount(1);
  await expect(sc.locator("[data-sc-past] li b")).toHaveText("Invisible Light");
  await expect(sc.locator(".sc-empty")).toBeHidden();
  // Back to its start, it stays scrobbled once.
  await key(page, "seekto", 0);
  await page.clock.runFor(2000);
  await expect(sc.locator("[data-sc-label]")).toHaveText("Scrobbled");
  await expect(sc.locator("[data-sc-past] li")).toHaveCount(1);
});

test("the lyric card follows the song, and a click on a line plays from there", async ({ page }) => {
  await page.goto("/");
  const card = page.locator("[data-piece='lyrics']");
  await card.scrollIntoViewIfNeeded();
  await expect(card.locator(".lc-line[data-state='current']")).toHaveText("So let me shake away the cloudy skies");
  await card.locator(".lc-line", { hasText: "We'll put our toes up to the fire" }).first().click();
  await expect(card.locator(".lc-line[data-state='current']")).toHaveText("We'll put our toes up to the fire");
  await expect(page.locator("html")).toHaveAttribute("data-playing", "true");
  // The window's own lyrics are on the same line, once it is in view again: one player.
  await page.evaluate(() => scrollTo(0, 0));
  await expect(page.locator("[data-window] [data-lines] .line[data-state='current']")).toHaveText(/We'll put our toes up to the fire|And I will sing your cares away/);
});

test("the tray cell's arrow leads from the close question to the tray at every width", async ({ page }) => {
  await page.goto("/");
  const cell = page.locator("#kept .c-tray");
  for (const w of [761, 900, 1024, 1100, 1280, 1440, 1600, 1800, 2000]) {
    await page.setViewportSize({ width: w, height: 900 });
    await cell.scrollIntoViewIfNeeded();
    const [one, arrow, two] = await cell.locator(".step, .step-arrow").evaluateAll((els) => els.map((e) => e.getBoundingClientRect().toJSON() as DOMRect));
    const side = two.left >= one.right;
    // Beside: the arrow between them, left to right; stacked: between them, top to bottom.
    if (side) expect(arrow.left >= one.right - 1 && arrow.right <= two.left + 1, `${w} beside`).toBe(true);
    else expect(arrow.top >= one.bottom - 1 && arrow.bottom <= two.top + 1 && arrow.height > arrow.width, `${w} stacked`).toBe(true);
  }
});
