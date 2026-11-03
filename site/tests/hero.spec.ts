import { expect, test } from "@playwright/test";
import { lidAngle, wake } from "./helpers";

test.describe("wide screens", () => {
  test.use({ viewport: { width: 1440, height: 900 } });

  test("the laptop starts open and lit, smaller and leaning back a little under the name", async ({ page }) => {
    await page.goto("/");
    const laptop = page.locator("[data-laptop]").first();
    await expect(laptop).toHaveAttribute("data-awake", "true");
    await expect(page.locator("[data-screen-sleep]").first()).toHaveCSS("opacity", "0");
    // After the short first-load opening, it rests leaning back (the lid's back never shows).
    await page.waitForTimeout(1000);
    const lean = await lidAngle(page);
    expect(lean).toBeGreaterThan(6);
    expect(lean).toBeLessThan(20);
    const h1 = (await page.locator("[data-hero] h1").boundingBox())!;
    const box = (await laptop.boundingBox())!;
    expect(box.y).toBeGreaterThan(h1.y + h1.height - 4);
    // A little air between the name (and its line) and the lid's top edge, not much.
    const side = (await page.locator("[data-hero] .hero-side").boundingBox())!;
    const gap = box.y - Math.max(h1.y + h1.height, side.y + side.height);
    expect(gap).toBeGreaterThan(44);
    expect(gap).toBeLessThan(72);
    // Smaller than it settles, and all of it, with the credit line under it, in the first screen.
    expect(box.y + box.height).toBeLessThan(900);
    const credit = (await page.locator("[data-hero] .hero-audio").boundingBox())!;
    expect(credit.y + credit.height).toBeLessThan(900);
    await expect(page.locator("[data-hero] .hero-line")).toBeVisible();
  });

  test("scrolling rises, straightens and grows it to fill the stage, and the name blurs away", async ({ page }) => {
    await page.goto("/");
    await page.waitForTimeout(1000);
    const laptop = page.locator("[data-laptop]").first();
    const start = (await laptop.boundingBox())!;
    // Part way: rising, still leaning a little, the name fading.
    await page.evaluate(() => scrollTo({ top: innerHeight * 0.35, behavior: "instant" }));
    await expect.poll(() => lidAngle(page)).toBeGreaterThan(0.5);
    const head = page.locator("[data-hero-head]");
    await expect.poll(() => head.evaluate((el) => Number(getComputedStyle(el).opacity))).toBeLessThan(1);
    // Settled: upright, larger, in the middle of the screen; the name gone and out of reach.
    await wake(page);
    await expect.poll(() => lidAngle(page)).toBe(0);
    const end = (await laptop.boundingBox())!;
    expect(end.width).toBeGreaterThan(start.width * 1.15);
    expect(end.y).toBeLessThan(start.y);
    const mid = end.y + end.height / 2;
    expect(Math.abs(mid - 450)).toBeLessThan(60);
    await expect(head).toHaveCSS("visibility", "hidden");
    await expect(laptop).toHaveAttribute("data-settled", "true");
  });

  for (const theme of ["dark", "light"]) {
    test(`the cover glow stays behind and below the laptop, masked away from the name (${theme})`, async ({ page }) => {
      await page.addInitScript((t) => localStorage.setItem("fp-theme", t), theme);
      await page.goto("/");
      await page.waitForTimeout(1000);
      const glow = await page.locator("[data-hero] [data-glow]").evaluate((el) => {
        const r = el.getBoundingClientRect();
        const cs = getComputedStyle(el);
        return { top: r.top, height: r.height, z: cs.zIndex, mask: cs.maskImage || cs.webkitMaskImage };
      });
      const h1 = (await page.locator("[data-hero] h1").boundingBox())!;
      expect(glow.z).toBe("-1");
      expect(glow.mask).toContain("linear-gradient");
      // Where the name ends, the mask has barely started: under a fifth of the way in.
      const into = (h1.y + h1.height - glow.top) / (glow.height * 0.34);
      expect(into).toBeLessThan(0.2);
    });
  }

  test("the hero pins while the laptop settles, then lets go", async ({ page }) => {
    await page.goto("/");
    const pin = page.locator("[data-hero-pin]");
    await expect(pin).toHaveCSS("position", "sticky");
    await page.evaluate(() => scrollTo({ top: innerHeight * 0.6, behavior: "instant" }));
    const top = await pin.evaluate((el) => el.getBoundingClientRect().top);
    expect(Math.abs(top)).toBeLessThan(2);
    await page.evaluate(() => {
      const hero = document.querySelector<HTMLElement>("[data-hero]")!;
      scrollTo({ top: hero.offsetTop + hero.offsetHeight + 200, behavior: "instant" });
    });
    const after = await pin.evaluate((el) => el.getBoundingClientRect().bottom);
    expect(after).toBeLessThan(10);
  });

  test("the replica works from the first screen: play at the top of the page", async ({ page }) => {
    await page.goto("/");
    await page.waitForTimeout(1000);
    const r = page.locator("[data-replica]").first();
    await r.getByRole("button", { name: "Play", exact: true }).click();
    await expect(r).toHaveAttribute("data-playing", "true");
  });

  test("one line under the name, and nothing under the download button", async ({ page }) => {
    await page.goto("/");
    const hero = page.locator("[data-hero]");
    await expect(hero.locator(".hero-line")).toHaveText(
      "A free, native Windows player for YouTube Music, light enough to forget it's running.",
    );
    // The download, and beside it the way into the tour; nothing under them.
    await expect(hero.locator(".hero-act > *")).toHaveText(["Download for Windows", "Take the tour"]);
    await expect(hero.locator(".hero-head")).not.toContainText(/unofficial|MB exe/i);
  });

  test("the lid's first-load opening is a Web Animation on its transform (the compositor plays it), from nearly shut", async ({ page }) => {
    await page.goto("/");
    const lid = page.locator("[data-hero] [data-laptop-body]");
    const opening = await lid.evaluate((el) =>
      el.getAnimations().map((a) => {
        const frames = (a.effect as KeyframeEffect).getKeyframes();
        return { props: [...new Set(frames.flatMap((f) => Object.keys(f).filter((k) => !["offset", "computedOffset", "easing", "composite"].includes(k))))], from: frames[0].transform, to: frames.at(-1)!.transform };
      }),
    );
    expect(opening).toHaveLength(1);
    expect(opening[0].props).toEqual(["transform"]);
    expect(opening[0].from).toBe("rotateX(-70deg)");
    expect(opening[0].to).toBe("rotateX(12deg)");
    await expect.poll(() => lid.evaluate((el) => el.getAnimations().length)).toBe(0);
  });

  test("as the lid opens, the screen's light and the glow behind it come up in step, on opacity alone", async ({ page }) => {
    await page.goto("/");
    const r = await page.evaluate(() => {
      const one = (sel: string) => {
        const el = document.querySelector<HTMLElement>(sel)!;
        return el.getAnimations().map((a) => {
          const t = a.effect!.getTiming();
          const frames = (a.effect as KeyframeEffect).getKeyframes();
          const props = [...new Set(frames.flatMap((f) => Object.keys(f).filter((k) => !["offset", "computedOffset", "easing", "composite"].includes(k))))];
          return { start: a.startTime, duration: t.duration, easing: t.easing, props, from: frames[0].opacity, to: frames.at(-1)!.opacity };
        });
      };
      return {
        lid: one("[data-hero] [data-laptop-body]"),
        light: one("[data-hero] [data-screen-sleep]"),
        glow: one("[data-hero] [data-glow]"),
      };
    });
    expect(r.lid).toHaveLength(1);
    // A hinge's curve, finely sampled.
    expect(r.lid[0].easing).toMatch(/^linear\(/);
    for (const part of [r.light, r.glow]) {
      expect(part).toHaveLength(1);
      expect(part[0].props).toEqual(["opacity"]);
      expect(part[0].duration).toBe(r.lid[0].duration);
      expect(part[0].easing).toBe(r.lid[0].easing);
      expect(part[0].start).toBe(r.lid[0].start);
    }
    // The screen starts dim and ends lit; the glow starts faint and ends in full.
    expect(Number(r.light[0].from)).toBeGreaterThan(0.5);
    expect(Number(r.light[0].to)).toBe(0);
    expect(Number(r.glow[0].from)).toBeLessThan(0.5);
    expect(Number(r.glow[0].to)).toBe(1);
  });

  test("while the laptop rises, the tour's hidden card and ring aren't placed on every frame", async ({ page }) => {
    await page.goto("/");
    await page.waitForTimeout(1000);
    const ring = page.locator("[data-tour-ring]");
    const before = await ring.getAttribute("style");
    for (const y of [120, 260, 400]) {
      await page.evaluate((top) => scrollTo({ top, behavior: "instant" }), y);
      await page.waitForTimeout(80);
    }
    expect(await ring.getAttribute("style")).toBe(before);
    // Settled, it's placed at once.
    await wake(page);
    await expect(page.locator("[data-hero] [data-tour-card]")).toBeVisible();
    await expect.poll(() => ring.getAttribute("style")).not.toBe(before);
  });

  test("headline words rise in once on load", async ({ page }) => {
    await page.goto("/");
    const words = page.locator("[data-hero] .rise");
    expect(await words.count()).toBeGreaterThan(5);
    const anim = await words.first().evaluate((el) => getComputedStyle(el).animationName);
    expect(anim).toContain("blur-rise");
    await expect(words.last()).toHaveCSS("opacity", "1", { timeout: 3000 });
  });
});

test.describe("phones", () => {
  test.use({ viewport: { width: 390, height: 844 } });

  test("on a phone the headline words fade and rise without a blur (cheap on a slow phone)", async ({ page }) => {
    await page.goto("/");
    const words = page.locator("[data-hero] .rise");
    const anim = await words.first().evaluate((el) => getComputedStyle(el).animationName);
    expect(anim).toBe("word-rise");
    await expect(words.last()).toHaveCSS("opacity", "1", { timeout: 3000 });
    await expect(words.last()).toHaveCSS("filter", "none");
  });

  test("on a 390 px screen nothing pins and the lid starts flat", async ({ page }) => {
    await page.goto("/");
    await expect(page.locator("[data-hero-pin]")).not.toHaveCSS("position", "sticky");
    expect(await lidAngle(page)).toBe(0);
    await expect(page.locator("[data-screen-sleep]").first()).toHaveCSS("opacity", "0");
  });
});

test.describe("reduced motion", () => {
  test.use({ viewport: { width: 1440, height: 900 }, reducedMotion: "reduce" });

  test("the headline, subline and download button are visible and clickable at once", async ({ page }) => {
    await page.goto("/");
    const hero = page.locator("[data-hero]");
    for (const el of [hero.locator("h1"), hero.locator(".hero-line"), hero.getByRole("link", { name: "Download for Windows" })]) {
      await expect(el).toBeVisible();
      expect(await el.evaluate((n) => Number(getComputedStyle(n).opacity))).toBe(1);
    }
    for (const w of await hero.locator(".rise").all()) {
      expect(await w.evaluate((n) => Number(getComputedStyle(n).opacity))).toBe(1);
    }
    await hero.getByRole("link", { name: "Download for Windows" }).click({ trial: true, timeout: 500 });
    await expect(page.locator("[data-hero-pin]")).not.toHaveCSS("position", "sticky");
    expect(await lidAngle(page)).toBe(0);
    await expect(page.locator("[data-screen-sleep]").first()).toHaveCSS("opacity", "0");
  });
});
