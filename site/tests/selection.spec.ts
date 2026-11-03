import { expect, test, type Page } from "@playwright/test";
import { mergeLines } from "../src/scripts/selection";

// Selected text: rounded pieces of a soft accent wash, drawn behind the selected lines
// (src/scripts/selection.ts); native selection stays underneath, so copying works as ever.

const selectLede = (page: Page) =>
  page.evaluate(() => {
    const p = document.querySelector<HTMLElement>("#light .lede")!;
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
    const lede = (await page.locator("#light .lede").boundingBox())!;
    for (const b of await pieces.all()) {
      const r = (await b.boundingBox())!;
      expect(r.x).toBeGreaterThanOrEqual(lede.x - 4);
      expect(r.x + r.width).toBeLessThanOrEqual(lede.x + lede.width + 4);
      expect(r.y).toBeGreaterThanOrEqual(lede.y - 4);
      expect(r.y + r.height).toBeLessThanOrEqual(lede.y + lede.height + 4);
    }
    const bg = await page.evaluate(() => getComputedStyle(document.querySelector("#light .lede")!, "::selection").backgroundColor);
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
  expect(rule).toMatch(/color-mix/);
  expect(await page.locator("html").getAttribute("class")).toContain("sel-drawn");
});

test.describe("reduced motion", () => {
  test.use({ reducedMotion: "reduce" });
  test("the native soft wash stays, under the text's own colour", async ({ page }) => {
    await page.goto("/");
    const sel = await page.evaluate(() => {
      const p = document.querySelector(".hero-line")!;
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
