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
