import { existsSync, readdirSync } from "node:fs";
import { expect, test, type Page } from "@playwright/test";
import { skipTour, wake } from "./helpers";
import { songs } from "../src/data/songs";

// The demo plays real clips under free licences: nothing loads until
// the visitor presses play, the playing song's credit sits by the controls, and the sound
// stops when the demo can't be seen.

/** Records every audio element the page plays, so a test can ask whether it's sounding. */
async function watchAudio(page: Page) {
  await page.addInitScript(() => {
    const w = window as unknown as { __audio: HTMLMediaElement[] };
    w.__audio = [];
    const play = HTMLMediaElement.prototype.play;
    HTMLMediaElement.prototype.play = function (this: HTMLMediaElement) {
      if (!w.__audio.includes(this)) w.__audio.push(this);
      return play.call(this);
    };
  });
}
const sounding = (page: Page) =>
  page.evaluate(() => (window as unknown as { __audio: HTMLMediaElement[] }).__audio.some((a) => !a.paused));

function musicRequests(page: Page) {
  const urls: string[] = [];
  page.on("request", (r) => {
    if (new URL(r.url()).pathname.startsWith("/music/")) urls.push(new URL(r.url()).pathname);
  });
  return urls;
}

const heroPlay = (page: Page) => page.locator("[data-hero] .bar .fp-play").first();
const heroCredit = (page: Page) => page.locator("[data-hero] [data-credit]");

test("nothing under /music/ loads before the first press of play, even scrolling the whole page", async ({ page }) => {
  const music = musicRequests(page);
  await page.goto("/");
  await wake(page);
  await skipTour(page);
  await page.evaluate(async () => {
    for (let y = 0; y < document.body.scrollHeight; y += innerHeight / 2) {
      scrollTo({ top: y, behavior: "instant" });
      await new Promise((r) => setTimeout(r, 30));
    }
  });
  await page.waitForTimeout(300);
  expect(music).toEqual([]);
});

test("pressing play loads the song's clip, and its credit sits by the controls", async ({ page }) => {
  await watchAudio(page);
  const music = musicRequests(page);
  await page.goto("/");
  await wake(page);
  await skipTour(page);
  const first = songs[0];
  // The credit is there for the loaded song from the start, by the laptop's bar.
  await expect(heroCredit(page)).toBeVisible();
  await expect(heroCredit(page)).toContainText(`“${first.title}” by ${first.artist}`);
  await expect(heroCredit(page).getByRole("link", { name: first.credit!.site.label })).toHaveAttribute("href", first.credit!.site.url);
  await expect(heroCredit(page).getByRole("link", { name: first.credit!.licence })).toHaveAttribute("href", first.credit!.licenceUrl);

  await heroPlay(page).click();
  await expect.poll(() => music).toContain(first.clip!.opus);
  await expect.poll(() => sounding(page)).toBe(true);
  // The sound switch is on, and mutes.
  const sound = page.locator("[data-hero] [data-sound]");
  await expect(sound).toBeVisible();
  await expect(sound).toHaveAttribute("aria-pressed", "true");

  // Another song: its clip, its credit.
  await page.locator("[data-hero] [data-replica]").getByRole("button", { name: "Next", exact: true }).click();
  const second = songs[1];
  await expect.poll(() => music).toContain(second.clip!.opus);
  await expect(heroCredit(page)).toContainText(`“${second.title}” by ${second.artist}`);
  // Only the playing song's clip loads, never the others.
  expect(music.every((u) => u === first.clip!.opus || u === second.clip!.opus)).toBe(true);
});

test("the Sound switch mutes and unmutes the clip", async ({ page }) => {
  await watchAudio(page);
  await page.goto("/");
  await wake(page);
  await skipTour(page);
  await heroPlay(page).click();
  const sound = page.locator("[data-hero] [data-sound]");
  // One icon at a time: the speaker while on, the struck speaker while muted.
  await expect(sound.locator("svg:visible")).toHaveCount(1);
  await sound.click();
  await expect(sound).toHaveAttribute("aria-pressed", "false");
  await expect(sound.locator("svg:visible")).toHaveCount(1);
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { __audio: HTMLMediaElement[] }).__audio[0]?.muted))
    .toBe(true);
  await sound.click();
  await expect(sound).toHaveAttribute("aria-pressed", "true");
  await expect
    .poll(() => page.evaluate(() => (window as unknown as { __audio: HTMLMediaElement[] }).__audio[0]?.muted))
    .toBe(false);
});

test("a hidden tab pauses the demo and its sound", async ({ page }) => {
  await watchAudio(page);
  await page.goto("/");
  await wake(page);
  await skipTour(page);
  await heroPlay(page).click();
  await expect.poll(() => sounding(page)).toBe(true);
  await page.evaluate(() => {
    Object.defineProperty(document, "visibilityState", { value: "hidden", configurable: true });
    Object.defineProperty(document, "hidden", { value: true, configurable: true });
    document.dispatchEvent(new Event("visibilitychange"));
  });
  await expect.poll(() => sounding(page)).toBe(false);
  await expect(page.locator("[data-hero] [data-replica]")).toHaveAttribute("data-playing", "false");
});

test("scrolling the demo out of view pauses it", async ({ page }) => {
  await watchAudio(page);
  await page.goto("/");
  await wake(page);
  await skipTour(page);
  await heroPlay(page).click();
  await expect.poll(() => sounding(page)).toBe(true);
  await page.locator("#download").scrollIntoViewIfNeeded();
  await expect.poll(() => sounding(page)).toBe(false);
  await expect(page.locator("[data-hero] [data-replica]")).toHaveAttribute("data-playing", "false");
});

test("the credits page credits all six tracks with their licence links", async ({ page }) => {
  await page.goto("/credits");
  const credits = page.locator("[data-music-credits]");
  for (const s of songs) {
    const quoted = new RegExp(`[“‘]${s.title.replace(/[()]/g, "\\$&")}[”’]`);
    await expect(credits, s.title).toContainText(quoted);
    await expect(credits.locator(`a[href="${s.credit!.licenceUrl}"]`).first()).toBeVisible();
  }
  // CC BY 3.0 is the one seabirds track's: once in its row, once in the artists' wording.
  await expect(credits.locator('a[href="https://creativecommons.org/licenses/by/3.0/"]')).toHaveCount(2);
  await expect(credits).toContainText("Lyric timings are our own.");
});

test("the built site ships only the web clips: no raw downloads, no unused lyrics", () => {
  const dist = new URL("../dist/", import.meta.url);
  expect(existsSync(dist)).toBe(true);
  const all = readdirSync(dist, { recursive: true }).map(String);
  expect(all.filter((f) => /raw|unused|novox|\.mp3$|\.flac$|\.wav$/i.test(f))).toEqual([]);
  expect(readdirSync(new URL("music/", dist)).sort()).toEqual(songs.flatMap((s) => [`${s.id}.m4a`, `${s.id}.webm`]).sort());
});

test("the tour's Start over cues the first song paused, without a blip of sound", async ({ page }) => {
  await watchAudio(page);
  await page.goto("/");
  await wake(page);
  await heroPlay(page).click();
  await expect.poll(() => sounding(page)).toBe(true);
  await heroPlay(page).click(); // pause
  await expect.poll(() => sounding(page)).toBe(false);
  await page.evaluate(() => {
    const w = window as unknown as { __plays: number };
    w.__plays = 0;
    const play = HTMLMediaElement.prototype.play;
    HTMLMediaElement.prototype.play = function (this: HTMLMediaElement) {
      w.__plays++;
      return play.call(this);
    };
    document.querySelector<HTMLButtonElement>("[data-tour-restart]")!.click();
  });
  const r = page.locator("[data-hero] [data-replica]");
  await expect(r).toHaveAttribute("data-song", songs[0].id);
  await expect(r).toHaveAttribute("data-playing", "false");
  await page.waitForTimeout(200);
  expect(await page.evaluate(() => (window as unknown as { __plays: number }).__plays)).toBe(0);
});

test("while it plays, the clip is seeked only when the position jumps, not on the clock's ticks", async ({ page }) => {
  await watchAudio(page);
  await page.goto("/");
  await wake(page);
  await skipTour(page);
  await heroPlay(page).click();
  await expect.poll(() => sounding(page)).toBe(true);
  // Once the clip is under way (the first placement, applied as its data arrives, is done).
  await expect
    .poll(() => page.evaluate(() => {
      const a = (window as unknown as { __audio: HTMLMediaElement[] }).__audio[0];
      return a.readyState >= 3 && !a.seeking && a.currentTime > 0;
    }))
    .toBe(true);
  await page.evaluate(() => {
    const w = window as unknown as { __audio: HTMLMediaElement[]; __seeks: number };
    w.__seeks = 0;
    w.__audio[0].addEventListener("seeking", () => w.__seeks++);
  });
  await page.waitForTimeout(1500);
  expect(await page.evaluate(() => (window as unknown as { __seeks: number }).__seeks)).toBe(0);
  // A click on a line is a jump: one seek.
  await page.locator("[data-hero] [data-lyrics] .line").nth(5).evaluate((b: HTMLElement) => b.click());
  await expect.poll(() => page.evaluate(() => (window as unknown as { __seeks: number }).__seeks)).toBe(1);
});

test("when the browser refuses to start the sound, the demo goes on muted and doesn't retry each frame", async ({ page }) => {
  await page.addInitScript(() => {
    const w = window as unknown as { __plays: number };
    w.__plays = 0;
    HTMLMediaElement.prototype.play = function () {
      w.__plays++;
      return Promise.reject(new DOMException("blocked", "NotAllowedError"));
    };
  });
  await page.goto("/");
  await wake(page);
  await skipTour(page);
  await heroPlay(page).click();
  const sound = page.locator("[data-hero] [data-sound]");
  await expect(sound).toHaveAttribute("aria-pressed", "false");
  await expect(page.locator("[data-hero] [data-replica]")).toHaveAttribute("data-playing", "true");
  await page.waitForTimeout(800);
  expect(await page.evaluate(() => (window as unknown as { __plays: number }).__plays)).toBe(1);
});

test("every player carries the same line, sound then credit, attached under it", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await wake(page);
  const lines = page.locator("[data-now-playing]");
  await expect(lines).toHaveCount(3);
  for (const line of await lines.all()) {
    await expect(line.locator("[data-sound]")).toHaveCount(1);
    await expect(line.locator("[data-credit]")).toHaveCount(1);
  }
  // In the hero: centred under the laptop, just below it.
  const laptop = (await page.locator("[data-hero] [data-laptop]").boundingBox())!;
  const hero = (await page.locator("[data-hero] [data-now-playing]").boundingBox())!;
  expect(hero.y - (laptop.y + laptop.height)).toBeGreaterThanOrEqual(0);
  expect(hero.y - (laptop.y + laptop.height)).toBeLessThan(24);
  expect(Math.abs(hero.x + hero.width / 2 - (laptop.x + laptop.width / 2))).toBeLessThan(2);
});

for (const theme of ["dark", "light"]) {
  test(`every credit line's separators show, with the same gap either side, at any width (${theme})`, async ({ page }) => {
    await page.addInitScript((t) => localStorage.setItem("fp-theme", t), theme);
    for (const width of [390, 1560]) {
      await page.setViewportSize({ width, height: 900 });
      await page.goto("/");
      for (const credit of await page.locator("[data-credit]").all()) {
        const dots = await credit.evaluate((el) =>
          [...el.querySelectorAll<HTMLElement>(".dot")].map((dot) => {
            const box = dot.getBoundingClientRect();
            const before = dot.previousElementSibling!.getBoundingClientRect();
            // The next visible part (past the comma for screen readers and the <wbr>).
            let next = dot.nextElementSibling!;
            while (next.tagName === "WBR" || next.classList.contains("visually-hidden")) next = next.nextElementSibling!;
            const after = next.getBoundingClientRect();
            const style = getComputedStyle(dot);
            return {
              text: dot.textContent,
              opacity: Number(style.opacity),
              colour: style.color === getComputedStyle(el).color,
              before: box.left - before.right,
              // Only when the next part is on the same line (a line may end with a separator).
              after: Math.abs(after.top - box.top) < 4 ? after.left - box.right : null,
            };
          }),
        );
        expect(dots.length).toBeGreaterThanOrEqual(2);
        // At least one gap after a dot is measured on every line that doesn't wrap at it.
        expect(dots.some((dot) => dot.after !== null)).toBe(true);
        for (const dot of dots) {
          expect(dot.text).toBe("·");
          expect(dot.colour).toBe(true);
          expect(dot.opacity).toBeGreaterThan(0.6);
          expect(dot.before).toBeGreaterThan(3);
          if (dot.after !== null) expect(Math.abs(dot.after - dot.before)).toBeLessThan(1);
        }
      }
    }
  });
}
