import { expect, test, type Page } from "@playwright/test";
import { skipTour, wake } from "./helpers";
import { figureFor } from "../src/data/numbers";

test.use({ viewport: { width: 1440, height: 900 } });

const tour = (page: Page) => page.locator("[data-tour]");
const title = (page: Page) => page.locator("[data-tour-title]");
const replica = (page: Page) => page.locator("[data-replica]").first();

test("the tour starts with Press play once the screen wakes", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  await expect(tour(page)).toBeVisible();
  await expect(title(page)).toHaveText("Press play.");
});

test("each step advances when the visitor does it", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  const r = replica(page);
  await r.getByRole("button", { name: "Play", exact: true }).click();
  await expect(title(page)).toHaveText("Pick another song.");

  await r.getByRole("button", { name: "Play Origami" }).click();
  await expect(title(page)).toHaveText("Find a song by its link.");
  expect(await page.evaluate(() => getComputedStyle(document.documentElement).getPropertyValue("--glow").trim())).toMatch(
    /^(#d4a23a|rgb\(212, 162, 58\))$/i,
  );

  const link = await tour(page).locator("[data-tour-link]").textContent();
  await r.getByRole("button", { name: "Back" }).click();
  await r.getByRole("searchbox").fill(link!.trim());
  await r.getByRole("searchbox").press("Enter");
  await expect(title(page)).toHaveText("Minimise it to the tray.");

  await r.getByRole("button", { name: "Minimise featherplay" }).click();
  await expect(title(page)).toHaveText("That's featherplay.");
  await expect(tour(page).getByRole("link", { name: "Download for Windows" })).toBeVisible();
  await expect(tour(page)).toContainText("installer or portable exe");
});

test('"Show me" moves the cursor to the target and completes the step', async ({ page }) => {
  await page.goto("/");
  await wake(page);
  const cursor = page.locator("[data-tour-cursor]");
  await tour(page).getByRole("button", { name: "Show me" }).click();
  await expect(cursor).toBeVisible();
  await expect(title(page)).toHaveText("Pick another song.", { timeout: 4000 });
  // The cursor came to rest on the play button.
  const c = (await cursor.boundingBox())!;
  const play = (await replica(page).locator(".play").boundingBox())!;
  expect(c.x).toBeGreaterThan(play.x - 4);
  expect(c.x).toBeLessThan(play.x + play.width + 4);
  expect(c.y).toBeGreaterThan(play.y - 4);
  expect(c.y).toBeLessThan(play.y + play.height + 4);

  for (const next of ["Find a song by its link.", "Minimise it to the tray.", "That's featherplay."]) {
    await tour(page).getByRole("button", { name: "Show me" }).click();
    await expect(title(page)).toHaveText(next, { timeout: 6000 });
  }
});

test("Close ends the tour and leaves free play", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  await tour(page).getByRole("button", { name: "Close the tour" }).click();
  await expect(title(page)).toBeHidden();
  // The way back in: where the card was, beside the laptop's screen, not over it (the name
  // and its Take the tour have scrolled away by now).
  const start = page.locator("[data-tour-reopen]");
  await expect(start).toBeVisible();
  await expect(start).toBeFocused();
  await expect(start).toHaveAccessibleName("Take the tour");
  const overlaps = (a: { x: number; y: number; width: number; height: number }, b: typeof a) =>
    a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;
  expect(overlaps((await start.boundingBox())!, (await page.locator("[data-hero] [data-screen]").boundingBox())!)).toBe(false);
  await replica(page).getByRole("button", { name: "Play", exact: true }).click();
  await expect(replica(page).getByRole("button", { name: "Pause", exact: true })).toBeVisible();
  // It can start again.
  await start.click();
  await expect(title(page)).toBeVisible();
  await expect(tour(page).getByRole("button", { name: "Show me" })).toBeFocused();
});

test("the tour is fully operable by keyboard and announces each step (aria-live polite)", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  await expect(tour(page).locator("[aria-live=polite]")).toContainText("Press play.");
  const showMe = tour(page).getByRole("button", { name: "Show me" });
  await showMe.focus();
  await page.keyboard.press("Enter");
  await expect(tour(page).locator("[aria-live=polite]")).toContainText("Pick another song.", { timeout: 4000 });
  await expect(showMe).toBeFocused();
  await page.keyboard.press("Tab");
  await expect(tour(page).getByRole("button", { name: "Next step" })).toBeFocused();
});

test("the tray step shows featherplay's measured memory figure", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  await replica(page).getByRole("button", { name: "Minimise featherplay" }).click();
  const meter = page.locator("[data-meter]");
  await expect(meter).toBeVisible();
  // Minimised and playing: featherplay 0.2.2 on 10 November, 42 MB; Pear and Edge on 1 November, 798 and 988 MB.
  for (const [app, mb] of [
    ["featherplay", 42],
    ["Pear Desktop", 798],
    ["YouTube Music in Edge", 988],
  ] as const) {
    expect(figureFor(app).memoryMB).toBe(mb);
    await expect(meter.locator(`[data-meter-app="${app}"] [data-meter-value]`)).toHaveText(`${mb} MB`);
  }
  await expect(meter).not.toContainText(/drops|not measured/);
  await expect(meter).toContainText("10 November 2026");
});

test("Take the tour, from the top of the page, wakes the laptop and starts the tour", async ({ page }) => {
  await page.goto("/");
  await skipTour(page);
  await page.evaluate(() => scrollTo({ top: 0, behavior: "instant" }));
  await page.locator(".hero-act").getByRole("button", { name: "Take the tour" }).click();
  await expect(page.locator("[data-laptop]").first()).toHaveAttribute("data-awake", "true");
  await expect(title(page)).toBeVisible();
  await expect(tour(page).getByRole("button", { name: "Show me" })).toBeFocused();
});

test("without script there's no Take the tour button", async ({ browser }) => {
  const context = await browser.newContext({ javaScriptEnabled: false });
  const page = await context.newPage();
  await page.goto("/");
  await expect(page.getByRole("button", { name: "Take the tour" })).toBeHidden();
  await context.close();
});

const overlap = (a: { x: number; y: number; width: number; height: number }, b: typeof a) =>
  a.x < b.x + b.width && b.x < a.x + a.width && a.y < b.y + b.height && b.y < a.y + a.height;

test("Next goes ahead a step without doing it, and Close then Take the tour continues where it was", async ({ page }) => {
  await page.goto("/");
  await wake(page);
  await tour(page).getByRole("button", { name: "Next step" }).click();
  await expect(title(page)).toHaveText("Pick another song.");
  await expect(replica(page)).toHaveAttribute("data-playing", "false");
  await tour(page).getByRole("button", { name: "Close the tour" }).click();
  await expect(title(page)).toBeHidden();
  await page.locator("[data-tour-reopen]").click();
  await expect(title(page)).toHaveText("Pick another song.");
});

test('"Show me" can be stopped part way, and started again', async ({ page }) => {
  await page.goto("/");
  await wake(page);
  await tour(page).getByRole("button", { name: "Next step" }).click();
  await tour(page).getByRole("button", { name: "Next step" }).click();
  await expect(title(page)).toHaveText(/link/i);
  await tour(page).getByRole("button", { name: "Show me" }).click();
  const stop = tour(page).getByRole("button", { name: "Stop" });
  await expect(stop).toBeVisible();
  await stop.click();
  await expect(page.locator("[data-tour-cursor]")).toBeHidden();
  await page.waitForTimeout(1500);
  // Stopped: the search wasn't run, and the step is still there to do.
  await expect(title(page)).toHaveText(/link/i);
  await expect(replica(page).locator("[data-search-input]")).not.toHaveValue(/music\.example/);
  await tour(page).getByRole("button", { name: "Show me" }).click();
  await expect(title(page)).toHaveText("Minimise it to the tray.", { timeout: 8000 });
});

test('the link step\'s "Show me" pastes the link into search and lands on the song it finds', async ({ page }) => {
  await page.goto("/");
  await wake(page);
  await tour(page).getByRole("button", { name: "Next step" }).click();
  await tour(page).getByRole("button", { name: "Next step" }).click();
  await expect(tour(page).locator("[data-tour-body]")).not.toContainText(/go back/i);
  await tour(page).getByRole("button", { name: "Show me" }).click();
  await expect(title(page)).toHaveText("Minimise it to the tray.", { timeout: 8000 });
  const r = replica(page);
  await expect(r.getByRole("searchbox")).toHaveValue(/music\.example\/watch\?v=she-dreams-in-blue/);
  const row = r.locator("[data-search-result] [data-row]").first();
  await expect(row).toContainText("She Dreams in Blue");
});

for (const width of [1280, 1300, 1440, 1920]) {
  test(`at ${width} px the tour card covers neither the laptop's screen nor the memory panel`, async ({ page }) => {
    await page.setViewportSize({ width, height: 900 });
    await page.goto("/");
    await wake(page);
    const card = page.locator("[data-tour-card]");
    const screen = page.locator("[data-hero] [data-screen]");
    for (let i = 0; i < 4; i++) {
      await expect(card).toBeVisible();
      await page.waitForTimeout(400);
      expect(overlap((await card.boundingBox())!, (await screen.boundingBox())!), `step ${i + 1}`).toBe(false);
      const before = await title(page).textContent();
      await tour(page).getByRole("button", { name: "Show me" }).click();
      await expect(title(page)).not.toHaveText(before!, { timeout: 8000 });
    }
    const meter = page.locator("[data-meter]");
    await expect(meter).toBeVisible();
    await page.waitForTimeout(400);
    expect(overlap((await card.boundingBox())!, (await meter.boundingBox())!)).toBe(false);
  });
}

// The bug in the user's recording (2026-11-01): the window minimised by hand, then "Show me"
// on Press play, Pick another song and the link clicked inside the hidden window (shrunk to
// the taskbar) and nothing showed. Each step's "Show me" brings the window back first.
test('with the window minimised, "Show me" brings it back from the taskbar before it acts', async ({ page }) => {
  await page.goto("/");
  await wake(page);
  const r = replica(page);
  await r.getByRole("button", { name: "Minimise featherplay" }).click();
  await expect(r).toHaveAttribute("data-minimised", "");
  // The ring points at the taskbar button, the way back.
  const ring = page.locator("[data-tour-ring]");
  await expect(ring).toBeVisible();
  const tb = (await page.locator("[data-hero] [data-taskbar-app]").boundingBox())!;
  await expect.poll(async () => overlap((await ring.boundingBox())!, tb)).toBe(true);

  await tour(page).getByRole("button", { name: "Show me" }).click();
  await expect(title(page)).toHaveText("Pick another song.", { timeout: 6000 });
  await expect(r).not.toHaveAttribute("data-minimised", "");
  await expect(r).toHaveAttribute("data-playing", "true");

  // Minimised again mid-tour: the next steps restore it too, and act on the visible window.
  await r.evaluate((el) => el.closest("[data-desktop]")!.querySelector<HTMLElement>("[data-taskbar-app]")!.click());
  await expect(r).toHaveAttribute("data-minimised", "");
  await tour(page).getByRole("button", { name: "Show me" }).click();
  await expect(title(page)).toHaveText("Find a song by its link.", { timeout: 6000 });
  await expect(r).not.toHaveAttribute("data-minimised", "");

  await r.evaluate((el) => el.closest("[data-desktop]")!.querySelector<HTMLElement>("[data-taskbar-app]")!.click());
  await tour(page).getByRole("button", { name: "Show me" }).click();
  await expect(title(page)).toHaveText("Minimise it to the tray.", { timeout: 8000 });
  await expect(r.getByRole("searchbox")).toHaveValue(/she-dreams-in-blue/);
});
