import { expect, test } from "@playwright/test";

// The small live pieces all follow the page's one player: one song, one clock.
test.use({ viewport: { width: 1440, height: 900 } });

// No clip loads, so the clock runs on its own (the sound would hold it to real time).
test.beforeEach(async ({ page }) => {
  await page.route("**/music/**", (r) => r.abort());
});

test("pressing play anywhere plays the one song everywhere: the media keys, the tray's tooltip, Discord, the system's controls", async ({ page }) => {
  await page.goto("/");
  const kept = page.locator("#kept");
  await kept.locator(".m-media [data-do='toggle']").click();
  await expect(page.locator("html")).toHaveAttribute("data-playing", "true");
  await expect(page.locator("[data-play-label]").first()).toHaveAttribute("aria-label", "Pause");
  await expect(kept.locator("[data-tray-2]")).toHaveText(/^Josh Woodward, 0:2\d of 0:54$/);
  await expect(kept.locator("[data-piece='discord']")).toHaveAttribute("data-on", "");
  expect(await page.evaluate(() => navigator.mediaSession.metadata?.title)).toBe("California Lullabye");
  expect(await page.evaluate(() => navigator.mediaSession.playbackState)).toBe("playing");
  // Its progress line runs on the compositor: one animation, not a style per frame.
  expect(await kept.locator(".m-media [data-fill]").evaluate((el) => el.getAnimations().length)).toBe(1);
  // Next, from the media keys: every piece shows the next song.
  await kept.locator(".m-media [data-do='next']").click();
  for (const piece of ["media", "discord", "scrobble", "sponsor"]) {
    await expect(kept.locator(`[data-piece='${piece}'] [data-bind='title']`).first()).toHaveText("Invisible Light");
  }
  await expect(kept.locator("[data-tray-1]")).toHaveText("Invisible Light");
  // SponsorBlock hops the new song's opening talk.
  await expect(kept.locator("[data-piece='sponsor']")).toHaveAttribute("data-hop", "a");
  await kept.locator(".m-media [data-do='toggle']").click();
  await expect(kept.locator("[data-tray-2]")).toHaveText(/^Josh Woodward, paused at 0:\d\d$/);
});

test("the scrobble ring fills with the time heard, closes at half the song with a tick, and the song joins the past ones", async ({ page }) => {
  await page.clock.install();
  await page.goto("/");
  const sc = page.locator("[data-piece='scrobble']");
  await expect(sc.locator("[data-sc-label]")).toHaveText("Paused");
  await page.locator("#kept .m-media [data-do='toggle']").click();
  await page.clock.runFor(10_000);
  await expect(sc.locator("[data-sc-label]")).toHaveText("Scrobbling now");
  const p = Number(await sc.locator("[data-sc-ring]").evaluate((el) => getComputedStyle(el).getPropertyValue("--p")));
  expect(p).toBeGreaterThan(0.3);
  expect(p).toBeLessThan(0.45);
  await page.clock.runFor(18_000);
  await expect(sc.locator("[data-sc-label]")).toHaveText("Scrobbled");
  await expect(sc.locator("[data-sc-row]")).toHaveAttribute("data-scrobbled", "");
  await page.locator("#kept .m-media [data-do='next']").click();
  await expect(sc.locator("[data-sc-past] li")).toHaveCount(1);
  await expect(sc.locator("[data-sc-past] li b")).toHaveText("California Lullabye");
  await expect(sc.locator("[data-sc-label]")).toHaveText("Scrobbling now");
});

test("the lyric card follows the song, and a click on a line plays from there", async ({ page }) => {
  await page.goto("/");
  const card = page.locator("[data-piece='lyrics']");
  await card.scrollIntoViewIfNeeded();
  await expect(card.locator(".lc-line[data-state='current']")).toHaveText("So let me shake away the cloudy skies");
  await card.locator(".lc-line", { hasText: "We'll put our toes up to the fire" }).first().click();
  await expect(card.locator(".lc-line[data-state='current']")).toHaveText("We'll put our toes up to the fire");
  await expect(page.locator("html")).toHaveAttribute("data-playing", "true");
  // The window's own lyrics are on the same line: one player.
  await expect(page.locator("[data-window] [data-lines] .line[data-state='current']")).toHaveText("We'll put our toes up to the fire");
});
