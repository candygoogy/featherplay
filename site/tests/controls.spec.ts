import { expect, test, type Page } from "@playwright/test";
import { skipTour, wake } from "./helpers";

// The replica's controls behave as the app's do (apps/featherplay/ui: bottom_bar.slint,
// parts.slint, pages/now_playing.slint, sidebar.slint).

test.use({ viewport: { width: 1440, height: 900 } });
const replica = (page: Page) => page.locator("[data-hero] [data-replica]");
const bar = (page: Page) => replica(page).locator("[data-bar]");

async function ready(page: Page) {
  await page.goto("/");
  await wake(page);
  await skipTour(page);
}

test("the play button morphs into a rounded square while playing, the glyph turning", async ({ page }) => {
  await ready(page);
  const play = bar(page).locator(".fp-play");
  await expect(play).toHaveCSS("border-top-left-radius", "24px");
  await play.click();
  await expect(play).toHaveAttribute("aria-label", "Pause");
  await expect.poll(() => play.evaluate((el) => getComputedStyle(el).borderTopLeftRadius)).toBe("14.4px");
  await expect(play.locator(".i-pause")).toHaveCSS("opacity", "1");
  await expect(play.locator(".i-play")).toHaveCSS("opacity", "0");
});

test("like fills the heart in text1, everywhere it shows, and lists the song in Library", async ({ page }) => {
  await ready(page);
  const like = bar(page).getByRole("button", { name: "Like", exact: true });
  await like.click();
  await expect(bar(page).getByRole("button", { name: "Remove like" })).toHaveAttribute("aria-pressed", "true");
  const fill = await bar(page).locator(".like svg").evaluate((el) => getComputedStyle(el).fill);
  expect(fill).not.toBe("none");
  await replica(page).getByRole("button", { name: "Back", exact: true }).click();
  await replica(page).getByRole("button", { name: "Library", exact: true }).click();
  await expect(replica(page).locator("[data-liked-list] [data-row]")).toHaveCount(1);
});

test("Not for me skips the song, says so in a toast, and Undo brings it back", async ({ page }) => {
  await ready(page);
  const title = bar(page).locator("[data-bar-title]");
  const before = await title.textContent();
  await bar(page).getByRole("button", { name: "Not for me" }).click();
  await expect(title).not.toHaveText(before!);
  const toast = replica(page).locator("[data-toast]");
  await expect(toast).toContainText("Marked as not for me");
  await toast.getByRole("button", { name: "Undo" }).click();
  await expect(title).toHaveText(before!);
  await expect(toast).toBeHidden();
});

test("Up next and Related are tabs; Related lists songs like this one", async ({ page }) => {
  await ready(page);
  const related = replica(page).getByRole("tab", { name: "Related" });
  await related.click();
  await expect(related).toHaveAttribute("aria-selected", "true");
  await expect(replica(page).locator("[data-similar]")).toHaveText(/^Similar to /);
  await expect(replica(page).locator("[data-related-list] [data-row]")).toHaveCount(5);
  await replica(page).locator("[data-related-list] [data-row]").first().click();
  await expect(bar(page).locator(".fp-play")).toHaveAttribute("aria-label", "Pause");
});

test("the sidebar opens its pages, and History lists what was played", async ({ page }) => {
  await ready(page);
  await bar(page).locator(".fp-play").click();
  await replica(page).getByRole("button", { name: "Back", exact: true }).click();
  for (const name of ["Library", "Explore", "History"]) {
    await replica(page).getByRole("button", { name, exact: true }).click();
    await expect(replica(page).getByRole("button", { name, exact: true })).toHaveAttribute("aria-current", "page");
    await expect(replica(page).locator(`[data-pane="${name.toLowerCase()}"]`)).toBeVisible();
  }
  await expect(replica(page).locator("[data-history-list] [data-row]").first()).toContainText("California Lullabye");
});

test("the volume slider, its wheel and the mute button", async ({ page }) => {
  await ready(page);
  const volume = bar(page).getByRole("slider", { name: "Volume" });
  await volume.focus();
  await page.keyboard.press("ArrowLeft");
  await expect(volume).toHaveValue("69");
  await bar(page).getByRole("button", { name: "Mute" }).click();
  await expect(bar(page).getByRole("button", { name: "Unmute" })).toBeVisible();
  await expect(volume).toHaveAttribute("aria-valuetext", "Muted");
  const box = (await bar(page).locator("[data-volume-wrap]").boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  await page.mouse.wheel(0, -100);
  await expect(bar(page).getByRole("button", { name: "Mute" })).toBeVisible();
});

test("shuffle and repeat toggle in the accent; repeat cycles off, all, one", async ({ page }) => {
  await ready(page);
  const shuffle = bar(page).getByRole("button", { name: "Shuffle" });
  await shuffle.click();
  await expect(shuffle).toHaveAttribute("aria-pressed", "true");
  const repeat = bar(page).locator('[data-act="repeat"]');
  await repeat.click();
  await expect(repeat).toHaveAttribute("aria-label", "Repeat all");
  await repeat.click();
  await expect(repeat).toHaveAttribute("aria-label", "Repeat one");
  await expect(repeat.locator(".one")).toBeVisible();
});

test("the lyrics toggle glides the cover to its place and fades the lines", async ({ page }) => {
  await ready(page);
  const pill = replica(page).locator(".lyrics-pill");
  await pill.click();
  await expect(replica(page)).toHaveAttribute("data-view", "now");
  await expect(replica(page).locator(".lyrics")).toHaveCSS("opacity", "0");
  await pill.click();
  // The cover is mid-glide right after the click.
  const moving = await replica(page).locator(".np-cover").evaluate((el) => el.getAnimations().length);
  expect(moving).toBeGreaterThan(0);
  await expect(replica(page).locator(".lyrics")).toHaveCSS("opacity", "1");
});

test("the playing row shows the app's playing mark, travelling only while playing", async ({ page }) => {
  await ready(page);
  const mark = replica(page).locator('.queue [data-row][aria-current="true"] .fp-mark');
  await expect(mark).toBeVisible();
  const anim = () => mark.locator(".travel").evaluate((el) => getComputedStyle(el).animationName);
  expect(await anim()).toBe("none");
  await bar(page).locator(".fp-play").click();
  await expect.poll(anim).toBe("fp-mark-travel");
});

test("hovering Not for me for a moment shows what it does", async ({ page }) => {
  await ready(page);
  const tip = bar(page).locator(".nfm .fp-tip");
  await bar(page).getByRole("button", { name: "Not for me" }).hover();
  await expect(tip).toHaveCSS("opacity", "1", { timeout: 2000 });
});

test("the toast stays while the pointer rests on it, whatever the other players' toasts do", async ({ page }) => {
  await page.clock.install();
  await ready(page);
  await bar(page).getByRole("button", { name: "Not for me" }).click();
  const toast = replica(page).locator("[data-toast]");
  await expect(toast).toBeVisible();
  await toast.hover();
  await page.clock.runFor(8000);
  await expect(toast).toBeVisible();
  await page.mouse.move(5, 5);
  await page.clock.runFor(5500);
  await expect(toast).toBeHidden();
});

test("the toast is announced once, from one live region that's always on the page", async ({ page }) => {
  await ready(page);
  const live = page.locator("[data-toast-live]");
  await expect(live).toHaveCount(1);
  await expect(live).toHaveAttribute("role", "status");
  await expect(live).toBeAttached();
  // The visual toasts aren't live regions themselves (one per player would repeat it).
  await expect(page.locator('[data-toast][role="status"]')).toHaveCount(0);
  await bar(page).getByRole("button", { name: "Not for me" }).click();
  await expect(live).toHaveText("Marked as not for me");
});

test("minimising applies to the laptop's window only; the window without a taskbar has no minimise", async ({ page }) => {
  await ready(page);
  await replica(page).getByRole("button", { name: "Minimise featherplay" }).click();
  await expect(replica(page)).toHaveAttribute("data-minimised", "");
  const yours = page.locator("#replica-yours");
  await expect(yours).not.toHaveAttribute("data-minimised", /.*/);
  expect(await yours.evaluate((el) => el.inert)).toBe(false);
  await expect(page.locator("#replica-kept")).not.toHaveAttribute("data-minimised", /.*/);
  await yours.scrollIntoViewIfNeeded();
  await expect(yours.locator('[data-act="minimise"]')).toBeHidden();
});

test("rows built for History say Pause only for the song that's playing", async ({ page }) => {
  await ready(page);
  await bar(page).locator(".fp-play").click();
  await bar(page).getByRole("button", { name: "Next", exact: true }).click();
  await replica(page).getByRole("button", { name: "Back", exact: true }).click();
  await replica(page).getByRole("button", { name: "History", exact: true }).click();
  const rows = replica(page).locator("[data-history-list] [data-row]");
  await expect(rows.nth(0)).toHaveAccessibleName(/^Pause Invisible Light/);
  await expect(rows.nth(1)).toHaveAccessibleName(/^Play California Lullabye/);
});
