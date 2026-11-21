import { expect, test, type Page } from "@playwright/test";
import { downloadOptions, release, sizeLabel } from "../src/data/release";
import { laidOut } from "./helpers";

// The questions, the download and the footer (Questions.astro, DownloadSection.astro,
// SiteFooter.astro), and the words the site shares with the README.
test.use({ viewport: { width: 1440, height: 900 } });

const faq = (page: Page) => page.locator("#questions");
const summary = (page: Page, q: string | RegExp) => faq(page).locator("summary", { hasText: q });
/** Apostrophes as the README writes them, spaces as one. */
const plain = (s: string) => s.replace(/’/g, "'").replace(/\s+/g, " ");

test("the questions people ask, in three groups, before the download", async ({ page }) => {
  await page.goto("/");
  await expect(faq(page).locator(".faq-group h3")).toHaveText(["About featherplay", "Your account", "Using it"]);
  for (const q of [
    "Is featherplay official?",
    "Do I need an account?",
    "How does signing in work?",
    "Is my account safe with it?",
    "What does it sound like?",
    "Does it work with stream overlays and Stream Deck?",
    "Does it run on Linux or macOS?",
    "Is it free?",
    "Some songs won’t play. Why?",
    "How do I report a bug?",
  ]) {
    await expect(summary(page, q)).toBeVisible();
  }
  const order = await page.evaluate(() => document.querySelector("#questions")!.compareDocumentPosition(document.querySelector("#download")!) & Node.DOCUMENT_POSITION_FOLLOWING);
  expect(order).toBeTruthy();
});

test("they start closed, and one answer is open at a time; the open one's question closes it", async ({ page }) => {
  await page.goto("/");
  await expect(faq(page).locator("details[open]")).toHaveCount(0);
  await summary(page, /download anything else/i).click();
  await expect(faq(page).locator("details[open] .answer")).toContainText("yt-dlp");
  await summary(page, "Is it free?").click();
  await expect(faq(page).locator("details[open]:not([data-closing])")).toHaveCount(1);
  await expect(faq(page).locator("details[open]")).toHaveCount(1);
  await expect(faq(page).locator("details[open] summary")).toContainText("Is it free?");
  await summary(page, "Is it free?").click();
  await expect(faq(page).locator("details[open]")).toHaveCount(0);
  await expect(page.locator("#download")).not.toContainText("yt-dlp");
});

/** Clicks a question and reports, by the time the answer has finished opening or closing, how
 *  far the page scrolled and the highest the question got on screen (read every frame). */
async function clickAndWatch(page: Page, q: string) {
  const s = summary(page, q);
  // Brought on screen first, and left a moment for the sections around it to render.
  await s.scrollIntoViewIfNeeded();
  await page.waitForTimeout(300);
  await s.scrollIntoViewIfNeeded();
  const before = await page.evaluate(() => scrollY);
  await s.click();
  return page.evaluate(
    ([q, before]) =>
      new Promise<{ scrolled: number; top: number }>((r) => {
        const el = [...document.querySelectorAll<HTMLElement>("#questions summary")].find((x) => x.textContent!.includes(q as string))!;
        let scrolled = 0;
        let top = Infinity;
        let frames = 0;
        const tick = () => {
          scrolled = Math.max(scrolled, Math.abs(scrollY - (before as number)));
          top = Math.min(top, el.getBoundingClientRect().y);
          if (++frames < 30) requestAnimationFrame(tick);
          else r({ scrolled, top });
        };
        requestAnimationFrame(tick);
      }),
    [q, before] as const,
  );
}

test("opening and closing an answer leaves the page where it is: the other one closing above or below it, in any order, near the end of the page too", async ({ page }) => {
  await page.goto("/");
  await laidOut(page);
  const qs = ["Is featherplay official?", "How does signing in work?", "Some songs won’t play. Why?", "Is it free?", "How do I report a bug?"];
  // Down the list (the open one closes above), back up it (it closes below), skipping about,
  // and the open one's own question (none left open).
  for (const q of [...qs, qs[1], qs[3], qs[0], qs[4], qs[2], qs[2], qs[4], qs[0]]) {
    const { scrolled, top } = await clickAndWatch(page, q);
    // The page stays put; only a question that would have risen out of sight (the long
    // sign-in answer closing above it) is held at the line under the bar instead.
    expect(scrolled <= 1 || top >= 83, `${q}: scrolled ${scrolled}, top ${top}`).toBe(true);
    expect(top, q).toBeGreaterThan(0);
    await page.waitForTimeout(150);
    expect(await faq(page).locator("details[open]").count(), q).toBeLessThanOrEqual(1);
  }
  // At the very end of the page, the last question closed: the page isn't pulled up.
  await page.evaluate(() => scrollTo(0, document.documentElement.scrollHeight));
  const last = summary(page, "How do I report a bug?");
  if ((await last.locator("xpath=..").getAttribute("open")) === null) await last.click();
  await page.waitForTimeout(400);
  await last.scrollIntoViewIfNeeded();
  expect((await clickAndWatch(page, "How do I report a bug?")).scrolled).toBeLessThanOrEqual(1);
});

test("a question that would rise out of sight when a long answer closes above it is held under the bar", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 600 });
  await page.goto("/");
  await laidOut(page);
  const open = summary(page, "How does signing in work?");
  await open.click();
  await page.waitForTimeout(400);
  // The next question, brought just under the bar: the long answer above it closes.
  const next = open.locator("xpath=../following-sibling::details[1]/summary");
  await next.evaluate((el) => scrollBy(0, el.getBoundingClientRect().top - 100));
  await page.waitForTimeout(200);
  await next.click();
  await page.waitForTimeout(500);
  const y = (await next.boundingBox())!.y;
  expect(y).toBeGreaterThanOrEqual(80);
  expect(y).toBeLessThanOrEqual(101);
});

test("the keyboard opens and closes an answer; with reduced motion the swap is at once", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto("/");
  await summary(page, "Is featherplay official?").focus();
  await page.keyboard.press("Enter");
  await expect(faq(page).locator("details[open] summary")).toContainText("Is featherplay official?");
  expect(await faq(page).locator("details[open] [data-answer]").evaluate((a) => a.getAnimations().length)).toBe(0);
  // The next one down, by the keyboard: the one above shuts at once.
  await summary(page, "Is it free?").focus();
  await page.keyboard.press("Enter");
  await expect(faq(page).locator("details[open]")).toHaveCount(1);
  await expect(faq(page).locator("details[open] summary")).toContainText("Is it free?");
  await page.keyboard.press("Enter");
  await expect(faq(page).locator("details[open]")).toHaveCount(0);
});

test.describe("without JavaScript", () => {
  test.use({ javaScriptEnabled: false });
  test("the questions are plain details that open and close, one at a time (the browser's own)", async ({ page }) => {
    await page.goto("/");
    await summary(page, "Is it free?").click();
    await expect(faq(page).locator("details[open] .answer")).toBeVisible();
    await summary(page, "Do I need an account?").click();
    await expect(faq(page).locator("details[open]")).toHaveCount(1);
    await summary(page, "Do I need an account?").click();
    await expect(faq(page).locator("details[open]")).toHaveCount(0);
  });
});

test("the footer's unofficial notice opens the answer to \"Is featherplay official?\"", async ({ page }) => {
  await page.goto("/");
  await laidOut(page);
  await page.locator("footer").getByRole("link", { name: "Unofficial notice" }).click();
  await expect(page).toHaveURL(/#official$/);
  await expect(page.locator("#official")).toHaveAttribute("open", "");
  // (A long glide up from the footer: with every engine at work at once it takes its time.)
  await expect(page.locator("#official")).toBeInViewport({ timeout: 15_000 });
  // It lands clear of the bar, not under it.
  await expect.poll(async () => Math.round((await page.locator("#official").boundingBox())!.y), { timeout: 10_000 }).toBeGreaterThan(100);
  await expect.poll(async () => Math.round((await page.locator("#official").boundingBox())!.y)).toBeLessThan(240);
});

// The site says what the repo's README says. Closed answers are in the HTML: textContent.
test("the questions and the sections say what the README says", async ({ page }) => {
  await page.goto("/");
  const text = async (sel: string) => plain(await page.locator(sel).evaluate((el) => el.textContent!));
  const q = await text("#questions");
  for (const t of [
    "doesn't read your password; the code is open",
    "Use Microsoft Edge, Use Google Chrome, Use Brave or Use Vivaldi",
    "another channel of Chrome, Edge or Brave, or Yandex Browser",
    "Sign out",
    "ListenBrainz token",
    "experimental Linux build",
    "Linux has no sign-in window",
    "on Linux in your desktop's keyring",
    "Look up YouTube directly in Settings > Playback",
    "then close the browser",
    "its entry for starting when you sign in",
    "title, artist, album and length",
    "only while lyrics are on screen",
    "first four characters of a hash",
    "through the Discord app on your own PC",
    "SHA256",
    "at most once a day",
    "Without signing in, featherplay searches",
    "Opus",
    "Settings > Streaming quality",
    "Settings > Local API",
    "port 9863",
    "type browse in its filter",
    "right-click the browse request",
  ]) {
    expect(q, t).toContain(t);
  }
  expect(q).not.toContain("There's no macOS or Linux version");
  expect(q).not.toContain("the services you turn on, nothing else");
  // Signing in in a browser: the browser closed by the visitor, then the session read.
  expect(q).toContain("Once it has closed, featherplay opens that profile again without a window");
  expect(await text("#kept")).toContain("remember");
  const out = await text("[data-out]");
  expect(out.toLowerCase()).toContain("no server of its own");
  expect(out).toContain("your own browser, which you close when you're in");
});

test("the download card: the version, Windows 10 and 11, each way to get it with its size", async ({ page }) => {
  await page.goto("/");
  const card = page.locator("[data-download-card]");
  await expect(card.locator("[data-version]")).toHaveText(release.version);
  await expect(card).toContainText("Windows 10 and 11, 64-bit");
  const options = downloadOptions();
  const rows = card.locator("[data-download-option]");
  await expect(rows).toHaveCount(options.length);
  for (const [i, o] of options.entries()) {
    const row = rows.nth(i);
    await expect(row).toHaveAttribute("data-download-option", o.id);
    await expect(row.locator("[data-size]")).toHaveText(sizeLabel(o.file).replace(" ", " "));
    const link = row.getByRole("link", { name: o.action });
    await expect(link).toHaveAttribute("href", o.file.url);
    if (o.primary) await expect(link).not.toHaveClass(/tonal/);
    else await expect(link).toHaveClass(/tonal/);
    await expect(row).toContainText(o.file.name);
  }
  await expect(card.locator('[data-download-option="installer"]')).toContainText(/recommended/i);
  await expect(page.locator("#download").getByRole("link", { name: /all releases and checksums/i })).toHaveAttribute("href", release.url);
});

test("the first-run notes sit in the card side by side, each with its mark", async ({ page }) => {
  await page.goto("/");
  const notes = page.locator("[data-download-card] [data-first-run] .fr");
  await expect(notes).toHaveCount(2);
  await expect(notes.nth(0)).toContainText("Run anyway");
  await expect(notes.nth(1)).toContainText("%APPDATA%\\featherplay");
  for (const n of await notes.all()) await expect(n.locator("svg")).toHaveCount(1);
  await notes.nth(0).scrollIntoViewIfNeeded();
  const [a, b] = [(await notes.nth(0).boundingBox())!, (await notes.nth(1).boundingBox())!];
  expect(Math.abs(a.y - b.y)).toBeLessThan(2);
});

test("Linux has its own row outside the Windows card: experimental, the AppImage, the rest pointed to", async ({ page }) => {
  await page.goto("/");
  const linux = page.locator("#download [data-download-linux]");
  expect(await linux.evaluate((el) => !!el.closest("[data-download-card]"))).toBe(false);
  await expect(linux).toContainText("Also for Linux, experimental");
  const appImage = release.linux!.appImage;
  const get = linux.getByRole("link", { name: "Download the AppImage" });
  await expect(get).toHaveAttribute("href", appImage.url);
  await expect(get).toHaveClass(/tonal/);
  await expect(linux).toContainText(appImage.name);
  await expect(linux.getByRole("link", { name: "release page" })).toHaveAttribute("href", release.linux!.page);
  await expect(linux.getByRole("link", { name: "README" })).toHaveAttribute("href", "https://github.com/candygoogy/featherplay#linux-experimental");
  await expect(linux).toContainText("glibc 2.35");
});

test("the footer: its link columns, the unofficial line, the version and what it runs on; the music credits only as a link", async ({ page }) => {
  await page.goto("/");
  const footer = page.locator("footer");
  for (const [name, href] of [
    ["Download", "/#download"],
    ["Questions", "/#questions"],
    ["How we measured", "/method"],
    ["Music credits", "/credits"],
    ["GitHub", "https://github.com/candygoogy/featherplay"],
    ["Report a bug", "https://github.com/candygoogy/featherplay/issues"],
    ["MIT licence", "https://github.com/candygoogy/featherplay/blob/main/LICENSE"],
    ["Features", "/#kept"],
    ["How light?", "/#light"],
    ["Lyrics", "/#lyrics"],
    ["Releases and changes", "https://github.com/candygoogy/featherplay/releases"],
    ["Third-party licences", "https://github.com/candygoogy/featherplay/blob/main/THIRD_PARTY.md"],
    ["Unofficial notice", "/#official"],
    ["Make it yours", "/#yours"],
    ["What it connects to", "/#connects"],
    ["Uninstalling", "/#uninstall"],
    ["Coming from Pear", "https://github.com/candygoogy/featherplay#coming-from-pear"],
    ["Privacy", "/#privacy"],
    ["candygoogy", "https://github.com/candygoogy"],
    ["Back to top", "#top"],
  ]) {
    await expect(footer.getByRole("link", { name, exact: true })).toHaveAttribute("href", href);
  }
  await expect(footer).toContainText("Free and unofficial");
  await expect(footer).toContainText(`Version ${release.version}`);
  await expect(footer).toContainText("Windows 10 and 11, 64-bit");
  await expect(footer).toContainText("Linux (experimental), glibc 2.35+");
  for (const id of ["download", "kept", "light", "lyrics", "yours", "questions", "official", "connects", "uninstall", "privacy", "from-pear"]) {
    await expect(page.locator(`#${id}`)).toHaveCount(1);
  }
  await expect(footer).not.toContainText("Josh Woodward");
  await expect(footer).not.toContainText("CC BY");
  await footer.getByRole("link", { name: "Music credits" }).click();
  await expect(page).toHaveURL(/\/credits$/);
  await expect(page.locator("[data-music-credits]")).toBeVisible();
});

test("the first screen's credit line links to the full credits, at the song playing", async ({ page }) => {
  await page.goto("/");
  await expect(page.locator("[data-stage] [data-credit] [data-credit-all]")).toHaveAttribute("href", "/credits#california-lullabye");
  await page.goto("/credits#california-lullabye");
  await expect(page.locator("#california-lullabye")).toContainText("California Lullabye");
});

test('the page never says "Ads" or offers to download music', async ({ page }) => {
  for (const path of ["/", "/method"]) {
    await page.goto(path);
    const text = await page.evaluate(() => document.body.innerText);
    expect(text).not.toMatch(/\bads?\b/i);
    expect(text).not.toMatch(/ad[- ]?block/i);
    expect(text).not.toMatch(/download (the |your )?(music|songs?|tracks?)/i);
  }
});

test("the questions' index stays in view beside them and marks the group being read", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  await laidOut(page);
  const index = page.locator("[data-faq-index]");
  await page.locator("#q-use").evaluate((g) => g.scrollIntoView({ block: "start" }));
  await expect(index.locator('a[aria-current="true"]')).toHaveText("Using it");
  await expect(index).toBeInViewport();
  await page.locator("#q-account").evaluate((g) => g.scrollIntoView({ block: "start" }));
  await expect(index.locator('a[aria-current="true"]')).toHaveText("Your account");
});

test("on a phone the download card leaves out the cropped app, and the footer what the page above already says", async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/");
  const foot = page.locator("footer");
  await expect(page.locator(".dl-shot")).toBeHidden();
  await expect(foot.getByRole("heading", { name: "Product" })).toBeHidden();
  await expect(foot.getByText("Windows 10 and 11, 64-bit")).toBeHidden();
  await expect(foot.getByRole("heading", { name: "Legal" })).toBeVisible();
  // A desktop keeps them all.
  await page.setViewportSize({ width: 1440, height: 900 });
  await expect(page.locator(".dl-shot")).toBeVisible();
  await expect(foot.getByRole("heading", { name: "Product" })).toBeVisible();
  await expect(foot.getByText("Windows 10 and 11, 64-bit")).toBeVisible();
});
