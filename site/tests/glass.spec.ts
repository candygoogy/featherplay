import { expect, test, type Page } from "@playwright/test";
import { textContrast } from "./contrast";

// The panes over the desk's frosted glass: the docked bar painted (only the still glass passes
// under it), the floating panes really blurred while they show, and every text on them, and the
// first screen's words on the glass itself, at AA over what is really behind them, in both
// themes, for warm and cool songs.

const SONGS = ["california-lullabye", "invisible-light", "origami", "watching-seabirds-soar"];
const pick = async (page: Page, id: string) => {
  await page.evaluate((id) => document.querySelector<HTMLElement>(`#fly-song [data-song="${id}"]`)!.click(), id);
  await page.evaluate(() => document.querySelector<HTMLElement>("[data-window] .bar .fp-play")?.click());
  await page.waitForTimeout(1300);
};
const start = async (page: Page, theme: "dark" | "light") => {
  await page.emulateMedia({ colorScheme: theme });
  await page.route("**/music/**", (r) => r.abort());
  await page.goto("/");
  await expect(page.locator("[data-first]")).toHaveAttribute("data-shown", "");
  await page.waitForTimeout(1500);
};
const low = (rs: { text: string; ratio: number }[], label: string) => rs.filter((r) => r.ratio < 4.5).map((r) => `${label}: "${r.text}" ${r.ratio}`);

test("the docked bar is painted frost, the floating panes real glass only while they show", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await start(page, "dark");
  const dock = await page.locator("[data-site-bar] .dock").evaluate((d) => {
    const cs = getComputedStyle(d);
    const grain = getComputedStyle(d, "::before").backgroundImage;
    return { filter: cs.backdropFilter, bg: cs.backgroundImage + " " + cs.backgroundColor, shadow: cs.boxShadow, grain };
  });
  expect(dock.filter).toBe("none");
  expect(dock.grain).toContain("grain-a.png");
  // Translucent: the glass shows through.
  const alpha = Number(/rgba?\([^)]*,\s*([\d.]+)\)\s*$/.exec(dock.bg)?.[1] ?? 1);
  expect(alpha).toBeLessThan(0.7);
  // Its lit lower edge and dark hair.
  expect(dock.shadow).toMatch(/inset/);
  // The capsule: real glass as the capsule, none docked; flyouts only while open; the footer painted.
  const capFilter = () => page.locator("[data-site-bar] .cap-bg").evaluate((c) => getComputedStyle(c).backdropFilter);
  expect(await capFilter()).toBe("none");
  await page.evaluate(() => scrollTo(0, 3000));
  await expect(page.locator("[data-site-bar]")).toHaveAttribute("data-state", "capsule");
  expect(await capFilter()).toMatch(/blur/);
  expect(await page.locator("footer .foot-panel").evaluate((f) => getComputedStyle(f).backdropFilter)).toBe("none");
});

for (const theme of ["dark", "light"] as const) {
  for (const size of [{ width: 1440, height: 900 }, { width: 2560, height: 1270 }]) {
    test(`the bar's words and the first screen's on the glass are AA over it, every song (${theme}, ${size.width})`, async ({ page }) => {
      test.setTimeout(90_000);
      await page.setViewportSize(size);
      await start(page, theme);
      const found: string[] = [];
      for (const id of SONGS) {
        await pick(page, id);
        found.push(...low(await textContrast(page, "[data-site-bar] .bn, [data-site-bar] .b-sec .lbl, [data-site-bar] [data-clock] span"), `${id} bar`));
        found.push(...low(await textContrast(page, "[data-head] .line, [data-col] .t-lede, [data-col] .meta, [data-col] .credit a, [data-col] .credit"), `${id} words`));
      }
      expect(found).toEqual([]);
    });
  }

  test(`the capsule, the song's flyout and the footer panel keep their words AA (${theme})`, async ({ page }) => {
    test.setTimeout(90_000);
    await page.setViewportSize({ width: 1440, height: 900 });
    await start(page, theme);
    const found: string[] = [];
    for (const id of ["california-lullabye", "invisible-light"]) {
      await pick(page, id);
      // The song's flyout from the docked tray icon, over the window.
      await page.evaluate(() => scrollTo(0, 0));
      await page.waitForTimeout(300);
      await page.evaluate(() => document.getElementById("fly-song")!.showPopover());
      await page.waitForTimeout(400);
      found.push(...low(await textContrast(page, "#fly-song .fq-title, #fly-song .fq-artist, #fly-song .fq-time"), `${id} flyout`));
      await page.evaluate(() => document.getElementById("fly-song")!.hidePopover());
      // The capsule over two sections.
      for (const sec of ["light", "questions"]) {
        await page.evaluate((s) => document.getElementById(s)!.scrollIntoView(), sec);
        await expect(page.locator("[data-site-bar]")).toHaveAttribute("data-state", "capsule");
        await page.waitForTimeout(500);
        found.push(...low(await textContrast(page, "[data-site-bar] .b-sec[aria-current] .lbl"), `${id} capsule over ${sec}`));
      }
    }
    await page.evaluate(() => scrollTo(0, document.documentElement.scrollHeight));
    await page.waitForTimeout(600);
    found.push(...low(await textContrast(page, "footer .foot-panel a, footer .foot-panel p, footer .foot-panel h2", 30), "footer"));
    expect(found).toEqual([]);
  });
}

for (const theme of ["dark", "light"] as const) {
  test(`/method, /credits and the 404 lie on the same desk, drawn cheaply, their words AA over it (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.emulateMedia({ colorScheme: theme });
    const found: string[] = [];
    for (const path of ["/method", "/credits", "/a-page-that-is-not-there"]) {
      const images: string[] = [];
      page.on("request", (r) => r.resourceType() === "image" && images.push(r.url()));
      await page.goto(path);
      await expect(page.locator(".desk-lite")).toHaveCount(1);
      await page.waitForTimeout(500);
      // No image of the home page's sheet: gradients and the bar's grain tile only.
      expect(images.filter((u) => /\/img\/desk\/k2-/.test(u)), path).toEqual([]);
      found.push(...low(await textContrast(page, "main h1, main h2, main p, main li:not(:has(img))", 14), path));
    }
    expect(found).toEqual([]);
  });
}

// The desk's lit corner stays where it is while the page scrolls over it: every text that
// passes through the top left on the way down is AA there too (large text 3), at each place.
/** Each text passing through the top left of the window, at places `step` px apart from `from`
 *  to the page's end, under AA there (large text 3). */
async function cornerPass(page: Page, label: string, from: number, step = 360) {
  const found: string[] = [];
  const end = await page.evaluate(() => document.documentElement.scrollHeight - innerHeight);
  for (let y = Math.max(0, from); y <= end; y += step) {
    await page.evaluate((y) => scrollTo(0, y), y);
    await page.waitForTimeout(450);
    const n = await page.evaluate(() => {
      let n = 0;
      for (const el of document.querySelectorAll<HTMLElement>("main :is(h1, h2, h3, p, li, a, td, th, dt, dd, figcaption, summary), footer :is(h2, p, a)")) {
        if (el.closest("[data-stage]")) continue;
        // Kept for assistive technology only (clipped away where the story shows them).
        let up: Element | null = el;
        let clipped = false;
        while (up && !clipped) {
          clipped = getComputedStyle(up).clipPath === "inset(50%)";
          up = up.parentElement;
        }
        if (clipped) continue;
          if (el.querySelector("p, li, h2, h3, a, td") || el.closest("[data-replica], [data-piece], .win-body, .explorer")) continue;
          // Its own words (not only a row of drawn buttons, each on its own ground).
          if (![...el.childNodes].some((c) => c.nodeType === 3 && c.textContent!.trim())) continue;
          const r = el.getBoundingClientRect();
          if (!r.width || r.top < 0 || r.left > innerWidth * 0.5 || r.top > innerHeight * 0.45) continue;
          // Shown, and not clipped away or covered (a closed answer, a pane over it), nor part
          // under the floating bar as it passes: its top, middle and foot are all its own.
          if (!el.checkVisibility({ opacityProperty: true, visibilityProperty: true, contentVisibilityAuto: true })) continue;
          // Across its width too: the capsule sits at the top centre, so a line can start clear
          // of it at the left and run under it.
          const own = (y: number) =>
            [r.left + Math.min(r.width, 40) / 2, r.left + r.width / 2, r.right - Math.min(r.width, 40) / 2].every((x) => {
              const hit = document.elementFromPoint(x, y);
              return !!hit && (el.contains(hit) || hit.contains(el));
            });
          if (![r.top + 2, r.top + r.height / 2, r.bottom - 2].every(own)) continue;
          const cs = getComputedStyle(el);
          const px = parseFloat(cs.fontSize);
          const large = px >= 24 || (px >= 18.66 && Number(cs.fontWeight) >= 700);
          el.dataset.corner = large ? "l" : "s";
          n++;
        }
        return n;
      });
    if (n) {
      for (const [k, min] of [["s", 4.5], ["l", 3]] as const)
        found.push(...(await textContrast(page, `[data-corner="${k}"]`, 40)).filter((r) => r.ratio < min).map((r) => `${label} y=${y}: "${r.text}" ${r.ratio}`));
      await page.evaluate(() => document.querySelectorAll("[data-corner]").forEach((e) => e.removeAttribute("data-corner")));
    }
  }
  return found;
}

test("dark: text scrolling through the desk's lit corner stays AA at every place", async ({ page }) => {
  test.setTimeout(240_000);
  await page.setViewportSize({ width: 1440, height: 900 });
  await start(page, "dark");
  const found: string[] = [];
  for (const id of ["california-lullabye", "watching-seabirds-soar"]) {
    await pick(page, id);
    const stageEnd = await page.locator("[data-stage]").evaluate((s) => s.getBoundingClientRect().bottom + scrollY);
    found.push(...(await cornerPass(page, id, stageEnd - 900)));
  }
  expect(found).toEqual([]);
});

test("dark: on /method and /credits too, text scrolling through the lit corner stays AA", async ({ page }) => {
  test.setTimeout(240_000);
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.emulateMedia({ colorScheme: "dark" });
  const found: string[] = [];
  for (const path of ["/method", "/credits"]) {
    await page.goto(path);
    await page.waitForTimeout(500);
    found.push(...(await cornerPass(page, path, 0, 300)));
  }
  expect(found).toEqual([]);
});

// The skip links show at the top left, over the brightest of the desk: their focus ring lies on
// a halo of the page's own ground, so it reads at 3 or more over any desk, in both themes.
for (const theme of ["dark", "light"] as const) {
  test(`the skip links' focus ring lies on its own ground (${theme})`, async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await start(page, theme);
    for (const sel of ["a.skip[href='#main']", "a.skip-app"]) {
      const link = page.locator(sel);
      await page.keyboard.press("Tab");
      await link.focus();
      await expect(link).toBeFocused();
      const ring = await link.evaluate((el) => {
        const cs = getComputedStyle(el);
        const m = /(rgba?\([^)]*\)|color\([^)]*\))\s+0px\s+0px\s+0px\s+([\d.]+)px/.exec(cs.boxShadow);
        return { outline: cs.outlineColor, width: parseFloat(cs.outlineWidth), offset: parseFloat(cs.outlineOffset), halo: m?.[1] ?? "", spread: Number(m?.[2] ?? 0) };
      });
      expect(ring.halo, `${sel} has a halo`).not.toBe("");
      expect(ring.spread, `${sel} halo under the whole ring`).toBeGreaterThanOrEqual(ring.offset + ring.width);
      const lum = (c: string) => {
        const unit = c.startsWith("color(") ? 1 : 255;
        const [r, g, b] = c.replace(/^color\(srgb/, "").match(/[\d.]+/g)!.slice(0, 3).map((v) => {
          const s = Number(v) / unit;
          return s <= 0.04045 ? s / 12.92 : ((s + 0.055) / 1.055) ** 2.4;
        });
        return 0.2126 * r + 0.7152 * g + 0.0722 * b;
      };
      const [x, y] = [lum(ring.outline), lum(ring.halo)].sort((p, q) => q - p);
      expect((x + 0.05) / (y + 0.05), sel).toBeGreaterThanOrEqual(3);
    }
  });
}
