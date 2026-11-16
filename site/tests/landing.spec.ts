import { expect, test, type Page } from "@playwright/test";
import { laidOut } from "./helpers";

// Getting somewhere on the page: a link in the bar from the top, a link from outside to a
// #section, a reload part-way down. Just after a fresh load the sections below the first screen
// wait to be laid out, with an estimate of their height (content-visibility, release.ts); a
// glide's end is fixed as it starts, so a wrong estimate used to leave a link about 2,000 px
// short. Each is checked where it really lands, with no second scroll to help it.
test.use({ viewport: { width: 1440, height: 900 } });

/** Waits until the page has stopped moving (the same scrollY for 300 ms). */
async function still(page: Page) {
  // (A glide may take a moment to start: WebKit's does.)
  await page.waitForTimeout(250);
  let last = -1;
  let same = 0;
  await expect
    .poll(
      async () => {
        const y = await page.evaluate(() => scrollY);
        same = y === last ? same + 1 : 0;
        last = y;
        return same;
      },
      { intervals: [100], timeout: 8000 },
    )
    .toBeGreaterThanOrEqual(3);
}

/** How far the target's top is from where the page puts a target (its scroll padding), or 0
 *  when the page is at its end and can't bring the target any higher. */
const offBy = (page: Page, id: string) =>
  page.evaluate((id) => {
    const root = document.documentElement;
    const t = document.getElementById(id)!;
    const want = parseFloat(getComputedStyle(root).scrollPaddingTop) + parseFloat(getComputedStyle(t).scrollMarginTop);
    const top = t.getBoundingClientRect().top;
    const atEnd = scrollY >= root.scrollHeight - innerHeight - 1;
    return atEnd && top > want ? 0 : Math.round(top - want);
  }, id);

for (const [name, id] of [
  ["Numbers", "light"],
  ["Lyrics", "lyrics"],
  ["Questions", "questions"],
  ["Download", "download"],
] as const) {
  test(`the bar's ${name} link, clicked at the top, lands on its section`, async ({ page }) => {
    await page.goto("/");
    await page.locator("[data-site-bar]").getByRole("link", { name, exact: true }).click();
    await still(page);
    expect(Math.abs(await offBy(page, id))).toBeLessThanOrEqual(3);
  });
}

test("a link to a #section from outside lands on it", async ({ page }) => {
  await page.goto("/#questions");
  await still(page);
  expect(Math.abs(await offBy(page, "questions"))).toBeLessThanOrEqual(3);
  await expect(page.locator("[data-site-bar]")).toHaveAttribute("data-state", "capsule");
});

test("a reload part-way down comes back where it was, as the capsule", async ({ page }) => {
  await page.goto("/");
  await page.evaluate(() => document.getElementById("questions")!.scrollIntoView());
  await still(page);
  const y = await page.evaluate(() => scrollY);
  await page.reload();
  await still(page);
  expect(Math.abs((await page.evaluate(() => scrollY)) - y)).toBeLessThanOrEqual(4);
  await expect(page.locator("[data-site-bar]")).toHaveAttribute("data-state", "capsule");
});

test("the section on screen is the one the bar marks", async ({ page }) => {
  await page.goto("/");
  const bar = page.locator("[data-site-bar]");
  await bar.getByRole("link", { name: "Lyrics", exact: true }).click();
  await still(page);
  await expect(bar.locator('[data-sec][aria-current="true"]')).toHaveCount(1);
  await expect(bar.getByRole("link", { name: "Lyrics", exact: true })).toHaveAttribute("aria-current", "true");
});

test("as the page starts, nothing eases in from the browser's own colours", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "light" });
  await page.goto("/");
  await page.evaluate(() => localStorage.setItem("fp-theme", "dark"));
  // The first frame of a reload: the guard is on, and no colour is on its way anywhere.
  await page.addInitScript(() => {
    addEventListener("DOMContentLoaded", () => {
      (window as unknown as { __boot: unknown }).__boot = {
        guard: document.documentElement.classList.contains("booting"),
        easing: document.getAnimations().filter((a) => a instanceof CSSTransition).length,
      };
    });
  });
  await page.reload();
  expect(await page.evaluate(() => (window as unknown as { __boot: unknown }).__boot)).toEqual({ guard: true, easing: 0 });
  await expect(page.locator("html")).not.toHaveClass(/booting/);
});

test("soon after a fresh load no section is left waiting to be laid out", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("html")).toHaveClass(/\bsplit\b/);
  await laidOut(page);
});

// Wherever a section lands from a link, its title sits wholly below the floating capsule.
for (const [w, h] of [[390, 844], [1440, 900], [2560, 1270]] as const) {
  test(`landing on each section from a link leaves its title clear of the capsule (${w}x${h})`, async ({ page }) => {
    await page.setViewportSize({ width: w, height: h });
    await page.goto("/");
    await laidOut(page);
    const ids = await page.evaluate(() => [...document.querySelectorAll("main > section[id]:not([data-stage]) h2")].map((t) => t.closest("section")!.id));
    expect(ids.length).toBeGreaterThan(4);
    // And the questions' groups, which the index links to.
    for (const id of [...ids, "q-about", "q-account", "q-use"]) {
      await page.evaluate((id) => (location.hash = id), id);
      await still(page);
      const [title, cap] = await page.evaluate((id) => {
        const t = document.querySelector(`#${id} :is(h2, h3)`)!.getBoundingClientRect();
        const c = document.querySelector("[data-site-bar] .cap-bg")!.getBoundingClientRect();
        return [{ top: t.top, bottom: t.bottom }, { bottom: c.height ? c.bottom : 0 }];
      }, id);
      expect(title.top, `${id} title under the capsule`).toBeGreaterThanOrEqual(cap.bottom + 8);
      expect(title.bottom, `${id} title on screen`).toBeLessThanOrEqual(h);
    }
  });
}
