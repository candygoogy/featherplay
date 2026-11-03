import { expect, test, type Page } from "@playwright/test";

// The header: a plain bar at the top of the page; once the page scrolls a little it stays at
// the top of the window and becomes three floating pills (the name, the sections, the theme
// and the download), and a bar again back at the top. The current section is lit as you go.

const header = "[data-site-header]";
const surfaces = (page: Page) =>
  page.locator(`${header} [data-pill]`).evaluateAll((els) =>
    els.map((el) => {
      const cs = getComputedStyle(el, "::before");
      return Number(cs.opacity);
    }),
  );
const floating = (page: Page) => page.locator(header).evaluate((el) => el.hasAttribute("data-floating"));
const to = (page: Page, top: number) => page.evaluate((top) => scrollTo({ top, behavior: "instant" }), top);

test.describe("wide", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("a bar at the top, fixed pills once scrolled, a bar again back at the top", async ({ page }) => {
    await page.goto("/");
    await page.waitForTimeout(600);
    expect(await floating(page)).toBe(false);
    expect(await surfaces(page)).toEqual([0, 0, 0]);

    await to(page, 2400);
    await expect.poll(() => floating(page)).toBe(true);
    await expect.poll(() => surfaces(page)).toEqual([1, 1, 1]);
    // Still at the top of the window, the three in a row: name left, sections centred, actions right.
    const boxes = await page.locator(`${header} [data-pill]`).evaluateAll((els) =>
      els.map((el) => {
        const r = el.getBoundingClientRect();
        return { top: r.top, bottom: r.bottom, left: r.left, right: r.right, mid: (r.left + r.right) / 2 };
      }),
    );
    for (const b of boxes) {
      expect(b.top).toBeGreaterThanOrEqual(0);
      expect(b.bottom).toBeLessThan(80);
    }
    expect(boxes[0].right).toBeLessThan(boxes[1].left);
    expect(boxes[1].right).toBeLessThan(boxes[2].left);
    expect(Math.abs(boxes[1].mid - 720)).toBeLessThan(2);

    await to(page, 0);
    await expect.poll(() => floating(page)).toBe(false);
    await expect.poll(() => surfaces(page)).toEqual([0, 0, 0]);
  });

  test("the morph moves only transform and opacity, about 300 ms", async ({ page }) => {
    await page.goto("/");
    await page.waitForTimeout(600);
    await to(page, 2400);
    const props = await page.evaluate(async () => {
      await new Promise((r) => requestAnimationFrame(() => requestAnimationFrame(r)));
      const head = document.querySelector("[data-site-header]")!;
      return document
        .getAnimations()
        // The bar and the pills (a lit section's own colour easing isn't the morph).
        .filter((a) => a instanceof CSSTransition && ((a.effect as KeyframeEffect).target as Element).matches(".bar, [data-pill]") && head.contains((a.effect as KeyframeEffect).target as Node))
        .map((a) => ({ prop: (a as CSSTransition).transitionProperty, ms: a.effect!.getTiming().duration as number }));
    });
    expect(props.length).toBeGreaterThan(0);
    for (const p of props) {
      expect(["opacity", "transform", "translate", "scale"]).toContain(p.prop);
      expect(p.ms).toBeGreaterThanOrEqual(240);
      expect(p.ms).toBeLessThanOrEqual(340);
    }
  });

  test("the section on screen is lit in the header, and only that one", async ({ page }) => {
    await page.goto("/");
    await page.waitForTimeout(600);
    const current = () =>
      page.locator(`${header} nav a[aria-current="true"]`).evaluateAll((els) => els.map((e) => e.textContent!.trim()));
    expect(await current()).toEqual([]);
    for (const [id, name] of [
      ["light", "Numbers"],
      ["lyrics", "Lyrics"],
      ["questions", "Questions"],
    ]) {
      await page.evaluate((id) => document.getElementById(id)!.scrollIntoView({ behavior: "instant" }), id);
      await page.evaluate(() => scrollBy({ top: 120, behavior: "instant" }));
      await expect.poll(current).toEqual([name]);
    }
    // Each link carries a small icon.
    expect(await page.locator(`${header} nav a svg`).count()).toBe(3);
  });

  test("the settled laptop and the tour card sit clear of the pills", async ({ page }) => {
    for (const [w, h] of [
      [1440, 900],
      [1280, 720],
      [1024, 768],
      [1920, 1080],
    ]) {
      await page.setViewportSize({ width: w, height: h });
      await page.goto("/");
      await page.waitForTimeout(800);
      await page.evaluate(() => {
        const hero = document.querySelector<HTMLElement>("[data-hero]")!;
        scrollTo({ top: hero.offsetTop + innerHeight * 1.05, behavior: "instant" });
      });
      await expect(page.locator("[data-laptop]").first()).toHaveAttribute("data-settled", "true");
      await expect(page.locator("[data-hero] [data-tour-card]")).toBeVisible();
      await page.waitForTimeout(400);
      const r = await page.evaluate(() => {
        const pills = [...document.querySelectorAll("[data-site-header] [data-pill]")].map((p) => p.getBoundingClientRect());
        const bottom = Math.max(...pills.map((p) => p.bottom));
        const laptop = document.querySelector("[data-hero] [data-laptop]")!.getBoundingClientRect();
        const card = document.querySelector("[data-hero] [data-tour-card]")!.getBoundingClientRect();
        return { bottom, laptop: laptop.top, card: card.top };
      });
      expect(r.laptop, `${w}x${h}`).toBeGreaterThan(r.bottom + 6);
      expect(r.card, `${w}x${h}`).toBeGreaterThan(r.bottom + 6);
    }
  });
});

test("no sideways overflow from 320 to 3840 px, at the top or floating", async ({ page }) => {
  for (const w of [320, 360, 390, 600, 721, 900, 1024, 1280, 1440, 1920, 2560, 3840]) {
    await page.setViewportSize({ width: w, height: 800 });
    await page.goto("/");
    await page.waitForTimeout(300);
    for (const y of [0, 2000]) {
      await to(page, y);
      await expect.poll(() => floating(page)).toBe(y > 0);
      await page.waitForTimeout(350);
      const r = await page.evaluate(() => {
        const pills = [...document.querySelectorAll("[data-site-header] [data-pill]")]
          .filter((p) => getComputedStyle(p).display !== "none")
          .map((p) => {
            const b = p.getBoundingClientRect();
            const before = getComputedStyle(p, "::before");
            // The surface reaches past the content box by its insets.
            const inset = (v: string) => -parseFloat(v) || 0;
            return { left: b.left - inset(before.left), right: b.right + inset(before.right) };
          });
        return { sw: document.documentElement.scrollWidth, iw: innerWidth, pills };
      });
      expect(r.sw, `${w} at ${y}`).toBeLessThanOrEqual(r.iw);
      for (const p of r.pills) {
        expect(p.left, `${w} at ${y}`).toBeGreaterThanOrEqual(0);
        expect(p.right, `${w} at ${y}`).toBeLessThanOrEqual(r.iw);
      }
      // The pills never overlap each other.
      for (let i = 1; i < r.pills.length; i++) expect(r.pills[i].left, `${w} at ${y}`).toBeGreaterThan(r.pills[i - 1].right);
    }
  }
});

test.describe("phones", () => {
  test.use({ viewport: { width: 390, height: 844 } });
  test("a compact pair: the name and the download, the sections out of the way", async ({ page }) => {
    await page.goto("/");
    await to(page, 1500);
    await expect.poll(() => floating(page)).toBe(true);
    await expect(page.locator(`${header} nav`)).toBeHidden();
    await expect(page.locator(`${header} .home`)).toBeVisible();
    await expect(page.locator(`${header} .download`)).toBeVisible();
  });
});

test("a skip link leads to the content, and every item in the pills shows its focus", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await to(page, 2400);
  await expect.poll(() => floating(page)).toBe(true);
  await page.keyboard.press("Tab");
  const skip = page.getByRole("link", { name: "Skip to content" });
  await expect(skip).toBeFocused();
  await expect(skip).toBeVisible();
  const items = await page.locator(`${header} a, ${header} button`).count();
  for (let i = 0; i < items; i++) {
    await page.keyboard.press("Tab");
    const ring = await page.evaluate(() => {
      const el = document.activeElement as HTMLElement;
      const cs = getComputedStyle(el);
      return { inHeader: !!el.closest("[data-site-header]"), outline: cs.outlineStyle, width: parseFloat(cs.outlineWidth) };
    });
    expect(ring.inHeader).toBe(true);
    expect(ring.outline).not.toBe("none");
    expect(ring.width).toBeGreaterThanOrEqual(2);
  }
  await page.goto("/");
  await page.keyboard.press("Tab");
  await page.keyboard.press("Enter");
  await expect(page.locator("main")).toBeFocused();
});

test.describe("reduced motion", () => {
  test.use({ reducedMotion: "reduce", viewport: { width: 1440, height: 900 } });
  test("the header switches to pills at once", async ({ page }) => {
    await page.goto("/");
    await page.waitForTimeout(300);
    await to(page, 2400);
    await expect.poll(() => floating(page)).toBe(true);
    const r = await page.evaluate(() => {
      const head = document.querySelector("[data-site-header]")!;
      return document
        .getAnimations()
        .filter((a) => ((a.effect as KeyframeEffect).target as Element).matches(".bar, [data-pill]") && head.contains((a.effect as KeyframeEffect).target as Node)).length;
    });
    expect(r).toBe(0);
    expect(await surfaces(page)).toEqual([1, 1, 1]);
  });
});

// The pills' frosting reads cleanly over anything that passes under it. Each item's text
// (the name, the sections, the theme's icon) is measured against the frosted surface itself
// as the browser draws it, pixel by pixel, at its worst: the items' ink is hidden, the strip
// under the header is captured, and the lowest WCAG contrast between the ink and any pixel
// under each item's text is kept. Swept over the page's worst stretches: the large lyrics,
// the cover's glow behind the laptop, and the footer.
/** The lowest contrast of each pill item's ink over what is drawn under it, at scroll `y`. */
async function worstContrast(page: Page, y: number) {
  await to(page, y);
  await page.waitForTimeout(60);
  const items = await page.evaluate(() => {
    const out: { name: string; ink: string; box: { x: number; y: number; w: number; h: number } }[] = [];
    for (const el of document.querySelectorAll<HTMLElement>("[data-site-header] [data-pill] :is(.home, nav a, .theme-toggle)")) {
      if (!el.offsetParent) continue;
      const range = document.createRange();
      range.selectNodeContents(el);
      // The theme's icon only (its tooltip is hidden).
      const r = el.matches(".theme-toggle") ? el.querySelector("svg[class]")!.getBoundingClientRect() : range.getBoundingClientRect();
      const name = el.textContent!.trim() || el.getAttribute("aria-label")!;
      // The name's ink is the word's (the Tri-vane is art in its own colours).
      const ink = el.matches(".home") ? getComputedStyle(el.querySelector(".word")!).color : getComputedStyle(el).color;
      const word = el.matches(".home") ? el.querySelector(".word")!.getBoundingClientRect() : r;
      if (word.width === 0) continue;
      out.push({ name, ink, box: { x: word.x, y: word.y, w: word.width, h: word.height } });
    }
    const style = document.createElement("style");
    style.id = "no-ink";
    style.textContent =
      "[data-site-header] [data-pill] :is(.home, nav a, .theme-toggle) { color: transparent !important; transition: none !important; } [data-site-header] [data-pill] svg { visibility: hidden !important; }";
    document.head.append(style);
    return out;
  });
  const shot = (await page.screenshot({ clip: { x: 0, y: 0, width: page.viewportSize()!.width, height: 96 } })).toString("base64");
  return page.evaluate(
    async ({ shot, items }) => {
      document.getElementById("no-ink")!.remove();
      const bytes = Uint8Array.from(atob(shot), (c) => c.charCodeAt(0));
      const bmp = await createImageBitmap(new Blob([bytes], { type: "image/png" }));
      const canvas = new OffscreenCanvas(bmp.width, bmp.height);
      const g = canvas.getContext("2d")!;
      g.drawImage(bmp, 0, 0);
      const px = g.getImageData(0, 0, bmp.width, bmp.height).data;
      const lum = (r: number, gr: number, b: number) => {
        const f = (c: number) => ((c /= 255) <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
        return 0.2126 * f(r) + 0.7152 * f(gr) + 0.0722 * f(b);
      };
      return items.map(({ name, ink, box }) => {
        const [r, gr, b] = (ink.match(/[\d.]+/g) ?? []).map(Number);
        const li = lum(r, gr, b);
        let worst = Infinity;
        for (let yy = Math.floor(box.y); yy < Math.ceil(box.y + box.h); yy++) {
          for (let xx = Math.floor(box.x); xx < Math.ceil(box.x + box.w); xx++) {
            const i = (yy * bmp.width + xx) * 4;
            const lb = lum(px[i], px[i + 1], px[i + 2]);
            const c = (Math.max(li, lb) + 0.05) / (Math.min(li, lb) + 0.05);
            if (c < worst) worst = c;
          }
        }
        return { name, worst };
      });
    },
    { shot, items },
  );
}

for (const scheme of ["dark", "light"] as const) {
  test.describe(`frosting (${scheme})`, () => {
    test.use({ viewport: { width: 1440, height: 900 }, colorScheme: scheme, reducedMotion: "reduce" });
    test("the pills' text keeps AA contrast over the lyrics, the glow and the footer", async ({ page }) => {
      test.slow();
      await page.goto("/");
      await page.waitForTimeout(400);
      // Through the whole page once, so every section is drawn at its real size.
      for (let y = 0; y < (await page.evaluate(() => document.documentElement.scrollHeight)); y += 700) {
        await to(page, y);
        await page.waitForTimeout(40);
      }
      // The footer passes under the pills only where the window is shorter than it (a
      // laptop's 1440 x 640); the rest at 1440 x 900.
      const spots = [
        { spot: "glow", height: 900, span: () => [30, 1600] },
        {
          spot: "lyrics",
          height: 900,
          span: () => {
            const el = document.querySelector<HTMLElement>("#lyrics")!;
            const top = el.getBoundingClientRect().top + scrollY;
            return [top - 200, top + el.offsetHeight];
          },
        },
        {
          spot: "footer",
          height: 640,
          span: () => {
            const top = document.querySelector("body > footer")!.getBoundingClientRect().top + scrollY;
            return [top - 120, document.documentElement.scrollHeight - innerHeight];
          },
        },
      ];
      const worst: Record<string, { y: number; name: string; worst: number }> = {};
      for (const { spot, height, span } of spots) {
        await page.setViewportSize({ width: 1440, height });
        await page.waitForTimeout(100);
        const [from, to] = await page.evaluate(span);
        expect(to - from, spot).toBeGreaterThan(100);
        for (let y = from; y <= to; y += 24) {
          for (const r of await worstContrast(page, Math.round(y))) {
            if (!worst[spot] || r.worst < worst[spot].worst) worst[spot] = { y: Math.round(y), ...r };
          }
        }
      }
      console.log(`frosting (${scheme})`, JSON.stringify(worst));
      for (const [spot, w] of Object.entries(worst)) {
        expect(w.worst, `${spot}: ${w.name} at ${w.y} px`).toBeGreaterThanOrEqual(4.5);
      }
    });
  });
}
