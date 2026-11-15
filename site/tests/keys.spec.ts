import { expect, test, type Page } from "@playwright/test";
import { wake } from "./helpers";

// The page for the keyboard and the ear: the ways past things, what is said aloud, the
// capsule's words over the moving page, the taskbar clock.
test.use({ viewport: { width: 1440, height: 900 } });

/** Tabs until `name` has the focus (at most 40 presses). */
async function tabTo(page: Page, name: string) {
  for (let i = 0; i < 40; i++) {
    await page.keyboard.press("Tab");
    if ((await page.evaluate(() => document.activeElement?.textContent?.trim())) === name) return;
  }
  throw new Error(`never reached ${name}`);
}

test("Skip to the page takes the keys past the bar to the page", async ({ page }) => {
  await page.goto("/");
  await page.keyboard.press("Tab");
  const skip = page.getByRole("link", { name: "Skip to the page" });
  await expect(skip).toBeFocused();
  await expect(skip).toBeInViewport();
  await page.keyboard.press("Enter");
  await expect(page.locator("main")).toBeFocused();
});

test("Skip past the app shows where it can be seen when it has the focus, not under the bar", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  await tabTo(page, "Skip past the app");
  const hit = await page.evaluate(() => {
    const a = document.activeElement as HTMLElement;
    const r = a.getBoundingClientRect();
    return document.elementFromPoint(r.left + r.width / 2, r.top + r.height / 2) === a;
  });
  expect(hit).toBe(true);
});

test("what is said aloud: the song once as it plays, not the scrobble row's every change", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("[data-announce][role=status]")).toHaveCount(1);
  // The scrobble row changes on every play, every song and part-way through each: it is shown,
  // not said.
  await expect(page.locator("[data-piece=scrobble] [role=status], [data-piece=scrobble] [aria-live]")).toHaveCount(0);
});

test("the capsule's section names read clearly over the page, in both themes (AA)", async ({ page }) => {
  for (const scheme of ["dark", "light"] as const) {
    await page.emulateMedia({ colorScheme: scheme });
    await page.goto("/");
    await page.evaluate(() => document.getElementById("questions")!.scrollIntoView());
    await expect(page.locator("[data-site-bar]")).toHaveAttribute("data-state", "capsule");
    await page.waitForTimeout(600);
    const ratio = await page.evaluate(() => {
      const rgba = (s: string) => {
        const m = s.match(/[\d.]+/g)!.map(Number);
        return { r: m[0], g: m[1], b: m[2], a: m[3] ?? 1 };
      };
      const over = (top: ReturnType<typeof rgba>, under: ReturnType<typeof rgba>) => ({
        r: top.r * top.a + under.r * (1 - top.a),
        g: top.g * top.a + under.g * (1 - top.a),
        b: top.b * top.a + under.b * (1 - top.a),
        a: 1,
      });
      const lum = (c: ReturnType<typeof rgba>) => {
        const f = (v: number) => ((v /= 255) <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4);
        return 0.2126 * f(c.r) + 0.7152 * f(c.g) + 0.0722 * f(c.b);
      };
      // The capsule's glass over the page's ground (its blur only softens what is under it).
      const page = rgba(getComputedStyle(document.body).backgroundColor);
      const ground = over(rgba(getComputedStyle(document.querySelector("[data-site-bar] .cap-bg")!).backgroundColor), page);
      const worst = [...document.querySelectorAll<HTMLElement>("[data-site-bar] .b-sec")]
        .filter((a) => a.getClientRects().length)
        .map((a) => {
          const t = over(rgba(getComputedStyle(a).color), ground);
          const [hi, lo] = [lum(t), lum(ground)].sort((x, y) => y - x);
          return (hi + 0.05) / (lo + 0.05);
        });
      return Math.min(...worst);
    });
    expect(ratio, scheme).toBeGreaterThanOrEqual(4.5);
  }
});

test("the taskbar clock shows the visitor's time and moves on with the minute", async ({ page }) => {
  await page.clock.install({ time: new Date(2026, 10, 15, 22, 9, 30) });
  await page.goto("/");
  const clock = page.locator("[data-site-bar] [data-clock-time]");
  const want = (h: number, m: number) => page.evaluate(([h, m]) => new Date(2026, 10, 15, h, m).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" }), [h, m]);
  await expect(clock).toHaveText(await want(22, 9));
  await page.clock.runFor(31_000);
  await expect(clock).toHaveText(await want(22, 10));
});
