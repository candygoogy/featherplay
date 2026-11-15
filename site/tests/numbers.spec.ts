import { expect, test, type Page } from "@playwright/test";
import { figures, metrics } from "../src/data/numbers";
import { figureText, NOT_MEASURED } from "../src/data/format";

// The numbers (Numbers.astro) and what featherplay leaves out (LeavesOut.astro): a calm table
// of facts, every figure in the HTML, no counting up; two Explorer windows and four notes that
// point at the files in Pear's folder.
const numbers = (page: Page) => page.locator("#light");
const leaves = (page: Page) => page.locator("#leaves");
const cellText = (t: string) => t.replace(/ (MB|s)$/, " $1");

test.describe("without JavaScript", () => {
  test.use({ javaScriptEnabled: false });

  test("every figure is in the table as written, the two timings featherplay's alone", async ({ page }) => {
    await page.goto("/");
    for (const f of figures) {
      for (const m of metrics) {
        const cell = numbers(page).locator(`[data-figure="${f.app}:${m}"]`);
        const t = figureText(f, m);
        if (t === NOT_MEASURED) {
          await expect(cell, `${f.app} ${m}`).toContainText("not measured");
        } else {
          await expect(cell, `${f.app} ${m}`).toHaveText(cellText(t));
        }
      }
    }
    for (const m of ["startToSoundS", "clickToSoundS"]) {
      for (const app of ["Pear Desktop", "YouTube Music in Edge"]) expect(figureText(figures.find((f) => f.app === app)!, m as never)).toBe(NOT_MEASURED);
    }
    await expect(numbers(page)).toContainText("We didn’t time Pear Desktop or Edge.");
    await expect(numbers(page)).toContainText("Edge comes with Windows");
    expect(await numbers(page).innerText()).not.toMatch(/provisional/i);
  });
});

test("nothing counts up or races: the figures are final as the table comes into view", async ({ page }) => {
  await page.goto("/");
  const memory = numbers(page).locator('[data-figure="featherplay:memoryMB"]');
  await expect(memory).toHaveText(cellText(figureText(figures[0], "memoryMB")));
  await numbers(page).scrollIntoViewIfNeeded();
  await expect(memory).toHaveText(cellText(figureText(figures[0], "memoryMB")));
  expect(await numbers(page).evaluate((s) => s.getAnimations({ subtree: true }).filter((a) => a.playState === "running").length)).toBe(0);
});

test("pointed at, a row stays and the others step back", async ({ page }) => {
  await page.goto("/");
  await numbers(page).scrollIntoViewIfNeeded();
  const row = numbers(page).locator('tr[data-metric="processes"]');
  await row.hover();
  await expect.poll(() => numbers(page).locator('tr[data-metric="memoryMB"]').evaluate((r) => Number(getComputedStyle(r).opacity))).toBeLessThan(1);
  expect(await row.evaluate((r) => Number(getComputedStyle(r).opacity))).toBe(1);
});

test("what it leaves out: four plain notes, none struck through, and no ads wording", async ({ page }) => {
  await page.goto("/");
  await leaves(page).scrollIntoViewIfNeeded();
  const notes = leaves(page).locator("[data-leaves-out] > li");
  await expect(notes).toHaveCount(4);
  await expect.poll(() => notes.locator(".note-name").allInnerTexts()).toEqual(["Chromium", "Electron", "Video", "Telemetry"]);
  await expect(notes.locator("s, del")).toHaveCount(0);
  expect(await leaves(page).innerText()).not.toMatch(/\bads\b/i);
});

test("each note lights its files in Pear's folder: the first to begin with, then on hover, on focus and pressed", async ({ page }) => {
  await page.goto("/");
  const out = leaves(page);
  await out.scrollIntoViewIfNeeded();
  const litNames = () =>
    out.locator(".pear .files li").evaluateAll((els) =>
      els.filter((el) => getComputedStyle(el).color === getComputedStyle(document.body).color).map((el) => el.querySelector(".fname")!.textContent),
    );
  // To begin with Chromium is lit, so the idea shows itself.
  await expect(out).toHaveAttribute("data-lit", "chromium");
  await expect.poll(litNames).toEqual(expect.arrayContaining(["dxcompiler.dll", "locales"]));
  // Pointed at: Video's files.
  await out.getByRole("button", { name: "Video" }).hover();
  await expect(out).toHaveAttribute("data-lit", "video");
  await expect.poll(litNames).toEqual(expect.arrayContaining(["ffmpeg.dll", "vk_swiftshader.dll"]));
  expect(await litNames()).not.toContain("YouTube Music.exe");
  // Left: back to the pressed one.
  await page.mouse.move(0, 0);
  await expect(out).toHaveAttribute("data-lit", "chromium");
  // Focus and press: Electron stays lit.
  const electron = out.getByRole("button", { name: "Electron" });
  await electron.focus();
  await expect(out).toHaveAttribute("data-lit", "electron");
  await page.keyboard.press("Enter");
  await expect(electron).toHaveAttribute("aria-pressed", "true");
  await expect(out.getByRole("button", { name: "Chromium" })).toHaveAttribute("aria-pressed", "false");
  await electron.blur();
  await expect(out).toHaveAttribute("data-lit", "electron");
});

test("the sizes are weighed like with like: installed against installed", async ({ page }) => {
  await page.goto("/");
  const weigh = leaves(page).locator("[data-weigh]");
  await expect(weigh).toContainText("Installed, about 34 MB against Pear Desktop’s 396 MB");
  await expect(weigh).toContainText("uninstaller");
  await expect(weigh).toContainText("Visual C++ runtime");
  await expect(weigh).toContainText("12 times smaller");
  // The exe in its own window, its size as Explorer shows it.
  await expect(leaves(page).locator(".ours")).toContainText("featherplay.exe");
  await expect(leaves(page).locator(".ours .status")).toContainText("29.29 MB");
  await expect(leaves(page)).not.toContainText(/nothing to install|needs no install/i);
});

test("the method link opens /method", async ({ page }) => {
  await page.goto("/");
  await numbers(page).getByRole("link", { name: "How these were measured" }).click();
  await expect(page).toHaveURL(/\/method$/);
  await expect(page.locator("h1")).toContainText("measured");
  const text = await page.locator("main").innerText();
  for (const f of figures) {
    for (const m of metrics) {
      const src = f.sources[m];
      if (src) expect(text, `${f.app} ${m}`).toContain(src.split("/").pop()!);
    }
  }
});

test.describe("/method without JavaScript", () => {
  test.use({ javaScriptEnabled: false });

  test("it gives the setup, every figure and the runs left out", async ({ page }) => {
    await page.goto("/method");
    const text = await page.locator("main").innerText();
    for (const fact of [
      "AMD Ryzen 7 9800X3D",
      "32 GB",
      "Pear Desktop 3.12.0",
      "Edge 156.0.4314.58",
      "1 November 2026",
      "14 November",
      "Start to music playing",
      "Click to sound",
      "Hovered first means",
      "run-starts.ps1",
      "run-clicks.ps1",
      "1,048,576 bytes",
      "16 logical",
      "measure-pass.ps1",
      "measure-pear.ps1",
      "measure-edge.ps1",
      "Left out",
    ]) {
      expect(text, fact).toContain(fact);
    }
    for (const f of figures) for (const m of metrics) expect(text, `${f.app} ${m}`).toContain(figureText(f, m));
    expect(text).not.toMatch(/not measured|provisional|latest measurements from development/i);
  });
});
