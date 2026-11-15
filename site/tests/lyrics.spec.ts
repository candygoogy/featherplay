import { expect, test, type Page } from "@playwright/test";
import { demoStart, songs } from "../src/data/songs";
import { currentLineIndex } from "../src/demo/player";

// Lyrics, up close (Lyrics.astro): the lyric card large, on the page's one player. Its lines
// follow the song (src/demo/follow.ts), a click on one seeks there and plays, the wheel or a
// finger moves them for a while, and its licence credit sits under them.
test.use({ viewport: { width: 1440, height: 900 } });

const sec = (page: Page) => page.locator("#lyrics");
const card = (page: Page) => sec(page).locator("[data-piece='lyrics']");
const view = (page: Page) => card(page).locator("[data-lc-view]");
const lines = (page: Page) => card(page).locator("[data-lc-lines]");
const shift = (page: Page) => lines(page).evaluate((el) => new DOMMatrixReadOnly(getComputedStyle(el).transform).m42);
const current = (page: Page) => lines(page).locator('.lc-line[data-state="current"]');

async function open(page: Page) {
  await page.route("**/music/**", (r) => r.abort());
  await page.goto("/");
  await sec(page).scrollIntoViewIfNeeded();
  await page.waitForTimeout(300);
}

test("the card shows the song playing on the page, its line marked", async ({ page }) => {
  await open(page);
  const song = songs.find((s) => s.id === demoStart.songId)!;
  await expect(sec(page).locator("[data-piece='now'] [data-bind='title']")).toHaveText(song.title);
  await expect(current(page)).toHaveText(song.lyrics[currentLineIndex(song, demoStart.positionMs)].line);
  await expect(current(page)).toHaveAttribute("aria-current", "true");
});

test("a click on a line seeks there and plays, here and in the window", async ({ page }) => {
  await open(page);
  const line = lines(page).getByRole("button", { name: "With a California lullabye" }).first();
  await line.click();
  await expect(page.locator("html")).toHaveAttribute("data-playing", "true");
  await expect(line).toHaveAttribute("aria-current", "true");
  await expect(page.locator('[data-window] .line[data-state="current"]').first()).toHaveText("With a California lullabye");
});

test("the lines follow the song: the line sung a third of the way down", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await open(page);
  await lines(page).getByRole("button", { name: "And when you're frozen with desire" }).first().click();
  const target = await current(page).evaluate((b) => {
    const li = b.parentElement!;
    const list = li.parentElement!;
    const h = list.parentElement!.clientHeight;
    return -Math.min(Math.max(0, li.offsetTop + li.offsetHeight / 2 - h / 3), Math.max(0, list.offsetHeight - h));
  });
  await expect.poll(() => shift(page)).toBeCloseTo(target, 0);
});

test("with reduced motion the lines change place without gliding", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await open(page);
  expect(await lines(page).evaluate((el) => getComputedStyle(el).transitionDuration)).toMatch(/^0s/);
});

test("its credit names whose lyrics they are and sits under the lines, never over them", async ({ page }) => {
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 844 });
    await open(page);
    const source = card(page).locator("[data-lc-source]");
    await expect(source).toHaveText("Lyrics: Josh Woodward, CC BY 4.0");
    const [v, s] = [(await view(page).boundingBox())!, (await source.boundingBox())!];
    expect(v.y + v.height, `${width}`).toBeLessThanOrEqual(s.y + 0.5);
  }
});

test("a song without lyrics says so, and offers the ones that have them", async ({ page }) => {
  await open(page);
  await page.evaluate(() => document.querySelector<HTMLElement>("#fly-song [data-song='origami']")!.click());
  await expect(card(page)).toHaveAttribute("data-has-lyrics", "false");
  await expect(card(page)).toContainText("No lyrics for this song");
  const offer = sec(page).locator(".no-lyr");
  await expect(offer).toBeVisible();
  await offer.getByRole("button", { name: "Invisible Light" }).click();
  await expect(card(page)).toHaveAttribute("data-has-lyrics", "true");
  await expect(offer).toBeHidden();
  await expect(sec(page).locator("[data-piece='now'] [data-bind='title']")).toHaveText("Invisible Light");
});

test.describe("moving the lines by hand", () => {
  async function pointAt(page: Page) {
    await open(page);
    await page.evaluate(() => (document.body.style.paddingBottom = "2000px"));
    await view(page).evaluate((el) => el.scrollIntoView({ block: "center", behavior: "instant" }));
    await lines(page).locator(".lc-line").first().evaluate((b: HTMLElement) => b.click());
    await page.waitForTimeout(700);
    const box = (await view(page).boundingBox())!;
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await page.waitForTimeout(400);
  }

  test("the wheel over the card moves the lines, not the page, then they come back to the line sung", async ({ page }) => {
    await pointAt(page);
    const resting = await shift(page);
    const pageY = await page.evaluate(() => scrollY);
    await page.mouse.wheel(0, 240);
    await expect.poll(() => shift(page)).toBeLessThan(resting - 180);
    expect(await page.evaluate(() => scrollY)).toBe(pageY);
    await expect.poll(() => shift(page), { timeout: 9000 }).toBeCloseTo(resting, 0);
  });

  test("a notch glides over several frames; a trackpad's fine deltas move 1:1", async ({ page }) => {
    await pointAt(page);
    const resting = await shift(page);
    await page.mouse.wheel(0, 120);
    expect(await shift(page)).toBeGreaterThan(resting - 100);
    await expect.poll(() => shift(page)).toBeCloseTo(resting - 120, 0);
    await page.waitForTimeout(300);
    const at = await shift(page);
    await page.mouse.wheel(0, 7.5);
    expect(await shift(page)).toBeCloseTo(at - 7.5, 1);
  });

  test.describe("with reduced motion", () => {
    test.use({ reducedMotion: "reduce" });
    test("a notch steps the lines at once", async ({ page }) => {
      await pointAt(page);
      const resting = await shift(page);
      await page.mouse.wheel(0, 120);
      expect(await shift(page)).toBeCloseTo(resting - 120, 0);
    });
  });
});

test("the lyrics in the Make it yours window glide too", async ({ page }) => {
  await page.goto("/");
  const p = page.locator("#replica-yours [data-lyrics]");
  await p.evaluate((el) => el.scrollIntoView({ block: "center", behavior: "instant" }));
  await page.locator("#replica-yours .line").first().evaluate((b: HTMLElement) => b.click());
  await page.waitForTimeout(700);
  const box = (await p.boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  // A moment for the browser to know what is under the pointer after the jump there.
  await page.waitForTimeout(400);
  const before = await page.locator("#replica-yours [data-lines]").evaluate((el) => new DOMMatrixReadOnly(getComputedStyle(el).transform).m42);
  await page.mouse.wheel(0, 120);
  await expect
    .poll(() => page.locator("#replica-yours [data-lines]").evaluate((el) => new DOMMatrixReadOnly(getComputedStyle(el).transform).m42))
    .toBeLessThan(before - 40);
});

test("with a clip playing, the card's line follows the clip's own time", async ({ page }) => {
  await page.addInitScript(() => {
    const w = window as unknown as { __audio: HTMLMediaElement[] };
    w.__audio = [];
    const play = HTMLMediaElement.prototype.play;
    HTMLMediaElement.prototype.play = function (this: HTMLMediaElement) {
      if (!w.__audio.includes(this)) w.__audio.push(this);
      return play.call(this);
    };
  });
  // The clip arrives a second and a half after play is pressed.
  await page.route("**/music/**", async (route) => {
    await new Promise((r) => setTimeout(r, 1500));
    await route.continue();
  });
  await page.goto("/");
  const song = songs.find((s) => s.id === demoStart.songId)!;
  await sec(page).scrollIntoViewIfNeeded();
  await sec(page).locator(".round-play").click();
  let checked = 0;
  for (let i = 0; i < 20; i++) {
    await page.waitForTimeout(450);
    const { audioMs, shown } = await page.evaluate(() => {
      const a = (window as unknown as { __audio: HTMLMediaElement[] }).__audio[0];
      return {
        audioMs: a && !a.paused && a.readyState >= 3 ? a.currentTime * 1000 : null,
        shown: document.querySelector('#lyrics .lc-line[data-state="current"]')?.textContent?.trim() ?? null,
      };
    });
    if (audioMs === null) continue;
    if (song.lyrics.some((l) => Math.abs(l.atMs - audioMs) < 250)) continue;
    expect(shown, `at ${Math.round(audioMs)} ms of the clip`).toBe(song.lyrics[currentLineIndex(song, audioMs)]?.line ?? null);
    checked++;
  }
  expect(checked).toBeGreaterThan(4);
});
