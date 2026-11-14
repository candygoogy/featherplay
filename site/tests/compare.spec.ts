import { expect, test, type Page } from "@playwright/test";
import { figures, metrics, timesLess } from "../src/data/numbers";
import { figureText, formatFigure } from "../src/data/format";

const section = (page: Page) => page.locator("#light");

test.describe("without JavaScript", () => {
  test.use({ javaScriptEnabled: false });

  test("every final figure is in the HTML, and nothing waits to be measured", async ({ page }) => {
    await page.goto("/");
    for (const f of figures) {
      for (const m of metrics) {
        await expect(section(page).locator(`[data-figure="${f.app}:${m}"] [data-value]`), `${f.app} ${m}`).toHaveText(
          figureText(f, m),
        );
      }
    }
    const text = await section(page).innerText();
    expect(text).not.toMatch(/not measured|provisional/i);
    // The ratios are worked out from the figures.
    expect(text).toContain(
      `About ${timesLess("memoryMB", "Pear Desktop")} times less than Pear Desktop and ${timesLess("memoryMB", "YouTube Music in Edge")} times less than Edge.`,
    );
    expect(text).toContain("Edge comes with Windows");
  });

  test("the two timings are featherplay's alone: the others show a dash and say why, and no panel has bars", async ({ page }) => {
    await page.goto("/");
    for (const m of ["startToSoundS", "clickToSoundS"] as const) {
      const panel = section(page).locator("figure.panel", { has: page.locator(`[data-figure="featherplay:${m}"]`) });
      await expect(panel.locator(`[data-figure="Pear Desktop:${m}"] [data-value]`)).toHaveText("—");
      await expect(panel.locator(`[data-figure="YouTube Music in Edge:${m}"] [data-value]`)).toHaveText("—");
      await expect(panel).toContainText("Pear Desktop and Edge weren't part of this measurement.");
      await expect(panel.locator(".track")).toHaveCount(0);
    }
  });
});

test("the count-up runs once", async ({ page }) => {
  await page.goto("/");
  const fpMemory = section(page).locator('[data-figure="featherplay:memoryMB"] [data-value]');
  const final = formatFigure("memoryMB", figures[0].memoryMB);
  // Out of view it waits at zero.
  await expect(fpMemory).toHaveText(formatFigure("memoryMB", 0));
  await section(page).locator("[data-chart]").scrollIntoViewIfNeeded();
  await expect(fpMemory).toHaveText(final, { timeout: 4000 });
  await expect(section(page).locator("[data-chart]")).toHaveAttribute("data-counted", "true");
  // Leave and come back: it doesn't run again.
  await page.evaluate(() => scrollTo({ top: 0, behavior: "instant" }));
  await section(page).locator("[data-chart]").scrollIntoViewIfNeeded();
  await expect(fpMemory).toHaveText(final);
});

// On a phone the chart is far taller than the window (about 2150 px against 812), so it's
// never mostly on screen: the count starts from where the chart is in the window instead.
test.describe("on a phone", () => {
  test.use({ viewport: { width: 375, height: 812 } });

  test("the count-up runs once the chart comes up the window", async ({ page }) => {
    await page.goto("/");
    const chart = section(page).locator("[data-chart]");
    const fpMemory = section(page).locator('[data-figure="featherplay:memoryMB"] [data-value]');
    expect((await chart.boundingBox())!.height).toBeGreaterThan(812 / 0.4);
    await expect(fpMemory).toHaveText(formatFigure("memoryMB", 0));
    // Its top a third of the way down the window, the way a reader scrolls to it.
    await chart.evaluate((el) => scrollTo({ top: el.getBoundingClientRect().top + scrollY - innerHeight / 3, behavior: "instant" }));
    await expect(fpMemory).toHaveText(formatFigure("memoryMB", figures[0].memoryMB), { timeout: 4000 });
    await expect(chart).toHaveAttribute("data-counted", "true");
  });
});

test("what it leaves out: four quiet notes, none struck through, and no ads wording", async ({ page }) => {
  await page.goto("/");
  const items = section(page).locator("[data-leaves-out] li");
  await expect(items).toHaveCount(4);
  const names = await items.locator(".note-name").allInnerTexts();
  expect(names).toEqual(["Chromium", "Electron", "Video", "Telemetry"]);
  await expect(items.locator("s, del")).toHaveCount(0);
  for (const n of await items.locator(".note-name").all()) {
    expect(await n.evaluate((el) => getComputedStyle(el).textDecorationLine)).not.toContain("line-through");
  }
  expect(await section(page).innerText()).not.toMatch(/\bads\b/i);
});

test("each note lights its files in Pear's folder: on hover, on focus, and pressed", async ({ page }) => {
  await page.goto("/");
  const out = section(page).locator("[data-out]");
  await out.scrollIntoViewIfNeeded();
  const rows = out.locator(".folder:not(.ours) .files li");
  const lit = (group: string) =>
    rows.evaluateAll(
      (els, g) =>
        els.map((el) => ({ g: (el as HTMLElement).dataset.group ?? "", o: Number(getComputedStyle(el).opacity) })).filter((r) => r.g === g || r.o === 1),
      group,
    );
  // Nothing pointed at: every row in full.
  expect(await rows.evaluateAll((els) => els.every((el) => getComputedStyle(el).opacity === "1"))).toBe(true);

  const video = out.getByRole("button", { name: "Video" });
  await video.hover();
  await expect
    .poll(async () => (await lit("video")).every((r) => r.g === "video" && r.o === 1))
    .toBe(true);
  // ffmpeg.dll and vk_swiftshader.dll are among them.
  const names = await rows.evaluateAll((els) =>
    els.filter((el) => getComputedStyle(el).opacity === "1").map((el) => el.querySelector(".fname")!.textContent),
  );
  expect(names).toEqual(expect.arrayContaining(["ffmpeg.dll", "vk_swiftshader.dll"]));
  expect(names).not.toContain("YouTube Music.exe");

  // Pressed (a tap), it stays lit after the pointer leaves; pressing again lets go.
  const electron = out.getByRole("button", { name: "Electron" });
  await electron.click();
  await expect(electron).toHaveAttribute("aria-pressed", "true");
  await page.mouse.move(0, 0);
  await expect.poll(async () => (await lit("electron")).every((r) => r.g === "electron" && r.o === 1)).toBe(true);
  await electron.click();
  await expect(electron).toHaveAttribute("aria-pressed", "false");
});

test("the sizes are weighed like with like: Pear installed against featherplay installed, and the exe", async ({ page }) => {
  await page.goto("/");
  const weigh = section(page).locator("[data-weigh]");
  await weigh.scrollIntoViewIfNeeded();
  // MB as Windows counts them, the same unit for every size.
  await expect(weigh.locator("[data-weigh-row='pear']")).toContainText("Pear Desktop, installed");
  await expect(weigh.locator("[data-weigh-row='pear']")).toContainText("396 MB");
  await expect(weigh.locator("[data-weigh-row='installed']")).toContainText("featherplay, installed");
  await expect(weigh.locator("[data-weigh-row='installed']")).toContainText("about 35 MB");
  await expect(weigh.locator("[data-weigh-row='portable']")).toContainText("30.21 MB");
  await expect(weigh).toContainText("11 times smaller");
  await expect(weigh).toContainText("uninstaller");
  await expect(weigh).toContainText("Visual C++ runtime");
  await expect(weigh).not.toHaveAttribute("data-armed", /.*/);
});

test("the installer exists: nothing says featherplay needs no installing", async ({ page }) => {
  await page.goto("/");
  const out = section(page).locator("[data-out]");
  await expect(out).toContainText("installed or portable");
  await expect(out).not.toContainText(/nothing to install|needs no install/i);
  await expect(section(page).locator(".sum").first()).toContainText("396 MB");
});

test("the method link opens /method", async ({ page }) => {
  await page.goto("/");
  await section(page).getByRole("link", { name: "How these were measured" }).click();
  await expect(page).toHaveURL(/\/method$/);
  await expect(page.locator("h1")).toContainText("measured");
  // It names the file behind every figure.
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

test("with reduced motion the figures show at once", async ({ browser }) => {
  const ctx = await browser.newContext({ reducedMotion: "reduce" });
  const page = await ctx.newPage();
  await page.goto("/");
  await expect(section(page).locator('[data-figure="featherplay:memoryMB"] [data-value]')).toHaveText(
    formatFigure("memoryMB", figures[0].memoryMB),
  );
  await ctx.close();
});
