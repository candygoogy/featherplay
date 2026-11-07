import { expect, test, type Page } from "@playwright/test";
import { skipTour, wake } from "./helpers";

const replica = (page: Page) => page.locator("[data-replica]").first();

async function rootGlow(page: Page) {
  return page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue("--glow").trim());
}

test("the replica is a labelled demo with a text summary", async ({ page }) => {
  await page.goto("/");
  const r = replica(page);
  await expect(r).toHaveAttribute("role", "group");
  await expect(r).toHaveAttribute("aria-label", "featherplay demo");
  const describedBy = await r.getAttribute("aria-describedby");
  expect(describedBy).toBeTruthy();
  await expect(page.locator(`#${describedBy}`)).toContainText(/demo/i);
});

test("clicking a Home card plays it and the glow takes its colour", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  await skipTour(page);
  const r = replica(page);
  // The hero starts on Now playing: Back goes Home.
  await r.getByRole("button", { name: "Back", exact: true }).click();
  await expect(r.getByRole("button", { name: "Home", exact: true })).toHaveAttribute("aria-current", "page");
  await r.locator(".cards").getByRole("button", { name: "Play Origami" }).click();
  await expect(r.locator("[data-bar-title]")).toHaveText("Origami");
  await expect(r.getByRole("button", { name: "Pause" }).first()).toBeVisible();
  await expect.poll(() => rootGlow(page)).toMatch(/^(#d4a23a|rgb\(212, 162, 58\))$/i);
});

test("the lyrics view highlights the current line and clicking a line seeks to it", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  await skipTour(page);
  const r = replica(page);
  const lyrics = r.locator("[data-lyrics]");
  await expect(lyrics).toBeVisible();
  const current = lyrics.locator('[aria-current="true"]');
  await expect(current).toHaveCount(1);
  const target = lyrics.getByRole("button", { name: "With a California lullabye" });
  await target.click();
  await expect(target).toHaveAttribute("aria-current", "true");
  await expect(r.locator("[data-bar-time]")).toHaveText("0:25");
});

test('a song without lyrics keeps the cover view, and the pill says "No lyrics"', async ({ page }) => {
  await page.goto("/");
  await wake(page);
  await skipTour(page);
  const r = replica(page);
  await r.getByRole("button", { name: /^Play In This Moment/ }).first().click();
  await expect(r.locator("[data-bar-title]")).toHaveText("In This Moment");
  await expect(r.locator(".lyrics-pill")).toHaveAccessibleName("No lyrics");
  await expect(r.locator("[data-lyrics]")).toBeHidden();
  await expect(r.locator(".stage-panel")).toHaveCSS("opacity", "0");
  await expect(r.locator("[data-np-artist]")).toHaveText("Scott Buckley");
  // A single of its own, on no album: no album line, as the app leaves it out.
  await expect(r.locator("[data-np-album]")).toBeHidden();
  // Never pressed, and no note until the visitor presses it.
  const mic = r.locator(".bar [data-lyrics-toggle]");
  await expect(r.locator(".lyrics-pill")).toHaveAttribute("aria-pressed", "false");
  await expect(mic).toHaveAttribute("aria-pressed", "false");
  await expect(r.locator("[data-lyrics-tip]")).toHaveText("No lyrics");
  const note = r.locator("[data-no-lyrics]");
  await expect(note).toBeHidden();
  await r.locator(".lyrics-pill").click();
  await expect(note).toHaveText("No lyrics for this song");
  await expect(r.locator("[data-lyrics]")).toBeHidden();
  // A song with lyrics (Next, from the list's last song, goes back to its first): the note
  // goes, and the lines show (the view stayed on lyrics).
  await r.locator('.bar [data-act="next"]').click();
  await expect(r.locator("[data-bar-title]")).toHaveText("California Lullabye");
  await expect(note).toBeHidden();
  await expect(r.locator("[data-lyrics]")).toBeVisible();
  // Its album, with the year, under the artist ("album · year").
  await expect(r.locator("[data-np-album]")).toHaveText("The Beautiful Machine · 2014");
});

test("on Now playing, Up next hides and shows the side panel, and Playing from opens it", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  await skipTour(page);
  const r = replica(page);
  const upNext = r.locator('.bar [data-act="queue"]');
  const panel = r.locator("[data-queue]");
  await expect(panel).toBeVisible();
  await upNext.click();
  await expect(panel).toBeHidden();
  await expect(upNext).toHaveAttribute("aria-pressed", "false");
  // Still on Now playing, the stage across the window.
  await expect(r).toHaveAttribute("data-view", "lyrics");
  await upNext.click();
  await expect(panel).toBeVisible();
  await expect(upNext).toHaveAttribute("aria-pressed", "true");
  // Playing from opens the panel on Up next, from Related and with the panel hidden.
  await r.getByRole("tab", { name: "Related" }).click();
  await upNext.click();
  await expect(panel).toBeHidden();
  await r.getByRole("button", { name: "Playing from Made for the demo" }).click();
  await expect(panel).toBeVisible();
  await expect(r.getByRole("tab", { name: "Up next" })).toHaveAttribute("aria-selected", "true");
});

test("the play button pauses and plays, and time moves only while playing", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  await skipTour(page);
  const r = replica(page);
  const time = r.locator("[data-bar-time]");
  const before = await time.textContent();
  await r.getByRole("button", { name: "Play", exact: true }).first().click();
  await expect(time).not.toHaveText(before!, { timeout: 3000 });
  await r.getByRole("button", { name: "Pause", exact: true }).first().click();
  const paused = await time.textContent();
  await page.waitForTimeout(1500);
  await expect(time).toHaveText(paused!);
});

test("Tab reaches every control in the replica and leaves it", async ({ page }) => {
  await page.goto("/");
  const r = replica(page);
  const controls = await r.evaluate((root) =>
    [...root.querySelectorAll<HTMLElement>("button, a[href], input, [tabindex]")]
      .filter((el) => el.tabIndex >= 0 && el.offsetParent !== null && !el.closest("[inert]"))
      .map((el) => el.dataset.testKey ?? (el.getAttribute("aria-label") || el.textContent?.trim() || el.tagName)),
  );
  expect(controls.length).toBeGreaterThan(10);

  await page.locator("[data-before-replica]").focus();
  const seen: string[] = [];
  let left = false;
  for (let i = 0; i < 200; i++) {
    await page.keyboard.press("Tab");
    const where = await page.evaluate(() => {
      const el = document.activeElement as HTMLElement | null;
      const inside = !!el?.closest("[data-replica]");
      return {
        inside,
        key: el ? (el.dataset.testKey ?? (el.getAttribute("aria-label") || el.textContent?.trim() || el.tagName)) : "",
        // The ring is on the control, or on the field or rail that wraps it.
        ring: (() => {
          for (let n: HTMLElement | null = el, k = 0; n && k < 3; n = n.parentElement, k++) {
            const cs = getComputedStyle(n);
            if (cs.outlineStyle !== "none" && n.matches(":focus-visible, :focus-within")) return true;
            if (n.matches(":focus-within") && cs.boxShadow.includes("inset") && n.classList.contains("field")) return true;
          }
          return false;
        })(),
      };
    });
    if (where.inside) {
      expect(where.ring, `focus is visible on ${where.key}`).toBe(true);
      seen.push(where.key);
    } else if (seen.length) {
      left = true;
      break;
    }
  }
  expect(left, "focus leaves the replica").toBe(true);
  expect(new Set(seen)).toEqual(new Set(controls));
});

test("the wheel over the laptop scrolls the page", async ({ page }) => {
  await page.goto("/");
  const box = (await page.locator("[data-laptop]").first().boundingBox())!;
  await page.mouse.move(box.x + box.width / 2, box.y + box.height * 0.4);
  const before = await page.evaluate(() => scrollY);
  await page.mouse.wheel(0, 500);
  await expect.poll(() => page.evaluate(() => scrollY)).toBeGreaterThan(before + 100);
});

test.describe("without JavaScript", () => {
  test.use({ javaScriptEnabled: false });
  test("the replica shows a static featherplay frame", async ({ page }) => {
    await page.goto("/");
    const r = replica(page);
    await expect(r).toBeVisible();
    await expect(r.locator("[data-bar-title]")).toHaveText("California Lullabye");
    await expect(r.locator("[data-lyrics]")).toBeVisible();
  });
});

/** The list's shift, and where the app's rule puts it: the current line's centre a third of
 *  the way down the panel, within [0, the list's range] (pages/lyrics.slint). */
const placement = (page: Page) =>
  replica(page)
    .locator("[data-lines]")
    .evaluate((list) => {
      const view = list.parentElement!.clientHeight;
      const li = list.querySelector('[aria-current="true"]')!.parentElement as HTMLElement;
      const top = li.offsetParent === list ? li.offsetTop : li.offsetTop - list.offsetTop;
      const rule = Math.min(Math.max(0, top + li.offsetHeight / 2 - view / 3), Math.max(0, list.offsetHeight - view));
      return { shift: -new DOMMatrixReadOnly(getComputedStyle(list).transform).m42, rule };
    });

test("on load, and on a new song, the lines are placed at once, not glided in from the top", async ({ page }) => {
  await page.goto("/");
  const lines = replica(page).locator("[data-lines]");
  // The first placement is instant: no transition on the list, already at the current line.
  expect(await lines.evaluate((el) => el.style.transition)).toBe("none");
  const p = await placement(page);
  expect(p.shift).toBeCloseTo(p.rule, 0);
});

test("the lines' place rendered without script is where the script puts them: no jump on load", async ({ page, browser }) => {
  await page.goto("/");
  const p = await placement(page);
  const still = await browser.newContext({ javaScriptEnabled: false });
  const bare = await still.newPage();
  await bare.goto(page.url());
  const ssr = await bare
    .locator("[data-replica] [data-lines]")
    .first()
    .evaluate((el) => -new DOMMatrixReadOnly(getComputedStyle(el).transform).m42);
  await still.close();
  expect(Math.abs(ssr - p.rule)).toBeLessThan(2);
});

test.describe("the lines start at the top, as in the app", () => {
  test.use({ reducedMotion: "reduce", viewport: { width: 1440, height: 900 } });

  /** Measures in the replica's own pixels (the hero draws it scaled on the laptop). */
  const measure = (page: Page) =>
    replica(page).evaluate((r) => {
      const scale = r.getBoundingClientRect().height / r.offsetHeight;
      const cover = r.querySelector(".np-cover")!.getBoundingClientRect();
      // The line's own box (its hover tile), as the app's LyricLine rectangle.
      const line = r.querySelector<HTMLElement>("[data-lines] .line")!.getBoundingClientRect();
      return { lineTop: (line.top - cover.top) / scale };
    });

  test("at a song's start the first line's top is level with the cover's top edge", async ({ page }) => {
    await page.goto("/");
    await wake(page);
    await skipTour(page);
    await replica(page).locator("[data-lines] .line").first().evaluate((b: HTMLElement) => b.click());
    await page.waitForTimeout(100);
    const m = await measure(page);
    expect(Math.abs(m.lineTop)).toBeLessThan(2);
  });

  test("mid-song the line playing is held a third of the way down; at the end the list stops at its end", async ({ page }) => {
    await page.goto("/");
    await wake(page);
    await skipTour(page);
    const lines = replica(page).locator("[data-lines] .line");
    await lines.nth(7).evaluate((b: HTMLElement) => b.click());
    await page.waitForTimeout(100);
    const mid = await replica(page)
      .locator("[data-lines]")
      .evaluate((list) => {
        const view = list.parentElement!.clientHeight;
        const li = list.querySelector('[aria-current="true"]')!.parentElement as HTMLElement;
        const shift = -new DOMMatrixReadOnly(getComputedStyle(list).transform).m42;
        const top = li.offsetParent === list ? li.offsetTop : li.offsetTop - list.offsetTop;
        return { shift, centre: top + li.offsetHeight / 2 - shift, view };
      });
    expect(mid.shift).toBeGreaterThan(0);
    expect(Math.abs(mid.centre - mid.view / 3)).toBeLessThan(2);

    await lines.last().evaluate((b: HTMLElement) => b.click());
    await page.waitForTimeout(100);
    const end = await replica(page)
      .locator("[data-lines]")
      .evaluate((list) => ({
        shift: -new DOMMatrixReadOnly(getComputedStyle(list).transform).m42,
        range: Math.max(0, list.offsetHeight - list.parentElement!.clientHeight),
      }));
    // The last line would sit higher than a third; the list stops at its end instead.
    expect(end.range).toBeGreaterThan(0);
    expect(Math.abs(end.shift - end.range)).toBeLessThan(1);
  });

  test("the lyrics view has the app's metrics beside the side panel: a line every 45.1 px with no verse gaps, the 208 px cover, the title in its width", async ({ page }) => {
    await page.goto("/");
    await wake(page);
    await skipTour(page);
    await replica(page).locator("[data-lines] .line").first().evaluate((b: HTMLElement) => b.click());
    await page.waitForTimeout(100);
    const m = await replica(page).evaluate((r) => {
      const scale = r.getBoundingClientRect().height / r.offsetHeight;
      const stage = r.querySelector(".stage")!.getBoundingClientRect();
      const rel = (el: Element) => {
        const b = el.getBoundingClientRect();
        return { x: (b.x - stage.x) / scale, y: (b.y - stage.y) / scale, w: b.width / scale, h: b.height / scale };
      };
      const tops = [...r.querySelectorAll("[data-lines] .line")].map((l) => rel(l).y);
      return {
        steps: tops.slice(1).map((t, i) => t - tops[i]),
        cover: rel(r.querySelector(".np-cover")!),
        title: rel(r.querySelector(".np-title")!),
        text: rel(r.querySelector("[data-lines] .line")!).x + 12,
      };
    });
    for (const s of m.steps) expect(s).toBeCloseTo(45.12, 0);
    for (const [k, v] of Object.entries({ x: 40, y: 96, w: 208, h: 208 })) expect(m.cover[k as keyof typeof m.cover]).toBeCloseTo(v, 1);
    // s5 under the cover, wrapping in its width.
    expect(m.title.y).toBeCloseTo(96 + 208 + 24, 0);
    expect(m.title.w).toBeLessThanOrEqual(208.5);
    expect(m.text).toBeCloseTo(312, 0);
  });

  test("with the side panel hidden the lyrics view is the app's at 1440 px: the 280 px cover, 26 px lines every 49.4 px", async ({ page }) => {
    await page.goto("/");
    await wake(page);
    await skipTour(page);
    await replica(page).locator('.bar [data-act="queue"]').click();
    await page.waitForTimeout(100);
    const m = await replica(page).evaluate((r) => {
      const scale = r.getBoundingClientRect().height / r.offsetHeight;
      const stage = r.querySelector(".stage")!.getBoundingClientRect();
      const rel = (el: Element) => {
        const b = el.getBoundingClientRect();
        return { x: (b.x - stage.x) / scale, y: (b.y - stage.y) / scale, w: b.width / scale, h: b.height / scale };
      };
      const tops = [...r.querySelectorAll("[data-lines] .line")].map((l) => rel(l).y);
      return {
        steps: tops.slice(1).map((t, i) => t - tops[i]),
        cover: rel(r.querySelector(".np-cover")!),
        text: rel(r.querySelector("[data-lines] .line")!).x + 12,
      };
    });
    for (const s of m.steps) expect(s).toBeCloseTo(49.35, 0);
    for (const [k, v] of Object.entries({ x: 188, y: 96, w: 280, h: 280 })) expect(m.cover[k as keyof typeof m.cover]).toBeCloseTo(v, 1);
    expect(m.text).toBeCloseTo(532, 0);
  });

  test("the lines fade over the app's 24 px at the panel's edges", async ({ page }) => {
    await page.goto("/");
    const mask = await replica(page).locator("[data-lyrics]").evaluate((el) => getComputedStyle(el).maskImage);
    expect(mask).toMatch(/\b24px\b.*calc\(100% - 24px\)/);
  });
});

// The replica doesn't take the app's keys (its Ctrl+L is named in the tooltip only), so Space
// is named the same way: in the play button's tooltip, as the app's shortcut.
test("the play button's tooltip names its shortcut, Space, as the app's", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  await skipTour(page);
  const r = replica(page);
  const play = r.locator(".bar [data-act='toggle']");
  const tip = r.locator(".bar .fp-tipped:has([data-act='toggle']) > .fp-tip");
  await expect(tip).toContainText("Space");
  await expect(tip).toContainText("Play");
  await play.hover();
  await expect(tip).toHaveCSS("opacity", "1");
  await play.click();
  await expect(tip).toContainText("Pause");
  // Only a name: the shortcut itself stays the page's (Space scrolls it).
  await expect(tip).toHaveAttribute("aria-hidden", "true");
});

test("focusing a line out of view brings it into view by moving the lines, not by scrolling the box", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await wake(page);
  await skipTour(page);
  const r = replica(page);
  const panel = r.locator("[data-lyrics]");
  // At the app's pitch the whole song fits the panel from its start, so the song at its end:
  // the list scrolled to its end, the first lines above the top edge.
  await panel.locator(".line").last().evaluate((b: HTMLElement) => b.click());
  await page.waitForTimeout(600);
  const first = panel.locator(".line").first();
  const [p0, l0] = await Promise.all([panel.boundingBox(), first.boundingBox()]);
  expect(l0!.y).toBeLessThan(p0!.y);
  await first.focus();
  await page.waitForTimeout(600);
  expect(await panel.evaluate((el) => el.scrollTop)).toBe(0);
  const [p, l] = await Promise.all([panel.boundingBox(), first.boundingBox()]);
  expect(l!.y).toBeGreaterThanOrEqual(p!.y);
  expect(l!.y + l!.height).toBeLessThanOrEqual(p!.y + p!.height);
});

for (const scheme of ["dark", "light"] as const) {
  test(`the cover view's wash from the top resolves in the ${scheme} theme (no light-dark() percentage)`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: scheme });
    await page.goto("/");
    const wash = replica(page).locator(".stage-wash").first();
    const image = await wash.evaluate((el) => getComputedStyle(el).backgroundImage);
    expect(image).toContain("linear-gradient");
    // The app's alphas: 0.36 in light, 0.55 in dark (WASH_ALPHA_LIGHT, _DARK).
    expect(image).toContain(scheme === "dark" ? "0.55" : "0.36");
  });
}
