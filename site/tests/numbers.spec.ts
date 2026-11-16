import { expect, test, type Page } from "@playwright/test";
import { figures, installFolders, metrics } from "../src/data/numbers";
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
  // (The note and the pen mark write themselves in once, then nothing runs.)
  await expect.poll(() => numbers(page).evaluate((s) => s.getAnimations({ subtree: true }).filter((a) => a.playState === "running").length), { timeout: 3000 }).toBe(0);
});

test("pointed at, a row stays and the others step back", async ({ page }) => {
  await page.goto("/");
  await numbers(page).scrollIntoViewIfNeeded();
  const row = numbers(page).locator('tr[data-metric="processes"]');
  await row.hover();
  await expect.poll(() => numbers(page).locator('tr[data-metric="memoryMB"]').evaluate((r) => Number(getComputedStyle(r).opacity))).toBeLessThan(1);
  expect(await row.evaluate((r) => Number(getComputedStyle(r).opacity))).toBe(1);
});

test("where a row compares, each figure has a bar to scale; featherplay's own timings have none", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  const widths = (m: string) =>
    numbers(page)
      .locator(`tr[data-metric="${m}"] td .bar`)
      .evaluateAll((bs) => bs.map((b) => parseFloat(getComputedStyle(b, "::after").width) / b.getBoundingClientRect().width));
  const mem = await widths("memoryMB");
  const [fp, pear, edge] = figures.map((f) => f.memoryMB);
  expect(mem[0]).toBeCloseTo(fp / edge, 2);
  expect(mem[1]).toBeCloseTo(pear / edge, 2);
  expect(mem[2]).toBeCloseTo(1, 2);
  // Edge's download is nothing, and draws nothing.
  expect((await widths("downloadMB"))[2]).toBe(0);
  for (const m of ["startToSoundS", "clickToSoundS"]) await expect(numbers(page).locator(`tr[data-metric="${m}"] .bar`)).toHaveCount(0);
});

test("the note points at featherplay's memory while playing, at the owner's sizes", async ({ page }) => {
  for (const [width, height] of [[1280, 1270], [1567, 870], [2560, 1270]]) {
    await page.setViewportSize({ width, height });
    await page.goto("/");
    const note = numbers(page).locator("[data-note='played']");
    await note.scrollIntoViewIfNeeded();
    await page.evaluate(() => document.fonts.ready);
    await expect(note, `${width}`).toBeVisible();
    await expect(note).not.toHaveClass(/note-off/);
    // (The stroke's own box: its head is its lower right.)
    const tip = (await note.locator("svg path").boundingBox())!;
    const fig = (await numbers(page).locator('[data-figure="featherplay:memoryMB"] .fig').boundingBox())!;
    // The arrow ends at the figure's left, level with it.
    expect(Math.abs(tip.x + tip.width - fig.x), `${width}`).toBeLessThan(16);
    expect(tip.y + tip.height, `${width}`).toBeGreaterThan(fig.y);
    expect(tip.y + tip.height, `${width}`).toBeLessThan(fig.y + fig.height);
  }
});

test("what it leaves out: four plain notes, none struck through, and no ads wording", async ({ page }) => {
  await page.goto("/");
  await leaves(page).scrollIntoViewIfNeeded();
  const notes = leaves(page).locator("[data-leaves-out] > li");
  await expect(notes).toHaveCount(4);
  await expect.poll(() => notes.locator(".note-name").allInnerTexts()).toEqual(["Chromium", "Electron", "Video", "Telemetry"]);
  await expect(notes.locator("s, del")).toHaveCount(0);
  expect(await leaves(page).innerText()).not.toMatch(/\bads\b/i);
  // The three that are files are buttons, named by their word; telemetry, no file, is none.
  await expect(notes.getByRole("button")).toHaveCount(3);
  await expect(leaves(page).getByRole("button", { name: "Chromium", exact: true })).toHaveAttribute("aria-describedby", /out-chromium-why/);
  await expect(notes.last().getByRole("button")).toHaveCount(0);
});

test.describe("on a touch screen", () => {
  test.use({ hasTouch: true, isMobile: true, viewport: { width: 390, height: 844 } });

  test("a tap chooses a note, and it stays chosen and lit", async ({ page }) => {
    await page.goto("/");
    const out = leaves(page);
    const video = out.getByRole("button", { name: "Video" });
    await video.scrollIntoViewIfNeeded();
    await video.tap();
    await expect(video).toHaveAttribute("aria-pressed", "true");
    await expect(out).toHaveAttribute("data-lit", "video");
    await page.waitForTimeout(300);
    await expect(out).toHaveAttribute("data-lit", "video");
  });
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

test("the sizes are weighed like with like, installed against installed, and drawn to scale", async ({ page }) => {
  await page.goto("/");
  const weigh = leaves(page).locator("[data-weigh]");
  const ours = weigh.locator('[data-scale="featherplay"]');
  const pear = weigh.locator('[data-scale="pear"]');
  await expect(weigh.locator(".weigh-h")).toHaveText("Installed");
  await expect(ours).toContainText("about 34 MB");
  await expect(pear).toContainText("Pear Desktop");
  await expect(pear).toContainText("396 MB");
  await expect(weigh).toContainText("uninstaller");
  await expect(weigh).toContainText("Visual C++ runtime");
  await expect(weigh).toContainText("12 times smaller");
  // featherplay's bar is its installed size against Pear's whole bar.
  await weigh.scrollIntoViewIfNeeded();
  const [a, b] = await Promise.all([ours, pear].map((r) => r.locator(".fill").evaluate((f) => f.getBoundingClientRect().width)));
  const installed = installFolders.featherplay.bytes + installFolders.featherplay.installed.uninstallerBytes + installFolders.featherplay.installed.runtimeBytes;
  expect(a / b).toBeCloseTo(installed / installFolders.pear.bytes, 2);
  // The exe in its own window, its size as Explorer shows it.
  await expect(leaves(page).locator(".ours")).toContainText("featherplay.exe");
  await expect(leaves(page).locator(".ours .status")).toContainText("29.29 MB");
  await expect(leaves(page)).not.toContainText(/nothing to install|needs no install/i);
});

test("the one-file note points at featherplay's file", async ({ page }) => {
  for (const [width, height] of [
    [1440, 900],
    [2560, 1270],
  ]) {
    await page.setViewportSize({ width, height });
    await page.goto("/");
    const note = page.locator("[data-note='onefile']");
    await note.evaluate((n) => n.scrollIntoView({ block: "center" }));
    await expect(note).toBeVisible();
    const { tip, row } = await note.evaluate((n) => {
      // The "right" arrow's head ends at (55.6, 9.1) of its 60 x 24 drawing (Note.astro).
      const a = n.querySelector("svg")!.getBoundingClientRect();
      const r = document.querySelector("#leaves .ours .files li")!.getBoundingClientRect();
      return { tip: { x: a.left + (55.6 / 60) * a.width, y: a.top + (9.1 / 24) * a.height }, row: { left: r.left, right: r.right, top: r.top, bottom: r.bottom } };
    });
    expect(tip.x, `${width}`).toBeGreaterThan(row.left);
    expect(tip.x, `${width}`).toBeLessThan(row.left + 40);
    expect(tip.y, `${width}`).toBeGreaterThan(row.top);
    expect(tip.y, `${width}`).toBeLessThan(row.bottom);
  }
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

test("/method: the index beside the text leads to each part and marks the one being read", async ({ page }) => {
  await page.setViewportSize({ width: 1567, height: 870 });
  await page.goto("/method");
  const index = page.getByRole("navigation", { name: "On this page" });
  const links = index.getByRole("link");
  await expect(links).toHaveCount(await page.locator("main section h2").count());
  await links.filter({ hasText: "Click to sound" }).click();
  await expect(page).toHaveURL(/#click$/);
  await expect(links.filter({ hasText: "Click to sound" })).toHaveAttribute("aria-current", "true");
  // Held in view while the parts scroll past, beside the text, never over it.
  const [nav, text] = [(await index.boundingBox())!, (await page.locator("#click ~ .text").boundingBox())!];
  expect(nav.y).toBeGreaterThanOrEqual(0);
  expect(nav.x + nav.width).toBeLessThan(text.x);
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

test("on a phone the table shows the memory and the processes; its button brings the rest, and takes them away", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/");
  const rows = numbers(page).locator("tbody tr");
  const total = await rows.count();
  await expect(rows.filter({ visible: true })).toHaveCount(3);
  const all = numbers(page).locator("[data-all-rows]");
  await expect(all).toHaveText(`All ${total} measurements`);
  await expect(all).toHaveAttribute("aria-expanded", "false");
  await all.click();
  await expect(rows.filter({ visible: true })).toHaveCount(total);
  await expect(all).toHaveAttribute("aria-expanded", "true");
  await expect(all).toHaveText("Fewer measurements");
  await all.click();
  await expect(rows.filter({ visible: true })).toHaveCount(3);
  // A wider screen has every row and no button.
  await page.setViewportSize({ width: 1440, height: 900 });
  await expect(rows.filter({ visible: true })).toHaveCount(total);
  await expect(all).toBeHidden();
});
