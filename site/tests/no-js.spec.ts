import { expect, test } from "@playwright/test";

// Review Focus 1: with JavaScript off (or failing to load) the page still says what it is,
// still offers the download, and every section is there to read.
test.use({ javaScriptEnabled: false });

const sectionHeadings = [
  "Light, and it shows",
  "Lyrics, up close",
  "Make it yours",
  "Everything you'd miss, kept",
  "Questions",
  "Download for Windows",
];

test("the headline, the download link and every section heading are there", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("h1")).toContainText("featherplay");
  await expect(page.getByRole("link", { name: /download/i }).first()).toBeVisible();
  for (const name of sectionHeadings) {
    await expect(page.getByRole("heading", { level: 2, name, exact: true })).toBeVisible();
  }
});

test("the theme still follows the system without JavaScript", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "light" });
  await page.goto("/");
  const bg = await page.evaluate(() => getComputedStyle(document.body).backgroundColor);
  expect(bg).toBe("rgb(255, 255, 255)");
});
