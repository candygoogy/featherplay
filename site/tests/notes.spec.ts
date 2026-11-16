import { expect, test, type Page } from "@playwright/test";
import { WIDTHS } from "./check";
import { notes } from "../src/data/notes";

// The handwritten notes (Note.astro, src/scripts/notes.ts): decoration hidden from assistive
// technology, written in once, and hidden rather than drawn over anything at any width.

/** Each visible note's words that touch another's words, a picture, a control or a live piece,
 *  or run off the screen (read independently of the page's own rule). */
const clashes = (page: Page) =>
  page.evaluate(() => {
    const out: string[] = [];
    const vw = document.documentElement.clientWidth;
    const near = (a: DOMRect, b: DOMRect) => a.left < b.right && a.right > b.left && a.top < b.bottom && a.bottom > b.top;
    for (const n of document.querySelectorAll<HTMLElement>(".hand[data-note]:not([data-note-manual])")) {
      if (n.classList.contains("note-off") || getComputedStyle(n).display === "none") continue;
      const w = n.querySelector("span")!.getBoundingClientRect();
      if (!w.width) continue;
      if (w.left < 0 || w.right > vw) out.push(`${vw}: ${n.dataset.note} off the screen`);
      const scope = n.closest("section, footer")!;
      for (const el of scope.querySelectorAll("h2, h3, p:not(.hand), li, a, button, td, th, img, [data-piece], .explorer, .win-body")) {
        if (el.contains(n) || el.closest(".visually-hidden") || el.parentElement?.closest("[data-piece]")) continue;
        let boxes: DOMRect[] = [el.getBoundingClientRect()];
        if (el.matches("h2, h3, p, li, a, button, td, th")) {
          const r = document.createRange();
          r.selectNodeContents(el);
          boxes = [...r.getClientRects()];
        }
        if (boxes.some((b) => b.width > 1 && near(w, b))) {
          out.push(`${vw}: ${n.dataset.note} on ${el.tagName}.${String(el.className).split(" ")[0]}`);
          break;
        }
      }
    }
    return out;
  });

test("no note is ever drawn over anything, nor off the screen, at any width", async ({ page }) => {
  test.setTimeout(120_000);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await page.evaluate(() => document.fonts.ready);
  const found: string[] = [];
  let shown = 0;
  for (const w of WIDTHS) {
    await page.setViewportSize({ width: w, height: 900 });
    await page.waitForTimeout(320);
    found.push(...(await clashes(page)));
    shown += await page.locator(".hand[data-note]:not([data-note-manual]):not(.note-off)").evaluateAll((ns) => ns.filter((n) => getComputedStyle(n).display !== "none").length);
  }
  expect(found).toEqual([]);
  // The rule hides notes where they don't fit; it doesn't hide them all.
  expect(shown).toBeGreaterThan(WIDTHS.length * 2);
});

test("the notes are decoration: hidden from assistive technology, in the hand's own subset face", async ({ page }) => {
  await page.goto("/");
  const hands = page.locator(".hand[data-note]");
  expect(await hands.count()).toBeGreaterThanOrEqual(8);
  for (const h of await hands.all()) await expect(h).toHaveAttribute("aria-hidden", "true");
  await page.locator("[data-note='line']").scrollIntoViewIfNeeded();
  await expect.poll(() => page.evaluate(() => document.fonts.check('30px "Reenie Beanie"', "click a line"))).toBe(true);
  const src = await page.evaluate(() => [...document.styleSheets].flatMap((s) => [...s.cssRules]).find((r) => r instanceof CSSFontFaceRule && r.style.getPropertyValue("font-family").includes("Reenie"))?.cssText ?? "");
  expect(src).toContain("/fonts/reenie-beanie-subset.woff2");
  // Each note's words are the data's.
  for (const h of await hands.all()) {
    const id = (await h.getAttribute("data-note")) as keyof typeof notes;
    await expect(h.locator("span")).toHaveText(notes[id]);
  }
});

test("a note writes itself in once, when wholly in view, and stays written", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  const note = page.locator("[data-note='onefile']");
  await expect(note).not.toHaveClass(/written/);
  // Unwritten, its words are wiped away (the pen hasn't been there).
  expect(await note.locator("span").evaluate((s) => getComputedStyle(s).clipPath)).toMatch(/inset/);
  await note.evaluate((n) => n.scrollIntoView({ block: "center" }));
  await expect(note).toHaveClass(/written/);
  await page.evaluate(() => scrollTo(0, 0));
  await page.waitForTimeout(200);
  await expect(note).toHaveClass(/written/);
  // The pen mark under featherplay's memory is drawn the same way.
  const mark = page.locator("#light .pen-mark");
  await mark.scrollIntoViewIfNeeded();
  await expect(mark).toHaveClass(/written/);
});

test.describe("reduced motion", () => {
  test.use({ reducedMotion: "reduce" });
  test("the notes are simply there", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator(".hand[data-note]:not([data-note-manual])").first()).toHaveClass(/written/);
    expect(await page.locator("html").getAttribute("class")).not.toMatch(/\bpen\b/);
    const clip = await page.locator("[data-note='onefile'] span").evaluate((s) => getComputedStyle(s).clipPath);
    expect(clip).toBe("none");
  });
});

test("on a phone a few notes show, and none collides", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/");
  await page.evaluate(() => document.fonts.ready);
  await page.waitForTimeout(300);
  expect(await clashes(page)).toEqual([]);
  const shown = await page.locator(".hand[data-note]:not([data-note-manual]):not(.note-off)").evaluateAll((ns) => ns.filter((n) => getComputedStyle(n).display !== "none").length);
  expect(shown).toBeGreaterThanOrEqual(2);
  expect(shown).toBeLessThanOrEqual(4);
});
