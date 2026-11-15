import { expect, test } from "@playwright/test";

// /method, /credits and the 404 still wear the old header (Header.astro, src/scripts/header.ts)
// until they move to the new bar: it is there, it floats once the page moves, its theme
// toggle works and its links lead back to the home page's sections.
test.use({ viewport: { width: 1440, height: 900 } });

for (const path of ["/method", "/credits", "/a-page-that-is-not-there"]) {
  test(`${path}: the header is there, floats as the page moves, and its toggle and links work`, async ({ page }) => {
    await page.goto(path);
    const header = page.locator("[data-site-header]");
    await expect(header).toBeVisible();
    await expect(header.getByRole("link", { name: "Download" })).toHaveAttribute("href", "/#download");
    await expect(header.getByRole("link", { name: "featherplay, home" })).toHaveAttribute("href", "/");
    await page.evaluate(() => scrollTo(0, 200));
    await expect(header).toHaveAttribute("data-floating", "");
    const theme = await page.locator("html").getAttribute("data-theme");
    await header.locator("[data-theme-toggle]").click();
    await expect(page.locator("html")).not.toHaveAttribute("data-theme", theme!);
  });
}
