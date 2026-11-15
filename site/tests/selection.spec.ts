import { expect, test, type Page } from "@playwright/test";
import { mergeLines } from "../src/scripts/selection";

// Selected text: rounded pieces of a soft wash in the playing song's colour, drawn behind the
// selected lines (src/scripts/selection.ts); native selection stays underneath, so copying
// works as ever.

const selectLede = (page: Page) =>
  page.evaluate(() => {
    const p = document.querySelector<HTMLElement>("#light .t-lede")!;
    p.scrollIntoView({ block: "center", behavior: "instant" });
    const r = document.createRange();
    r.selectNodeContents(p);
    const sel = document.getSelection()!;
    sel.removeAllRanges();
    sel.addRange(r);
    // The lines the paragraph's text sits on.
    const lines = mergeLinesInPage([...r.getClientRects()]);
    return { text: p.textContent!.trim(), lines };
    function mergeLinesInPage(rs: DOMRect[]) {
      const tops = new Set(rs.filter((x) => x.width > 1).map((x) => Math.round((x.top + x.bottom) / 2 / 4)));
      return tops.size;
    }
  });

for (const theme of ["dark", "light"] as const) {
  test(`selecting a paragraph draws a rounded piece per line, and the native highlight steps aside (${theme})`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: theme });
    await page.goto("/");
    const { lines } = await selectLede(page);
    const pieces = page.locator(".sel-layer > span");
    await expect(pieces).toHaveCount(lines);
    const radius = await pieces.first().evaluate((el) => parseFloat(getComputedStyle(el).borderTopLeftRadius));
    // A wash you can see: the accent at a low alpha, not transparent.
    const wash = await pieces.first().evaluate((el) => getComputedStyle(el).backgroundColor);
    expect(wash).not.toMatch(/rgba\(0, 0, 0, 0\)|transparent/);
    expect(radius).toBeGreaterThanOrEqual(4);
    // Each piece covers its line of text.
    const lede = (await page.locator("#light .t-lede").boundingBox())!;
    for (const b of await pieces.all()) {
      const r = (await b.boundingBox())!;
      expect(r.x).toBeGreaterThanOrEqual(lede.x - 4);
      expect(r.x + r.width).toBeLessThanOrEqual(lede.x + lede.width + 4);
      expect(r.y).toBeGreaterThanOrEqual(lede.y - 4);
      expect(r.y + r.height).toBeLessThanOrEqual(lede.y + lede.height + 4);
    }
    const bg = await page.evaluate(() => getComputedStyle(document.querySelector("#light .t-lede")!, "::selection").backgroundColor);
    expect(bg).toMatch(/rgba\(0, 0, 0, 0\)|transparent/);
    // Collapsed: the pieces fade and go.
    await page.evaluate(() => document.getSelection()!.removeAllRanges());
    await expect(pieces).toHaveCount(0);
  });
}

test("copying the selection still yields the text", async ({ page, context }) => {
  await context.grantPermissions(["clipboard-read", "clipboard-write"]);
  await page.goto("/");
  const { text } = await selectLede(page);
  await page.keyboard.press("Control+C");
  const copied = await page.evaluate(() => navigator.clipboard.readText());
  expect(copied.replace(/\s+/g, " ").trim()).toBe(text.replace(/\s+/g, " "));
});

test("text fields keep the native highlight", async ({ page }) => {
  await page.goto("/");
  // Chrome doesn't compute ::selection for form controls, so read the rule itself.
  const rule = await page.evaluate(() => {
    for (const sheet of document.styleSheets) {
      for (const r of sheet.cssRules) {
        if (r instanceof CSSStyleRule && /sel-drawn :is\(input, ?textarea\)(:where\(.*\))?::selection/.test(r.selectorText)) {
          return r.style.cssText;
        }
      }
    }
    return null;
  });
  expect(rule).toMatch(/--song/);
  expect(await page.locator("html").getAttribute("class")).toContain("sel-drawn");
});

test.describe("reduced motion", () => {
  test.use({ reducedMotion: "reduce" });
  test("the native soft wash stays, under the text's own colour", async ({ page }) => {
    await page.goto("/");
    const sel = await page.evaluate(() => {
      const p = document.querySelector("[data-stage] .col .t-lede")!;
      const s = getComputedStyle(p, "::selection");
      return { bg: s.backgroundColor, color: s.color, text: getComputedStyle(p).color, layer: !!document.querySelector(".sel-layer") };
    });
    expect(sel.layer).toBe(false);
    const alpha = sel.bg.match(/\/\s*([\d.]+)\)|rgba\([^)]*,\s*([\d.]+)\)/);
    expect(alpha, sel.bg).not.toBeNull();
    expect(Number(alpha![1] ?? alpha![2])).toBeLessThan(0.5);
    expect(Number(alpha![1] ?? alpha![2])).toBeGreaterThan(0.1);
    expect(sel.color).toBe(sel.text);
  });
  test("the native wash is the playing song's colour too", async ({ page }) => {
    await page.goto("/");
    const wash = () => page.evaluate(() => getComputedStyle(document.querySelector("[data-stage] .col .t-lede")!, "::selection").backgroundColor);
    const before = await wash();
    await page.evaluate(() => document.querySelector<HTMLElement>('#fly-song [data-song="watching-seabirds-soar"]')!.click());
    await expect.poll(wash).not.toBe(before);
  });
});

/** Selects the text of the first element matching `sel` (scrolled into view first). */
const selectIn = (page: Page, sel: string) =>
  page.evaluate((sel) => {
    const el = document.querySelector<HTMLElement>(sel)!;
    el.scrollIntoView({ block: "center", behavior: "instant" });
    const r = document.createRange();
    r.selectNodeContents(el);
    const s = document.getSelection()!;
    s.removeAllRanges();
    s.addRange(r);
  }, sel);

test("the pieces wear the playing song's colour, and ease to the next song's on the colour clock", async ({ page }) => {
  await page.route("**/music/**", (r) => r.abort());
  await page.goto("/");
  await selectLede(page);
  const layer = page.locator(".sel-layer").filter({ has: page.locator("span") });
  await expect(page.locator(".sel-layer > span").first()).toBeVisible();
  const song = () => layer.evaluate((el) => getComputedStyle(el).getPropertyValue("--song").trim());
  // California Lullabye's colour, #E8894A.
  expect(await song()).toBe("rgb(232, 137, 74)");
  await page.evaluate(() => document.querySelector<HTMLElement>('#fly-song [data-song="origami"]')!.click());
  // On its way, by the one colour clock (900 ms), not at once.
  const fading = await layer.evaluate((el) => el.getAnimations().some((a) => a instanceof CSSTransition && a.transitionProperty === "--song"));
  expect(fading).toBe(true);
  await expect.poll(song, { timeout: 3000 }).toBe("rgb(212, 162, 58)");
  // The pieces themselves are a wash of it.
  const bg = await page.locator(".sel-layer > span").first().evaluate((el) => getComputedStyle(el).backgroundColor);
  expect(bg).not.toMatch(/rgba\(0, 0, 0, 0\)|transparent/);
});

test("light ink is screened and dark ink multiplied, so the text keeps its colour on any surface", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "light" });
  await page.goto("/");
  // Dark ink on the paper desk.
  await selectLede(page);
  await expect(page.locator(".sel-layer[data-blend='multiply'] > span").first()).toBeVisible();
  await expect(page.locator(".sel-layer[data-blend='screen'] > span")).toHaveCount(0);
  // White lines on the lyric card's deep colour, in the light theme too.
  await selectIn(page, "[data-piece='lyrics'] [data-lc-lines]");
  await expect(page.locator(".sel-layer[data-blend='screen'] > span").first()).toBeVisible();
  await expect(page.locator(".sel-layer[data-blend='multiply'] > span")).toHaveCount(0);
  for (const blend of ["multiply", "screen"]) {
    expect(await page.locator(`.sel-layer[data-blend='${blend}']`).evaluate((el) => getComputedStyle(el).mixBlendMode)).toBe(blend);
  }
});

test("over the bar and in what floats over the page (flyouts, the menu, the dialog) the native wash stays, and nothing is drawn", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("[data-window]")).toHaveClass(/\bplaced\b/);
  // The drawn pieces lie under the bar, never over its glass.
  const z = await page.evaluate(() => [Number(getComputedStyle(document.querySelector(".sel-layer")!).zIndex), Number(getComputedStyle(document.querySelector("[data-site-bar]")!).zIndex)]);
  expect(z[0]).toBeLessThan(z[1]);
  await page.evaluate(() => document.getElementById("fly-song")!.showPopover());
  await selectIn(page, "#fly-song .fq-title");
  await page.waitForTimeout(200);
  await expect(page.locator(".sel-layer > span")).toHaveCount(0);
  const bg = await page.evaluate(() => getComputedStyle(document.querySelector("#fly-song .fq-title")!, "::selection").backgroundColor);
  expect(bg).not.toMatch(/rgba\(0, 0, 0, 0\)|transparent/);
});

test("line boxes merge into one piece per run of text on a line", () => {
  const merged = mergeLines([
    { left: 0, top: 0, right: 50, bottom: 20 },
    { left: 52, top: 1, right: 120, bottom: 21 },
    { left: 0, top: 24, right: 80, bottom: 44 },
    { left: 300, top: 24, right: 340, bottom: 44 },
  ]);
  expect(merged).toEqual([
    { left: 0, top: 0, right: 120, bottom: 21 },
    { left: 0, top: 24, right: 80, bottom: 44 },
    { left: 300, top: 24, right: 340, bottom: 44 },
  ]);
});

test("a piece is never drawn where its text is clipped away (the lyric card's lines out of its window)", async ({ page }) => {
  await page.goto("/");
  await selectIn(page, "[data-piece='lyrics'] [data-lc-lines]");
  await expect(page.locator(".sel-layer > span").first()).toBeVisible();
  const view = (await page.locator("[data-piece='lyrics'] [data-lc-view]").boundingBox())!;
  for (const p of await page.locator(".sel-layer > span").all()) {
    const r = (await p.boundingBox())!;
    expect(r.y).toBeGreaterThanOrEqual(view.y - 3);
    expect(r.y + r.height).toBeLessThanOrEqual(view.y + view.height + 3);
  }
});
