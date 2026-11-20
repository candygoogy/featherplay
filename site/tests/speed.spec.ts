import { expect, test, type Page } from "@playwright/test";
import { laidOut } from "./helpers";

// What the page costs while a song plays, with a section on screen (not only at the top): the
// review found 5.3 % of the main thread at 1x and 68 restyles a second with "Kept" in view (the
// scrobble ring's transition restarted every second, the window's progress thumb animated
// unseen, the replicas' animations running off screen) and every element restyled for the
// song colour's fade. The causes are checked directly; one measured ceiling guards the rest.
test.use({ viewport: { width: 1440, height: 900 } });

async function playAt(page: Page, id: string) {
  await page.route("**/music/**", (r) => r.abort());
  await page.goto("/");
  await page.evaluate(() => document.querySelector<HTMLElement>("#fly-song [data-do='play']")!.click());
  await expect(page.locator("html")).toHaveAttribute("data-playing", "true");
  // Once the sections have their heights: one laid out later would push this one out of view,
  // where the page rightly does less (no fades, no clocks).
  await laidOut(page);
  await page.evaluate((id) => document.getElementById(id)!.scrollIntoView(), id);
  await page.waitForTimeout(1200);
}

/** What runs: every running animation, by what it animates and where. */
const running = (page: Page) =>
  page.evaluate(() =>
    document
      .getAnimations()
      .filter((a) => a.playState === "running")
      .map((a) => {
        const t = (a.effect as KeyframeEffect).target as Element;
        const r = t.getBoundingClientRect();
        return {
          what: (a as CSSAnimation).animationName || (a as CSSTransition).transitionProperty || "script",
          cls: String((t as HTMLElement).className?.toString() ?? ""),
          inView: r.bottom > 0 && r.top < innerHeight && r.width > 0,
          section: t.closest("section")?.id ?? "",
          shown: getComputedStyle(t).opacity !== "0",
        };
      }),
  );

test("while a song plays with Kept on screen, nothing animates unseen and the scrobble line runs on the compositor", async ({ page }) => {
  await playAt(page, "kept");
  const now = await running(page);
  // Off screen, only in the section being read (the rest wait out of view, or aren’t rendered).
  expect(now.filter((a) => !a.inView && a.section !== "kept"), "off screen").toEqual([]);
  expect(now.filter((a) => !a.shown), "unseen").toEqual([]);
  // The scrobble line: one linear transform animation to the song's end, not a style a step.
  const line = page.locator("[data-sc-fill]").first();
  // (Scrolled to by the page: a line that moves is never "stable" enough for the locator's own.)
  await line.evaluate((el) => el.scrollIntoView({ block: "center" }));
  await expect.poll(() => line.evaluate((el) => el.getAnimations().map((a) => Object.keys((a.effect as KeyframeEffect).getKeyframes()[0]).includes("transform")))).toEqual([true]);
});

test("at the top, playing: the window's progress runs, its unseen thumb doesn't", async ({ page }) => {
  await page.route("**/music/**", (r) => r.abort());
  await page.goto("/");
  await page.locator("[data-window] .bar .fp-play").click();
  await expect(page.locator("html")).toHaveAttribute("data-playing", "true");
  // (A line starts on the frame after the change, src/demo/lines.ts.)
  await expect.poll(async () => (await running(page)).some((a) => a.cls.includes("fill"))).toBe(true);
  const now = await running(page);
  expect(now.filter((a) => a.cls.includes("thumb"))).toEqual([]);
  // Pointed at, the thumb shows and runs with the song.
  await page.locator("[data-window] [data-seek-line]").hover();
  await expect.poll(async () => (await running(page)).some((a) => a.cls.includes("thumb") && a.what === "script")).toBe(true);
});

test("a song change fades the colour where it is worn, not on every element of the page", async ({ page }) => {
  await playAt(page, "lyrics");
  const after = await page.evaluate(
    () =>
      new Promise<{ body: number; card: number }>((r) => {
        document.querySelector<HTMLElement>("#fly-song [data-do='play']:not([aria-current])")!.click();
        requestAnimationFrame(() =>
          requestAnimationFrame(() =>
            r({
              body: document.body.getAnimations().length,
              // (Its field's three bands fade, fields.css --w-*.)
              card: Math.min(1, document.querySelector(".lyric-card")!.getAnimations().filter((a) => (a as CSSTransition).transitionProperty.startsWith("--w-")).length),
            }),
          ),
        );
      }),
  );
  expect(after).toEqual({ body: 0, card: 1 });
});

test("measured: with each section on screen while a song plays, the page restyles at most a few times a second", async ({ page }) => {
  test.setTimeout(90_000);
  await playAt(page, "light");
  const cdp = await page.context().newCDPSession(page);
  await cdp.send("Performance.enable");
  const m = async () => Object.fromEntries((await cdp.send("Performance.getMetrics")).metrics.map((x) => [x.name, x.value]));
  const found: string[] = [];
  const ids = await page.evaluate(() => [...document.querySelectorAll("main > section[id]")].map((s) => s.id).filter((id) => id !== "app"));
  // Every section seen once first: what plays once as it comes into view has played.
  for (const id of ids) {
    await page.evaluate((id) => document.getElementById(id)!.scrollIntoView(), id);
    await page.waitForTimeout(400);
  }
  await page.waitForTimeout(1500);
  for (const id of ids) {
    await page.evaluate((id) => document.getElementById(id)!.scrollIntoView(), id);
    // The song back to its start, so no song change or scrobble falls in the 3 s measured (each
    // is its own moment of work, not the steady cost).
    await page.evaluate(() => document.querySelector<HTMLElement>("[data-window] .line")!.click());
    // Arrived (the bar marks the section, the sections around it render), then 3 s.
    await page.waitForTimeout(2000);
    const a = await m();
    await page.waitForTimeout(3000);
    const b = await m();
    const styles = (b.RecalcStyleCount - a.RecalcStyleCount) / 3;
    if (styles > 30) found.push(`${id}: ${styles} restyles a second`);
  }
  expect(found).toEqual([]);
});

test("over several lyric lines with Download on screen, nothing off screen animates (sampled, not one snapshot)", async ({ page }) => {
  await playAt(page, "download");
  const off: string[] = [];
  // Some 6 s of the song: several line changes.
  for (let i = 0; i < 60; i++) {
    for (const a of await running(page)) if (!a.inView && a.section !== "download") off.push(`${a.section}: ${a.what} on ${a.cls.split(" ")[0]}`);
    await page.waitForTimeout(100);
  }
  expect([...new Set(off)]).toEqual([]);
});
