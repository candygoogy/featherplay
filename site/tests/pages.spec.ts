import { expect, test } from "@playwright/test";

// /method, /credits and the 404 on the system: the same bar as the home page, its sections
// as links back to the home page's (no song, no tray icon, no desktop menu: there is no app
// here), the same footer, and the page's own skip link.
test.use({ viewport: { width: 1440, height: 900 } });

for (const path of ["/method", "/credits", "/a-page-that-is-not-there"]) {
  test(`${path}: the bar is the home page's, without the app, and it leads home`, async ({ page }) => {
    await page.goto(path);
    const bar = page.locator("[data-site-bar]");
    await expect(bar).toHaveAttribute("data-state", "docked");
    await expect(bar.getByRole("link", { name: "featherplay, home" })).toHaveAttribute("href", "/");
    for (const [name, href] of [
      ["The app", "/#app"],
      ["Numbers", "/#light"],
      ["Lyrics", "/#lyrics"],
      ["Features", "/#kept"],
      ["Questions", "/#questions"],
      ["Download", "/#download"],
    ]) {
      await expect(bar.getByRole("link", { name, exact: true })).toHaveAttribute("href", href);
    }
    await expect(bar.locator("[data-np], [data-tray], [data-ctx]")).toHaveCount(0);
    await expect(bar.locator("[data-clock-time]")).not.toBeEmpty();
    await expect(page.locator("footer.foot")).toBeVisible();
    // It floats once the page moves, and its theme toggle works.
    await page.evaluate(() => scrollTo(0, 400));
    await expect(bar).toHaveAttribute("data-state", "capsule");
    await page.evaluate(() => scrollTo(0, 0));
    await expect(bar).toHaveAttribute("data-state", "docked");
    const theme = await page.locator("html").getAttribute("data-theme");
    await bar.locator("[data-theme-toggle]").click();
    await expect(page.locator("html")).not.toHaveAttribute("data-theme", theme!);
    // The browser's own menu stays: no app here to have one.
    await page.mouse.click(1300, 700, { button: "right" });
    await expect(page.getByRole("menu", { name: "Desktop" })).toHaveCount(0);
  });
}

test("the 404 is the page's own words and two ways on, both in view on the first screen, with no window drawn", async ({ page }) => {
  for (const width of [1440, 390]) {
    await page.setViewportSize({ width, height: 900 });
    await page.goto("/a-page-that-is-not-there");
    const main = page.locator("main");
    await expect(main.locator("h1")).toHaveText("Not in the queue");
    for (const name of ["Back to featherplay", "Download"]) await expect(main.getByRole("link", { name }), `${width} ${name}`).toBeInViewport();
    await expect(page.locator(".nf-win, [data-replica], [data-laptop]")).toHaveCount(0);
  }
});

test("/credits credits the handwriting's face, with its licence beside the font", async ({ page, request }) => {
  await page.goto("/credits");
  const type = page.locator("#type");
  await expect(type).toContainText("Reenie Beanie by James Grieshaber");
  const href = await type.getByRole("link", { name: "licence" }).nth(1).getAttribute("href");
  expect(href).toBe("/fonts/reenie-beanie-OFL.txt");
  const res = await request.get(href!);
  expect(res.status()).toBe(200);
  expect(await res.text()).toContain("SIL OPEN FONT LICENSE Version 1.1");
});
