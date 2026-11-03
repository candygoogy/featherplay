import { expect, test, type Page } from "@playwright/test";
import { demoStart, songs } from "../src/data/songs";
import { currentLineIndex } from "../src/demo/player";

const show = (page: Page) => page.locator("#lyrics");
const lines = (page: Page) => show(page).locator("[data-showcase-lines]");

async function open(page: Page) {
  await page.goto("/");
  await show(page).scrollIntoViewIfNeeded();
}

const shift = (page: Page) =>
  lines(page).evaluate((el) => new DOMMatrixReadOnly(getComputedStyle(el).transform).m42);

test("the showcase shows the same song as the demo, with its lines", async ({ page }) => {
  await open(page);
  await expect(show(page).locator("[data-showcase-title]")).toHaveText("California Lullabye");
  await expect(lines(page).getByRole("button", { name: "So let me shake away the cloudy skies" })).toHaveAttribute("aria-current", "true");
});

test("clicking a line seeks, here and in the demo", async ({ page }) => {
  await open(page);
  const line = lines(page).getByRole("button", { name: "With a California lullabye" });
  await line.click();
  await expect(line).toHaveAttribute("aria-current", "true");
  await expect(show(page).locator("[data-showcase-time]")).toHaveText("0:25");
  await expect(page.locator("[data-replica] [data-bar-time]").first()).toHaveText("0:25");
});

test("lines glide to the current one while playing", async ({ page }) => {
  await open(page);
  // Just before a line, then play: the next line takes over and the list moves up.
  await lines(page).getByRole("button", { name: "Without a map, you chose your heart" }).evaluate((b: HTMLElement) => b.click());
  await page.waitForTimeout(500);
  const before = await shift(page);
  await show(page).getByRole("button", { name: "Play", exact: true }).click();
  await expect(lines(page).getByRole("button", { name: "Now we are one, but you're still far from home" })).toHaveAttribute("aria-current", "true", {
    timeout: 6000,
  });
  await expect.poll(() => shift(page)).toBeLessThan(before - 10);
  await expect(lines(page)).toHaveCSS("transition-duration", /0\.4s|400ms/);
});

test("the switch off hides lyrics here and in the replica", async ({ page }) => {
  await open(page);
  const toggle = show(page).getByRole("switch", { name: "Show lyrics" });
  await expect(toggle).toHaveAttribute("aria-checked", "true");
  await toggle.click();
  await expect(toggle).toHaveAttribute("aria-checked", "false");
  await expect(lines(page)).toBeHidden();
  await expect(show(page).getByText("Lyrics are off")).toBeVisible();
  const replica = page.locator("[data-replica]").first();
  await expect(replica).toHaveAttribute("data-lyrics-on", "false");
  await expect(replica.locator("[data-lyrics]")).toBeHidden();
  await toggle.click();
  await expect(lines(page)).toBeVisible();
});

test("turning lyrics off and on moves nothing: the panel keeps its size and place, the caption its line", async ({ page }) => {
  await open(page);
  const toggle = show(page).getByRole("switch", { name: "Show lyrics" });
  const box = async () => {
    const r = (sel: string) => show(page).locator(sel).boundingBox();
    return { panel: await r("[data-showcase-panel]"), source: await r("[data-showcase-source]"), side: await r(".side") };
  };
  const before = await box();
  await toggle.click();
  await page.waitForTimeout(400);
  const off = await box();
  expect(off).toEqual(before);
  // The off state sits inside the panel.
  const note = (await show(page).getByText("Lyrics are off").boundingBox())!;
  expect(note.y).toBeGreaterThan(before.panel!.y);
  expect(note.y + note.height).toBeLessThan(before.panel!.y + before.panel!.height);
  await toggle.click();
  await page.waitForTimeout(400);
  expect(await box()).toEqual(before);
});

test("with an instrumental playing elsewhere, the showcase keeps showing a song with lyrics, and Play plays it", async ({ page }) => {
  await open(page);
  await page.evaluate(() => document.querySelector<HTMLElement>('[data-replica] [data-play="in-this-moment"]')?.click());
  await expect(show(page).getByText("No lyrics for this song")).toBeHidden();
  await expect(show(page).locator("[data-showcase-title]")).toHaveText("California Lullabye");
  await expect(lines(page)).toBeVisible();
  // Its credit is the song it shows.
  await expect(show(page).locator("[data-credit]")).toContainText("joshwoodward.com");
  await show(page).getByRole("button", { name: "Play California Lullabye" }).click();
  await expect(page.locator("[data-replica]").first()).toHaveAttribute("data-song", "california-lullabye");
  await expect(show(page).getByRole("button", { name: "Pause", exact: true })).toBeVisible();
});

test("the showcase offers only the three songs with lyrics, and picking one plays it", async ({ page }) => {
  await open(page);
  const picks = show(page).locator("[data-showcase-pick]");
  await expect(picks).toHaveCount(3);
  await picks.nth(1).click();
  await expect(show(page).locator("[data-showcase-title]")).toHaveText("Invisible Light");
  await expect(picks.nth(1)).toHaveAttribute("aria-pressed", "true");
  await expect(page.locator("[data-replica]").first()).toHaveAttribute("data-song", "invisible-light");
});

/** WCAG contrast of each line state's colour (blended over the panel) against the panel. */
async function inks(page: Page) {
  return show(page).evaluate((root) => {
    const rgba = (c: string): number[] => {
      const n = c.match(/[\d.]+/g)!.map(Number);
      // color(srgb r g b / a) is in 0..1; rgb()/rgba() in 0..255.
      if (c.startsWith("color(")) return [n[0] * 255, n[1] * 255, n[2] * 255, n[3] ?? 1];
      return [n[0], n[1], n[2], n[3] ?? 1];
    };
    const lum = ([r, g, b]: number[]) =>
      [r, g, b]
        .map((v) => v / 255)
        .map((v) => (v <= 0.04045 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4))
        .reduce((s, v, i) => s + v * [0.2126, 0.7152, 0.0722][i], 0);
    const ground = rgba(getComputedStyle(root.querySelector("[data-showcase-panel]")!).backgroundColor);
    const contrast = (state: string) => {
      const b = root.querySelector<HTMLElement>(`.line[data-state="${state}"]`);
      if (!b) return null;
      const [r, g, bl, a] = rgba(getComputedStyle(b).color);
      const over = [r, g, bl].map((v, i) => v * a + ground[i] * (1 - a));
      const [x, y] = [lum(over), lum(ground)];
      return (Math.max(x, y) + 0.05) / (Math.min(x, y) + 0.05);
    };
    return { past: contrast("past"), current: contrast("current"), next: contrast("next") };
  });
}

test("the line being sung stands out: white, the lines to come clearly dimmer, the sung ones dimmest, also while paused", async ({ page }) => {
  await open(page);
  // Invisible Light, paused on "Cuz we are like invisible light in the dark" (the user's screenshot).
  await show(page).locator('[data-showcase-pick="invisible-light"]').click();
  const line = lines(page).getByRole("button", { name: "Cuz we are like invisible light in the dark" });
  await line.evaluate((b: HTMLElement) => b.click()); // it may sit below the panel's edge
  await show(page).getByRole("button", { name: "Pause", exact: true }).click();
  await expect(show(page).getByRole("button", { name: "Play", exact: true })).toBeVisible();
  await expect(line).toHaveAttribute("aria-current", "true");
  await page.waitForTimeout(700); // the colours ease
  await expect(line).toHaveCSS("color", "rgb(255, 255, 255)");
  for (const id of ["california-lullabye", "invisible-light", "she-dreams-in-blue"]) {
    if (id !== "invisible-light") {
      await show(page).locator(`[data-showcase-pick="${id}"]`).click();
      // Paused on its fourth line: sung lines above, lines to come below.
      await lines(page).locator(".line").nth(3).evaluate((b: HTMLElement) => b.click());
      await show(page).getByRole("button", { name: "Pause", exact: true }).click();
      await expect(lines(page).locator(".line").nth(3)).toHaveAttribute("aria-current", "true");
      await page.waitForTimeout(700);
    }
    const c = await inks(page);
    // As the app's lyric_panel: playing white (7+), to come 4.5:1 or a little more, sung 3:1 or a little more.
    expect(c.current!, id).toBeGreaterThanOrEqual(7);
    if (c.next !== null) {
      expect(c.next, id).toBeGreaterThanOrEqual(4.5);
      expect(c.current! / c.next, id).toBeGreaterThanOrEqual(1.5);
    }
    if (c.past !== null) {
      expect(c.past, id).toBeGreaterThanOrEqual(3);
      expect(c.next === null || c.past < c.next - 0.5, id).toBe(true);
    }
  }
});

test("the panel is one solid colour: the lines fade at its edges, its background doesn't", async ({ page }) => {
  await open(page);
  const panel = show(page).locator("[data-showcase-panel]");
  await expect(panel).toHaveCSS("mask-image", "none");
  const viewport = show(page).locator("[data-showcase-viewport]");
  expect(await viewport.evaluate((el) => getComputedStyle(el).maskImage)).toContain("gradient");
});

// The app's rule (pages/lyrics.slint, SyncedLines): the lines start at the top of the panel,
// the line playing is held a third of the way down once it gets there, and the list never
// scrolls above its first line or past its end.
test.describe("the lines start at the top, as in the app", () => {
  test.use({ reducedMotion: "reduce" }); // each placement is final at once
  const panel = (page: Page) => show(page).locator("[data-showcase-panel]");

  /** The panel's box, a line's box (by index; -1 the last) and the list's shift. */
  const geometry = (page: Page, index: number) =>
    panel(page).evaluate((p, i) => {
      const list = p.querySelector<HTMLElement>("[data-showcase-lines]")!;
      const items = [...list.querySelectorAll<HTMLElement>(".line")];
      const line = items.at(i)!.getBoundingClientRect();
      const box = p.getBoundingClientRect();
      return {
        top: line.top - box.top,
        centre: line.top + line.height / 2 - box.top,
        bottom: list.getBoundingClientRect().bottom - box.top,
        height: box.height,
      };
    }, index);
  const seekTo = (page: Page, index: number) =>
    lines(page).locator(".line").nth(index).evaluate((b: HTMLElement) => b.click());

  for (const width of [390, 1440, 3840]) {
    test(`at a song's start the first line sits at the panel's top (${width} px)`, async ({ page }) => {
      await page.setViewportSize({ width, height: width === 390 ? 844 : width === 3840 ? 2160 : 900 });
      await open(page);
      await seekTo(page, 0);
      const g = await geometry(page, 0);
      // Its words (under the line's 6 px padding) start just clear of the 24 px edge fade:
      // well up, not a third of the way down.
      expect(g.top + 6).toBeGreaterThanOrEqual(23.5);
      expect(g.top + 6).toBeLessThanOrEqual(64);
    });
  }

  test("once the song is under way, the line playing is held a third of the way down", async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await open(page);
    await seekTo(page, 10);
    const g = await geometry(page, 10);
    expect(g.centre).toBeGreaterThan(g.height / 3 - 4);
    expect(g.centre).toBeLessThan(g.height / 3 + 4);
  });

  test("at the song's end the lines stop at their end, not scrolled past it", async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await open(page);
    await seekTo(page, -1);
    const g = await geometry(page, -1);
    // The list ends where the panel does (its padding below is half the panel): the last
    // line rests between a third and the middle, never up at the third with nothing below.
    expect(g.bottom).toBeGreaterThan(g.height - 2);
    expect(g.bottom).toBeLessThan(g.height + 2);
    expect(g.centre).toBeGreaterThan(g.height / 3);
    expect(g.centre).toBeLessThan(g.height / 2 + 4);
  });

  test("the lines fade only at the panel's very edges, as the app's 24 px", async ({ page }) => {
    await open(page);
    const mask = await show(page).locator("[data-showcase-viewport]").evaluate((el) => getComputedStyle(el).maskImage);
    expect(mask).toMatch(/\b24px\b.*calc\(100% - 24px\)/);
  });

  test("lyrics off, the note sits where the lines start", async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await open(page);
    await seekTo(page, 0);
    const first = await geometry(page, 0);
    await show(page).getByRole("switch", { name: "Show lyrics" }).click();
    const note = show(page).getByText("Lyrics are off");
    await expect(note).toBeVisible();
    const [n, p] = await Promise.all([note.boundingBox(), panel(page).boundingBox()]);
    expect(Math.abs(n!.y - p!.y - (first.top + 6))).toBeLessThan(8);
  });
});

test.describe("reduced motion", () => {
  test.use({ reducedMotion: "reduce" });
  test("lines change without gliding", async ({ page }) => {
    await open(page);
    await expect(lines(page)).toHaveCSS("transition-duration", "0s");
    const before = await shift(page);
    await lines(page).getByRole("button", { name: "With a California lullabye" }).click();
    // No travel: the very next frame already has the new place.
    expect(await shift(page)).toBeLessThan(before - 10);
  });
});

test.describe("scrolling the lyrics by hand", () => {
  const panel = (page: Page) => show(page).locator("[data-showcase-panel]");

  async function pointAt(page: Page) {
    await page.setViewportSize({ width: 1440, height: 900 });
    await open(page);
    // Room to scroll on past the section, whatever follows it.
    await page.evaluate(() => (document.body.style.paddingBottom = "2000px"));
    await panel(page).evaluate((el) => el.scrollIntoView({ block: "center", behavior: "instant" }));
    const box = (await panel(page).boundingBox())!;
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  }

  test("the wheel over the panel moves the lines, not the page, then they ease back to the current line", async ({ page }) => {
    await page.clock.install();
    await pointAt(page);
    await page.clock.runFor(1000);
    await page.waitForTimeout(700); // the first glide (a CSS transition, on real time) settles
    const resting = await shift(page);
    const pageY = await page.evaluate(() => scrollY);

    // Down, then back up part of the way (the lines may rest at their top: nothing above).
    await page.mouse.wheel(0, 240);
    await expect.poll(() => shift(page)).toBeLessThan(resting - 180);
    expect(await page.evaluate(() => scrollY)).toBe(pageY);
    await page.mouse.wheel(0, -120);
    await expect.poll(() => shift(page)).toBeGreaterThan(resting - 150);

    // A few seconds without input: back to the line being sung.
    await page.clock.runFor(5200);
    await expect(lines(page)).toHaveCSS("transition-duration", /0\.4s|400ms/);
    await expect.poll(() => shift(page), { timeout: 3000 }).toBeCloseTo(resting, 0);
  });

  test("at either end the wheel scrolls the page on", async ({ page }) => {
    await pointAt(page);
    // Past the last line: the lines stop, the page moves.
    for (let i = 0; i < 12; i++) await page.mouse.wheel(0, 200);
    await page.waitForTimeout(800); // the lines' glide to their end settles
    const bottom = await shift(page);
    const pageY = await page.evaluate(() => scrollY);
    await page.waitForTimeout(300); // a new wheel gesture
    await page.mouse.wheel(0, 300);
    await expect.poll(() => page.evaluate(() => scrollY)).toBeGreaterThan(pageY + 100);
    expect(await shift(page)).toBeCloseTo(bottom, 0);

    // And above the first line, upwards.
    await pointAt(page);
    for (let i = 0; i < 12; i++) await page.mouse.wheel(0, -200);
    await expect.poll(() => shift(page)).toBeCloseTo(0, 0);
    const y2 = await page.evaluate(() => scrollY);
    await page.waitForTimeout(300);
    await page.mouse.wheel(0, -300);
    await expect.poll(() => page.evaluate(() => scrollY)).toBeLessThan(y2 - 100);
  });

  test("a click on a line follows the song again at once", async ({ page }) => {
    await pointAt(page);
    await page.mouse.wheel(0, 400);
    const line = lines(page).getByRole("button", { name: "And I will sing your cares away" }).last();
    await line.click();
    await expect(line).toHaveAttribute("aria-current", "true");
    // The app's rule: the line's centre a third of the way down, within the list's range.
    const target = await line.evaluate((b) => {
      const li = b.parentElement!;
      const list = li.parentElement!;
      const view = list.parentElement!.clientHeight;
      const top = li.offsetParent === list ? li.offsetTop : li.offsetTop - list.offsetTop;
      return -Math.min(Math.max(0, top + li.offsetHeight / 2 - view / 3), Math.max(0, list.offsetHeight - view));
    });
    await expect.poll(() => shift(page)).toBeCloseTo(target, 0);
  });
});

test("the showcase names whose lyrics they are, under which licence", async ({ page }) => {
  await open(page);
  const source = show(page).locator("[data-showcase-source]");
  await expect(source).toHaveText("Lyrics: Josh Woodward, CC BY 4.0");
  await page.evaluate(() => document.querySelector<HTMLElement>('[data-replica] [data-play="origami"]')?.click());
  await expect(source).toHaveText("Lyrics: Josh Woodward, CC BY 4.0");
});

// The lines follow what the visitor hears. A clip that starts late (a slow line, a refused
// start, a stall) must not leave the lyrics running ahead of the singing.
test.describe("lyrics follow the clip", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("with a clip playing, the highlighted line follows the lyrics' timings at the clip's own time", async ({ page }) => {
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
    const panel = page.locator("#lyrics");
    await panel.scrollIntoViewIfNeeded();
    await panel.locator("[data-showcase-play]").click();

    let checked = 0;
    for (let i = 0; i < 20; i++) {
      await page.waitForTimeout(450);
      const { audioMs, shown, heroShown } = await page.evaluate(() => {
        const a = (window as unknown as { __audio: HTMLMediaElement[] }).__audio[0];
        const text = (sel: string) => document.querySelector(`${sel} .line[data-state="current"]`)?.textContent?.trim() ?? null;
        return {
          audioMs: a && !a.paused && a.readyState >= 3 ? a.currentTime * 1000 : null,
          shown: text("[data-showcase-lines]"),
          heroShown: text("[data-hero] [data-lines]"),
        };
      });
      if (audioMs === null) continue;
      // Away from a line change (the page updates on the next frame), the line must be the
      // one the clip is at.
      const near = song.lyrics.some((l) => Math.abs(l.atMs - audioMs) < 250);
      if (near) continue;
      const expected = song.lyrics[currentLineIndex(song, audioMs)]?.line ?? null;
      expect(shown, `at ${Math.round(audioMs)} ms of the clip`).toBe(expected);
      expect(heroShown, `the hero's lines at ${Math.round(audioMs)} ms`).toBe(expected);
      checked++;
    }
    expect(checked).toBeGreaterThan(4);
  });
});

// A mouse wheel's notch over a lyrics panel glides the lines, as the page's own wheel does
// (src/scripts/smooth.ts): each notch eases over a few hundred milliseconds and notches in a
// row add up. A trackpad or a finger stays 1:1, reduced motion keeps the instant step.
test.describe("the wheel glides the lines", () => {
  test.use({ viewport: { width: 1440, height: 900 } });
  const panel = (page: Page) => show(page).locator("[data-showcase-panel]");

  /** Records `sel`'s translateY every frame until `stop()`. */
  async function record(page: Page, sel: string) {
    await page.evaluate((s) => {
      const w = window as any;
      const el = document.querySelector<HTMLElement>(s)!;
      w.__shifts = [] as number[];
      const tick = () => {
        w.__shifts.push(new DOMMatrixReadOnly(getComputedStyle(el).transform).m42);
        w.__raf = requestAnimationFrame(tick);
      };
      w.__raf = requestAnimationFrame(tick);
    }, sel);
  }
  const recorded = (page: Page): Promise<number[]> =>
    page.evaluate(() => {
      const w = window as any;
      cancelAnimationFrame(w.__raf);
      return w.__shifts;
    });
  const between = (ys: number[]) => {
    const [a, b] = [ys[0], ys.at(-1)!];
    const [lo, hi] = a < b ? [a, b] : [b, a];
    return new Set(ys.filter((y) => y > lo + 0.5 && y < hi - 0.5).map((y) => Math.round(y))).size;
  };

  async function pointAt(page: Page) {
    await open(page);
    await page.evaluate(() => (document.body.style.paddingBottom = "2000px"));
    await panel(page).evaluate((el) => el.scrollIntoView({ block: "center", behavior: "instant" }));
    // At the song's start, with room below for the lines to move up.
    await lines(page).locator(".line").first().evaluate((b: HTMLElement) => b.click());
    await page.waitForTimeout(700);
    const box = (await panel(page).boundingBox())!;
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
  }

  test("a notch over the panel moves the lines over several frames, not in one jump", async ({ page }) => {
    await pointAt(page);
    const resting = await shift(page);
    const pageY = await page.evaluate(() => scrollY);
    await record(page, "[data-showcase-lines]");
    await page.mouse.wheel(0, 120);
    // Right after the notch, the lines have only started on their way.
    expect(await shift(page)).toBeGreaterThan(resting - 100);
    await page.waitForTimeout(800);
    const ys = await recorded(page);
    expect(ys.at(-1)!).toBeCloseTo(resting - 120, 0);
    expect(between(ys)).toBeGreaterThanOrEqual(5);
    for (let i = 1; i < ys.length; i++) expect(ys[i]).toBeLessThanOrEqual(ys[i - 1] + 0.5);
    expect(await page.evaluate(() => scrollY)).toBe(pageY);
  });

  test("notches in a row add up", async ({ page }) => {
    await pointAt(page);
    const resting = await shift(page);
    for (let i = 0; i < 3; i++) await page.mouse.wheel(0, 100);
    await expect.poll(() => shift(page)).toBeCloseTo(resting - 300, 0);
  });

  test("a trackpad's fine deltas move the lines 1:1, at once", async ({ page }) => {
    await pointAt(page);
    const resting = await shift(page);
    await page.mouse.wheel(0, 7.5);
    expect(await shift(page)).toBeCloseTo(resting - 7.5, 1);
    await page.mouse.wheel(0, 7.5);
    expect(await shift(page)).toBeCloseTo(resting - 15, 1);
  });

  test("the lyrics in the Make it yours window glide too", async ({ page }) => {
    await page.goto("/");
    const p = page.locator("#replica-yours [data-lyrics]");
    await p.evaluate((el) => el.scrollIntoView({ block: "center", behavior: "instant" }));
    await page.locator("#replica-yours .line").first().evaluate((b: HTMLElement) => b.click());
    await page.waitForTimeout(700);
    const box = (await p.boundingBox())!;
    await page.mouse.move(box.x + box.width / 2, box.y + box.height / 2);
    await record(page, "#replica-yours [data-lines]");
    await page.mouse.wheel(0, 120);
    await page.waitForTimeout(800);
    const ys = await recorded(page);
    expect(ys.at(-1)!).toBeLessThan(ys[0] - 40);
    expect(between(ys)).toBeGreaterThanOrEqual(5);
  });

  test("the hover highlight doesn't hop from line to line while the lines glide under the pointer", async ({ page }) => {
    await pointAt(page);
    await page.waitForTimeout(300); // the hovered line's highlight is in
    await page.evaluate(() => {
      const w = window as any;
      const els = [...document.querySelectorAll<HTMLElement>("[data-showcase-lines] .line")];
      w.__hl = [] as number[];
      const tick = () => {
        // The strongest highlight on any line (its background's alpha).
        w.__hl.push(Math.max(0, ...els.map((b) => Number(getComputedStyle(b).backgroundColor.match(/[\d.]+/g)?.[3] ?? 0))));
        w.__raf = requestAnimationFrame(tick);
      };
      w.__raf = requestAnimationFrame(tick);
    });
    for (let i = 0; i < 3; i++) await page.mouse.wheel(0, 100);
    // A hand on the mouse is never quite still: each tiny move makes the browser look again at
    // what's under the pointer, so a line gliding past would light up.
    const box = (await panel(page).boundingBox())!;
    for (let i = 0; i < 16; i++) {
      await page.mouse.move(box.x + box.width / 2 + (i % 2), box.y + box.height / 2);
      await page.waitForTimeout(40);
    }
    await page.waitForTimeout(400);
    const hl: number[] = await page.evaluate(() => {
      const w = window as any;
      cancelAnimationFrame(w.__raf);
      return w.__hl;
    });
    // It may fade out while the lines move and come back once they rest: one rise at most.
    let rises = 0;
    for (let i = 1; i < hl.length; i++) if (hl[i] > hl[i - 1] + 0.005 && !(i > 1 && hl[i - 1] > hl[i - 2] + 0.005)) rises++;
    expect(rises, hl.map((a) => a.toFixed(2)).join(" ")).toBeLessThanOrEqual(1);
  });

  test.describe("on a touch screen", () => {
    test.use({ hasTouch: true });
    test("a finger moves the lines 1:1, at once", async ({ page }) => {
      await pointAt(page);
      const resting = await shift(page);
      const box = (await panel(page).boundingBox())!;
      const x = box.x + box.width / 2;
      const y = box.y + box.height / 2;
      const cdp = await page.context().newCDPSession(page);
      const touch = (type: string, ty?: number) =>
        cdp.send("Input.dispatchTouchEvent", { type, touchPoints: ty === undefined ? [] : [{ x, y: ty }] });
      await touch("touchStart", y);
      await touch("touchMove", y - 30);
      await touch("touchMove", y - 60);
      expect(await shift(page)).toBeCloseTo(resting - 60, 0);
      await touch("touchEnd");
    });
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
