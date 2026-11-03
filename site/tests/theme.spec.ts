import { expect, test } from "@playwright/test";

const theme = (page: import("@playwright/test").Page) =>
  page.locator("html").getAttribute("data-theme");

test("defaults to dark when the system has no preference", async ({ page }) => {
  // Chromium reports an emulated "no-preference" as light, so a browser that states no
  // preference is modelled directly: neither scheme query matches.
  await page.addInitScript(() => {
    const real = window.matchMedia.bind(window);
    window.matchMedia = (q: string) => {
      const m = real(q);
      if (!q.includes("prefers-color-scheme")) return m;
      return {
        matches: false,
        media: q,
        addEventListener: m.addEventListener.bind(m),
        removeEventListener: m.removeEventListener.bind(m),
      } as unknown as MediaQueryList;
    };
  });
  await page.goto("/");
  expect(await theme(page)).toBe("dark");
});

test("follows prefers-color-scheme: light", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "light" });
  await page.goto("/");
  expect(await theme(page)).toBe("light");
});

test("the toggle persists across reloads", async ({ page }) => {
  await page.goto("/");
  expect(await theme(page)).toBe("dark");
  await page.getByRole("button", { name: "Theme: System. Switch to Light" }).click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  expect(await page.evaluate(() => localStorage.getItem("fp-theme"))).toBe("light");
  await page.reload();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await expect(page.getByRole("button", { name: "Theme: Light. Switch to Dark" })).toBeVisible();
});

// One button, three modes: System (follow Windows), Light, Dark, and round again. Its icon is
// the mode it's in (a monitor, the sun, the moon), its name and tooltip say the mode and what a
// click does next.
const toggle = "[data-site-header] [data-theme-toggle]";
const mode = (page: import("@playwright/test").Page) =>
  page.locator(toggle).evaluate((b) => {
    const o = (s: string) => Number(getComputedStyle(b.querySelector(s)!).opacity);
    return {
      name: b.getAttribute("aria-label"),
      tip: b.querySelector("[data-tip]")?.textContent?.trim() ?? null,
      stored: localStorage.getItem("fp-theme"),
      theme: document.documentElement.dataset.theme,
      icons: { system: o(".system"), sun: o(".sun"), moon: o(".moon") },
    };
  });

test("the toggle cycles System, Light, Dark and back to System, saying each", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await page.goto("/");
  await page.waitForTimeout(400);
  const steps = [
    { name: "Theme: System. Switch to Light", stored: null, theme: "dark", icons: { system: 1, sun: 0, moon: 0 } },
    { name: "Theme: Light. Switch to Dark", stored: "light", theme: "light", icons: { system: 0, sun: 1, moon: 0 } },
    { name: "Theme: Dark. Switch to System", stored: "dark", theme: "dark", icons: { system: 0, sun: 0, moon: 1 } },
    { name: "Theme: System. Switch to Light", stored: null, theme: "dark", icons: { system: 1, sun: 0, moon: 0 } },
  ];
  for (const [i, step] of steps.entries()) {
    if (i) await page.locator(toggle).click();
    await expect.poll(() => mode(page)).toEqual({ ...step, tip: step.name });
  }
  // Dark is remembered over a reload, and says so.
  await page.locator(toggle).click();
  await page.locator(toggle).click();
  await page.reload();
  await expect.poll(() => mode(page)).toEqual({ ...steps[2], tip: steps[2].name });
});

test("back on System the page follows the OS live again; on Light or Dark it doesn't", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await page.goto("/");
  await page.locator(toggle).click(); // Light
  await page.emulateMedia({ colorScheme: "dark" });
  await page.waitForTimeout(100);
  expect(await theme(page)).toBe("light");
  await page.locator(toggle).click(); // Dark
  await page.emulateMedia({ colorScheme: "light" });
  await page.waitForTimeout(100);
  expect(await theme(page)).toBe("dark");
  await page.locator(toggle).click(); // System: the OS is light now
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await page.emulateMedia({ colorScheme: "dark" });
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  await page.emulateMedia({ colorScheme: "light" });
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  expect(await page.evaluate(() => localStorage.getItem("fp-theme"))).toBeNull();
  await expect(page.locator(toggle)).toHaveAttribute("aria-label", "Theme: System. Switch to Light");
});

test("the tooltip shows on hover and keyboard focus, and Escape puts it away", async ({ page }) => {
  await page.goto("/");
  await page.waitForTimeout(400);
  const tip = page.locator(`${toggle} [data-tip]`);
  await expect(tip).toHaveCSS("opacity", "0");
  await page.locator(toggle).hover();
  await expect(tip).toHaveCSS("opacity", "1");
  await page.keyboard.press("Escape");
  await expect(tip).toHaveCSS("opacity", "0");
  await page.mouse.move(0, 400);
  await page.locator(toggle).focus();
  await page.keyboard.press("Shift+Tab");
  await page.keyboard.press("Tab");
  await expect(page.locator(toggle)).toBeFocused();
  await expect(tip).toHaveCSS("opacity", "1");
});

for (const [label, viewport] of [
  ["the pills", { width: 1440, height: 900 }],
  ["a phone", { width: 390, height: 844 }],
] as const) {
  test(`the toggle works as ${label}`, async ({ page }) => {
    await page.setViewportSize(viewport);
    await page.emulateMedia({ colorScheme: "dark" });
    await page.goto("/");
    await page.evaluate(() => scrollTo({ top: 2400, behavior: "instant" }));
    await expect(page.locator("[data-site-header]")).toHaveAttribute("data-floating", "");
    await page.waitForTimeout(400);
    await page.locator(toggle).click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(page.locator(toggle)).toHaveAttribute("aria-label", "Theme: Light. Switch to Dark");
    // Its tooltip stays inside the window.
    const r = await page.locator(`${toggle} [data-tip]`).evaluate((el) => el.getBoundingClientRect().toJSON());
    expect(r.left).toBeGreaterThanOrEqual(0);
    expect(r.right).toBeLessThanOrEqual(viewport.width);
  });
}

test("a system change restyles without reload", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await page.goto("/");
  // Nothing stored means "system".
  expect(await page.evaluate(() => localStorage.getItem("fp-theme"))).toBeNull();
  const bgDark = await page.evaluate(() => getComputedStyle(document.body).backgroundColor);
  const events: string[] = [];
  await page.exposeFunction("recordTheme", (t: string) => events.push(t));
  await page.evaluate(() =>
    window.addEventListener("themechange", (e) =>
      (window as unknown as { recordTheme: (t: string) => void }).recordTheme(
        (e as CustomEvent<string>).detail,
      ),
    ),
  );
  await page.emulateMedia({ colorScheme: "light" });
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await expect
    .poll(() => page.evaluate(() => getComputedStyle(document.body).backgroundColor))
    .not.toBe(bgDark);
  await expect.poll(() => events).toContain("light");
});

test("a stored choice ignores the system", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await page.goto("/");
  await page.evaluate(() => localStorage.setItem("fp-theme", "dark"));
  await page.reload();
  await page.emulateMedia({ colorScheme: "light" });
  await page.waitForTimeout(100);
  expect(await theme(page)).toBe("dark");
});

// The switch eases live, through the colour tokens: every theme colour is a registered
// <color> on :root that transitions (320 ms, one curve), so every surface, text and edge
// moves together and nothing is snapshotted. No View Transition, no element easing a colour
// on its own clock.
const rgb = (s: string) => (s.match(/[\d.]+/g) ?? []).slice(0, 3).map(Number);
const between = (s: string) => {
  for (const c of rgb(s)) {
    expect(c).toBeGreaterThan(40);
    expect(c).toBeLessThan(230);
  }
};

/** The switch eases only where frames keep up: where they run long, theme.ts ends it at once
 *  and remembers that for the visit (sessionStorage). Then there's no easing to measure, so a
 *  test of the easing calls this after its switch. On CI (a shared runner drawing in
 *  software, where the first frame of a switch can take over 50 ms) the test is skipped and
 *  says why; elsewhere it fails, as a fast machine should always ease. */
async function needsEasing(page: import("@playwright/test").Page) {
  const flag = await page.evaluate(() => sessionStorage.getItem("fp-theme-instant"));
  if (flag !== "1") return;
  const why = "this machine's frames ran long, so the switch ended at once by design (theme.ts)";
  test.skip(!!process.env.CI, `${why}: no easing to measure here`);
  expect(flag, why).toBeNull();
}

/** Clicks `selector` in the page and reports what moved over the next frames. */
async function switchAndWatch(page: import("@playwright/test").Page, selector: string) {
  return page.evaluate(async (selector) => {
    const root = document.documentElement;
    let vt = 0;
    const real = document.startViewTransition?.bind(document);
    if (real) (document as any).startViewTransition = (cb: any) => (vt++, real(cb));
    const frame = () => new Promise((r) => requestAnimationFrame(r));
    document.querySelector<HTMLElement>(selector)!.click();
    const t0 = performance.now();
    let elementColour = 0;
    let mid = "";
    let midLight = NaN;
    for (let i = 0; i < 60; i++) {
      await frame();
      elementColour = Math.max(
        elementColour,
        document
          .getAnimations()
          .filter(
            (a) =>
              a instanceof CSSTransition &&
              (a.effect as KeyframeEffect).target !== root &&
              a.playState === "running" &&
              /color|background|border|shadow|fill|stroke/.test(a.transitionProperty),
          ).length,
      );
      // ~160 ms in by the switch's own clock (a slow machine can start it a frame or two late).
      const t = root.getAnimations().find((a) => a instanceof CSSTransition);
      if (!mid && (t ? Number(t.currentTime) >= 160 : performance.now() - t0 >= 160)) {
        mid = getComputedStyle(root).getPropertyValue("--bg").trim();
        midLight = Number(getComputedStyle(root).getPropertyValue("--light"));
      }
      if (performance.now() - t0 > 450) break;
    }
    return {
      vt,
      mid,
      midLight,
      after: getComputedStyle(root).getPropertyValue("--bg").trim(),
      elementColour,
      theme: root.dataset.theme,
    };
  }, selector);
}

test("the header's toggle eases the page live, there and back, with nothing on a clock of its own", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await page.goto("/");
  await page.waitForTimeout(400);
  const light = await switchAndWatch(page, "[data-theme-toggle]");
  await needsEasing(page);
  expect(light.vt).toBe(0);
  expect(light.theme).toBe("light");
  expect(light.elementColour).toBe(0);
  // ~160 ms in, the ground is between #0f0f0f and #ffffff, not at either end.
  between(light.mid);
  expect(light.midLight).toBeGreaterThan(0.1);
  expect(light.midLight).toBeLessThan(0.9);
  expect(rgb(light.after)).toEqual([255, 255, 255]);
  const dark = await switchAndWatch(page, "[data-theme-toggle]");
  await needsEasing(page);
  expect(dark.vt).toBe(0);
  expect(dark.theme).toBe("dark");
  expect(dark.elementColour).toBe(0);
  between(dark.mid);
});

test("the header's toggle eases the page and turns its icon over with it", async ({ page }) => {
  await page.goto("/");
  await page.waitForTimeout(400);
  const r = await page.evaluate(async () => {
    const root = document.documentElement;
    const system = document.querySelector<SVGElement>("[data-theme-toggle] .system")!;
    const sun = document.querySelector<SVGElement>("[data-theme-toggle] .sun")!;
    const moon = document.querySelector<SVGElement>("[data-theme-toggle] .moon")!;
    const o = (el: Element) => Number(getComputedStyle(el).opacity);
    let vt = 0;
    const real = document.startViewTransition?.bind(document);
    if (real) (document as any).startViewTransition = (cb: any) => (vt++, real(cb));
    document.querySelector<HTMLElement>("[data-theme-toggle]")!.click();
    // The middle of the switch by its own clock (a busy machine starts it a frame or two late).
    for (let i = 0; i < 120; i++) {
      await new Promise((r) => requestAnimationFrame(r));
      const t = root.getAnimations().find((a) => a instanceof CSSTransition);
      if (t && Number(t.currentTime) >= 150) break;
    }
    const mid = { bg: getComputedStyle(root).getPropertyValue("--bg").trim(), system: o(system), sun: o(sun) };
    await new Promise((r) => setTimeout(r, 400));
    return { vt, mid, end: { system: o(system), sun: o(sun), moon: o(moon) } };
  });
  await needsEasing(page);
  expect(r.vt).toBe(0);
  between(r.mid.bg);
  // System's monitor turns away as Light's sun turns in, with the colours.
  expect(r.mid.system).toBeGreaterThan(0.05);
  expect(r.mid.system).toBeLessThan(0.95);
  expect(r.mid.sun).toBeGreaterThan(0.05);
  expect(r.mid.sun).toBeLessThan(0.95);
  expect(r.end).toEqual({ system: 0, sun: 1, moon: 0 });
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
});

test("a system change eases the page too", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await page.goto("/");
  await page.waitForTimeout(400);
  await page.emulateMedia({ colorScheme: "light" });
  const mid = await page.evaluate(async () => {
    // The page hears of the change on a later frame (later still on a busy machine): time
    // the middle from when the switch starts.
    const root = document.documentElement;
    while (root.dataset.theme !== "light") {
      await new Promise((r) => requestAnimationFrame(r));
    }
    // ...and the middle by the switch's own clock, not the wall's: a slow machine's first
    // frames can start it late or run a timer long past it.
    for (let i = 0; i < 120; i++) {
      const t = root.getAnimations().find((a) => a instanceof CSSTransition);
      if (!t || Number(t.currentTime) >= 140) break;
      await new Promise((r) => requestAnimationFrame(r));
    }
    return getComputedStyle(root).getPropertyValue("--bg").trim();
  });
  await needsEasing(page);
  between(mid);
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
});

test("the art drawn per theme cross-fades with the colours, both copies in one box", async ({ page }) => {
  await page.goto("/");
  await page.waitForTimeout(400);
  const r = await page.evaluate(async () => {
    const dark = document.querySelector<HTMLElement>("header .only-dark")!;
    const light = document.querySelector<HTMLElement>("header .only-light")!;
    const o = (el: Element) => Number(getComputedStyle(el).opacity);
    const start = [o(dark), o(light)];
    document.querySelector<HTMLElement>("[data-theme-toggle]")!.click();
    // The middle by the switch's own clock, not a timer's (a slow machine runs those long).
    for (let i = 0; i < 120; i++) {
      const t = document.documentElement.getAnimations().find((a) => a instanceof CSSTransition);
      if (!t || Number(t.currentTime) >= 160) break;
      await new Promise((r) => requestAnimationFrame(r));
    }
    const mid = [o(dark), o(light)];
    await new Promise((r) => setTimeout(r, 400));
    const a = dark.getBoundingClientRect();
    const b = light.getBoundingClientRect();
    return { start, mid, end: [o(dark), o(light)], same: a.width > 0 && a.x === b.x && a.y === b.y };
  });
  await needsEasing(page);
  expect(r.start).toEqual([1, 0]);
  expect(r.mid[0]).toBeGreaterThan(0.05);
  expect(r.mid[0]).toBeLessThan(0.95);
  expect(r.end).toEqual([0, 1]);
  expect(r.same).toBe(true);
});

// A frame of the switch restyles only what's on screen: the blocks out of view sit it out
// (content-visibility: hidden at their own size, so nothing moves) and come back in the new
// theme afterwards, a block at a time; every block is quiet only while it takes the switch.
test("the blocks out of view sit out the switch at their size and come back in the new theme", async ({ page }) => {
  await page.goto("/");
  await page.waitForTimeout(400);
  const r = await page.evaluate(async () => {
    const blocks = [...document.querySelectorAll<HTMLElement>("body > header, main > *, body > footer")];
    const height = document.documentElement.scrollHeight;
    const footer = document.querySelector<HTMLElement>("body > footer")!;
    document.querySelector<HTMLElement>("[data-theme-toggle]")!.click();
    const during = {
      height: document.documentElement.scrollHeight,
      footer: getComputedStyle(footer).contentVisibility,
      hero: getComputedStyle(document.querySelector("[data-hero]")!).contentVisibility,
      quiet: blocks.every((b) => b.classList.contains("theme-quiet")),
    };
    await new Promise((r) => setTimeout(r, 1500));
    return {
      height,
      during,
      after: {
        height: document.documentElement.scrollHeight,
        inline: blocks.filter((b) => b.style.contentVisibility || b.style.containIntrinsicSize || b.style.padding).length,
        quiet: blocks.filter((b) => b.classList.contains("theme-quiet")).length,
        faq: getComputedStyle(document.querySelector("#questions")!).getPropertyValue("--bg").trim(),
      },
    };
  });
  expect(r.during.footer).toBe("hidden");
  expect(r.during.hero).toBe("visible");
  expect(r.during.quiet).toBe(true);
  expect(r.during.height).toBe(r.height);
  expect(r.after).toEqual({ height: r.height, inline: 0, quiet: 0, faq: "rgb(255, 255, 255)" });
});

// A device too slow for the eased switch (its frames near the hero run long) gets the new
// theme at once instead: every token to its end, the art's cross-fades done, and later
// switches in the visit are instant from the start (sessionStorage).
/** Clicks `selector` and records the ground's colour on each frame for ~600 ms. */
async function switchFrames(page: import("@playwright/test").Page, selector: string) {
  return page.evaluate(async (selector) => {
    const root = document.documentElement;
    const start = getComputedStyle(root).getPropertyValue("--bg").trim();
    document.querySelector<HTMLElement>(selector)!.click();
    const t0 = performance.now();
    const bgs: string[] = [];
    while (performance.now() - t0 < 600) {
      await new Promise((r) => requestAnimationFrame(r));
      bgs.push(getComputedStyle(root).getPropertyValue("--bg").trim());
    }
    const end = bgs.at(-1)!;
    return {
      start,
      end,
      // The frames that showed a colour between the two themes.
      between: bgs.filter((c) => c !== start && c !== end).length,
      light: getComputedStyle(root).getPropertyValue("--light").trim(),
      flag: sessionStorage.getItem("fp-theme-instant"),
    };
  }, selector);
}

test("on a slow device the switch ends at once, and the visit's later switches are instant", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await page.goto("/");
  await page.waitForTimeout(600);
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Emulation.setCPUThrottlingRate", { rate: 6 });

  // Near the hero: the first long frames end it.
  const first = await switchFrames(page, "[data-theme-toggle]");
  expect(first.flag).toBe("1");
  expect(rgb(first.end)).toEqual([255, 255, 255]);
  expect(first.light).toBe("1");
  // At most two eased frames: a first frame just under FIRST_FRAME, then the second of two
  // in a row over LONG_FRAME ends it (theme.ts), depending on how busy the machine is.
  expect(first.between).toBeLessThanOrEqual(2);

  // Later switches skip the easing from the first frame, wherever the page is.
  await page.evaluate(() =>
    document.querySelector("body > footer")!.scrollIntoView({ behavior: "instant", block: "center" }),
  );
  await page.waitForTimeout(800);
  const later = await switchFrames(page, "[data-theme-toggle]");
  expect(rgb(later.end)).toEqual([15, 15, 15]);
  expect(later.light).toBe("0");
  expect(later.between).toBe(0);
  await cdp.send("Emulation.setCPUThrottlingRate", { rate: 1 });
});

test("at full speed the switch eases and nothing is remembered", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "dark" });
  await page.goto("/");
  await page.waitForTimeout(400);
  const r = await switchFrames(page, "[data-theme-toggle]");
  await needsEasing(page);
  expect(r.between).toBeGreaterThan(5);
  expect(r.flag).toBeNull();
});

test.describe("reduced motion", () => {
  test.use({ reducedMotion: "reduce" });
  test("the switch is instant", async ({ page }) => {
    await page.goto("/");
    const r = await page.evaluate(async () => {
      const root = document.documentElement;
      let used = false;
      const real = document.startViewTransition?.bind(document);
      if (real) (document as any).startViewTransition = (cb: any) => ((used = true), real(cb));
      document.querySelector<HTMLButtonElement>("[data-theme-toggle]")!.click();
      const bg = getComputedStyle(root).getPropertyValue("--bg").trim();
      const running = document
        .getAnimations()
        .filter((a) => a instanceof CSSTransition && (a.effect as KeyframeEffect).target === root).length;
      return { used, theme: root.dataset.theme, bg, running };
    });
    expect(r.used).toBe(false);
    expect(r.theme).toBe("light");
    expect(rgb(r.bg)).toEqual([255, 255, 255]);
    expect(r.running).toBe(0);
  });
});
