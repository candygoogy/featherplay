import { expect, test } from "@playwright/test";
import { downloadOptions, release, sizeLabel } from "../src/data/release";

test.use({ viewport: { width: 1440, height: 900 } });

test("hovering or focusing a feature points at its place on the laptop", async ({ page }) => {
  await page.goto("/");
  const kept = page.locator("#kept");
  await kept.scrollIntoViewIfNeeded();
  for (const id of ["scrobble", "discord", "keys", "sponsor", "tray"]) {
    const item = kept.locator(`[data-feature="${id}"]`);
    await item.hover();
    await expect(kept).toHaveAttribute("data-highlight", id);
    await expect(kept.locator(`[data-spot="${id}"]`)).toHaveCSS("opacity", "1");
    await expect(item).toHaveAttribute("aria-pressed", "true");
  }
  // The scrobbling ring sits round the song in the replica's bar.
  await kept.locator('[data-feature="scrobble"]').hover();
  await page.waitForTimeout(900);
  const ring = (await kept.locator('[data-spot="scrobble"]').boundingBox())!;
  const song = (await kept.locator("[data-replica] [data-bar] .bar-song").boundingBox())!;
  expect(Math.abs(ring.x + ring.width / 2 - (song.x + song.width / 2))).toBeLessThan(6);
  expect(ring.width).toBeGreaterThan(song.width);
  // The keyboard reaches them too.
  await page.mouse.move(0, 0);
  await kept.locator('[data-feature="tray"]').focus();
  await expect(kept).toHaveAttribute("data-highlight", "tray");
});

test("scrolling through the features, each one in turn zooms the laptop to its place", async ({ page }) => {
  await page.goto("/");
  const kept = page.locator("#kept");
  const zoom = kept.locator("[data-kept-zoom]");
  const scaleOf = () => zoom.evaluate((el) => new DOMMatrixReadOnly(getComputedStyle(el).transform).a);
  // Above the first step: the whole laptop.
  await kept.evaluate((el) => scrollTo({ top: el.getBoundingClientRect().top + scrollY - innerHeight / 2, behavior: "instant" }));
  await expect.poll(scaleOf).toBeCloseTo(1, 2);
  for (const id of ["scrobble", "discord", "keys", "sponsor", "tray"]) {
    const step = kept.locator(`[data-step="${id}"]`);
    await step.evaluate((el) => {
      const r = el.getBoundingClientRect();
      scrollTo({ top: r.top + scrollY + r.height / 2 - innerHeight / 2, behavior: "instant" });
    });
    await expect(kept).toHaveAttribute("data-highlight", id);
    await expect(kept.locator(`[data-spot="${id}"]`)).toHaveCSS("opacity", "1");
    await expect.poll(scaleOf, { timeout: 3000 }).toBeGreaterThan(1.15);
    await page.waitForTimeout(850);
    // Framed well: the laptop whole in its frame, the place marked inside the screen.
    const frame = (await kept.locator("[data-kept-frame]").boundingBox())!;
    const laptop = (await kept.locator("[data-laptop]").boundingBox())!;
    expect(laptop.y).toBeGreaterThanOrEqual(frame.y);
    expect(laptop.y + laptop.height).toBeLessThanOrEqual(frame.y + frame.height);
    expect(laptop.y).toBeGreaterThanOrEqual(0);
    expect(laptop.y + laptop.height).toBeLessThanOrEqual(900);
    const screen = (await kept.locator("[data-screen]").boundingBox())!;
    const spot = (await kept.locator(`[data-spot="${id}"]`).boundingBox())!;
    const cx = spot.x + spot.width / 2;
    const cy = spot.y + spot.height / 2;
    expect(cx).toBeGreaterThan(screen.x);
    expect(cx).toBeLessThan(screen.x + screen.width);
    expect(cy).toBeGreaterThan(screen.y);
    expect(cy).toBeLessThan(screen.y + screen.height);
    // The progress shows which of the five this is.
    await expect(kept.locator(`[data-progress="${id}"]`)).toHaveCSS("width", "40px");
  }
});

test("the features say a lot in a few words, and leave the plugin talk out", async ({ page }) => {
  await page.goto("/");
  const kept = page.locator("#kept");
  await expect(kept).not.toContainText(/plugins?/i);
  for (const body of await kept.locator(".f-body").allTextContents()) expect(body.trim().length).toBeLessThan(90);
});

test("the FAQ answers the questions people ask, before the download", async ({ page }) => {
  await page.goto("/");
  const faq = page.locator("#questions");
  for (const q of [
    "Is featherplay official?",
    "Do I need an account?",
    "How does signing in work?",
    "Is my account safe with it?",
    "What does it sound like?",
    "Does it work with stream overlays and Stream Deck?",
    "Does it run on Linux or macOS?",
    "Is it free?",
    "Some songs won't play. Why?",
    "How do I report a bug?",
  ]) {
    await expect(faq.locator("summary", { hasText: q })).toBeVisible();
  }
  await faq.locator("summary", { hasText: "Is featherplay official?" }).click();
  await expect(faq.locator("details[open] .answer")).toContainText("No.");
  const order = await page.evaluate(() => {
    const q = document.querySelector("#questions")!;
    const d = document.querySelector("#download")!;
    return q.compareDocumentPosition(d) & Node.DOCUMENT_POSITION_FOLLOWING;
  });
  expect(order).toBeTruthy();
});

test("the FAQ starts closed, opens one answer at a time, and explains the two helpers", async ({ page }) => {
  await page.goto("/");
  const faq = page.locator("#questions");
  await expect(faq.locator("details[open]")).toHaveCount(0);
  const helpers = faq.locator("summary", { hasText: /download anything else/i });
  await expect(helpers).toBeVisible();
  await helpers.click();
  await expect(faq.locator("details[open] .answer")).toContainText("yt-dlp");
  await expect(faq.locator("details[open] .answer")).toContainText("Deno");
  await faq.locator("summary", { hasText: "Is it free?" }).click();
  await expect(faq.locator("details[open]")).toHaveCount(1);
  await expect(faq.locator("details[open] summary")).toContainText("Is it free?");
  // The download section no longer carries it.
  await expect(page.locator("#download")).not.toContainText("yt-dlp");
});

test("the download card shows the version, Windows 10 and 11, and each way to get it with its size", async ({ page }) => {
  await page.goto("/");
  const card = page.locator("[data-download-card]");
  await expect(card.locator("[data-version]")).toHaveText(release.version);
  await expect(card).toContainText("Windows 10 and 11");
  const options = downloadOptions();
  const rows = card.locator("[data-download-option]");
  await expect(rows).toHaveCount(options.length);
  for (const [i, o] of options.entries()) {
    const row = rows.nth(i);
    await expect(row).toHaveAttribute("data-download-option", o.id);
    await expect(row.locator("[data-size]")).toHaveText(sizeLabel(o.file));
    const link = row.getByRole("link", { name: o.action });
    await expect(link).toHaveAttribute("href", o.file.url);
    if (o.primary) await expect(link).not.toHaveClass(/tonal/);
    else await expect(link).toHaveClass(/tonal/);
  }
  await expect(card.getByRole("link", { name: /all releases/i })).toHaveAttribute("href", release.url);
  await expect(card).toContainText(/unofficial/i);
});

test("the first-run notes are part of the download card, side by side, each with its mark", async ({ page }) => {
  await page.goto("/");
  const card = page.locator("[data-download-card]");
  const notes = card.locator("[data-first-run] .note");
  await expect(notes).toHaveCount(2);
  await expect(notes.nth(0)).toContainText("Run anyway");
  await expect(notes.nth(1)).toContainText("%APPDATA%\\featherplay");
  for (const n of await notes.all()) await expect(n.locator("svg")).toHaveCount(1);
  const [a, b] = [(await notes.nth(0).boundingBox())!, (await notes.nth(1).boundingBox())!];
  expect(Math.abs(a.y - b.y)).toBeLessThan(2);
  // Nothing loose under the card any more.
  await expect(page.locator("#download .before")).toHaveCount(0);
});

test("the download sits close to the footer, and the card keeps its parts together", async ({ page }) => {
  for (const width of [390, 1560, 3840]) {
    await page.setViewportSize({ width, height: 900 });
    await page.goto("/");
    // Both rendered (they skip layout while far away): the footer's top, then the card.
    await page.locator("footer .band").scrollIntoViewIfNeeded();
    await page.locator("[data-download-card] .fine").scrollIntoViewIfNeeded();
    const m = await page.evaluate(() => {
      const b = (s: string) => document.querySelector(s)!.getBoundingClientRect();
      const card = b("[data-download-card]");
      const row = Math.max(b("[data-download-card] .who").bottom, b("[data-download-card] .options").bottom);
      return {
        // the bare ground between the card and the footer's band
        toBand: b("footer .band").top - card.bottom,
        // inside the card: the top row to the notes, the notes to Linux, Linux to the fine print
        toNotes: b("[data-first-run]").top - row,
        toLinux: b("[data-download-linux]").top - b("[data-first-run]").bottom,
        toFine: b("[data-download-card] .fine").top - b("[data-download-linux]").bottom,
      };
    });
    expect(m.toBand, `${width}px: card to footer`).toBeLessThanOrEqual(width < 640 ? 96 : 160);
    expect(m.toBand, `${width}px: card to footer`).toBeGreaterThanOrEqual(72);
    expect(m.toNotes, `${width}px: top row to notes`).toBeLessThanOrEqual(40);
    expect(m.toLinux, `${width}px: notes to Linux`).toBeLessThanOrEqual(40);
    expect(m.toFine, `${width}px: Linux to fine print`).toBeLessThanOrEqual(40);
  }
});

test("the card offers the installer (recommended) and the portable exe, each with its size", async ({ page }) => {
  await page.goto("/");
  const card = page.locator("[data-download-card]");
  await expect(card.locator("[data-download-option]")).toHaveCount(2);
  const installer = card.locator('[data-download-option="installer"]');
  await expect(installer).toContainText("Recommended");
  await expect(installer).toContainText("featherplay-setup-0.3.0.exe");
  await expect(installer.locator("[data-size]")).toHaveText("10.78 MB");
  await expect(card.locator('[data-download-option="portable"] [data-size]')).toHaveText("28.75 MB");
});

test("the card's Linux row, marked experimental, offers the x86_64 AppImage and points to the rest", async ({ page }) => {
  await page.goto("/");
  const linux = page.locator("[data-download-card] [data-download-linux]");
  await expect(linux.getByRole("heading", { name: "Linux (experimental)" })).toBeVisible();
  const appImage = release.linux!.appImage;
  await expect(linux.getByRole("link", { name: "Download the AppImage" })).toHaveAttribute("href", appImage.url);
  await expect(linux.getByRole("link", { name: "Download the AppImage" })).toHaveClass(/tonal/);
  await expect(linux).toContainText(appImage.name);
  await expect(linux.locator("[data-size]")).toHaveText(sizeLabel(appImage));
  await expect(linux.getByRole("link", { name: "release page" })).toHaveAttribute("href", release.linux!.page);
  await expect(linux.getByRole("link", { name: "README" })).toHaveAttribute(
    "href",
    "https://github.com/candygoogy/featherplay#linux-experimental",
  );
  await expect(linux).toContainText("glibc 2.35");
});
test("the footer: short link columns, the plain unofficial line, the version and what it runs on; the music credits only as a link to their own page", async ({ page }) => {
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
    ["Privacy", "/#connects"],
    ["candygoogy", "https://github.com/candygoogy"],
    ["Back to top", "#top"],
  ]) {
    await expect(footer.getByRole("link", { name, exact: true })).toHaveAttribute("href", href);
  }
  await expect(footer).toContainText("unofficial");
  await expect(footer).toContainText(`Version ${release.version}`);
  await expect(footer).toContainText("Windows 10 and 11, 64-bit");
  await expect(footer).toContainText("Linux (experimental), glibc 2.35+");
  // Every in-page link lands on something that exists.
  for (const id of ["download", "kept", "light", "lyrics", "yours", "questions", "official", "connects", "uninstall"]) {
    await expect(page.locator(`#${id}`)).toHaveCount(1);
  }
  await expect(footer.locator("[data-music-credits]")).toHaveCount(0);
  await expect(footer).not.toContainText("Josh Woodward");
  await expect(footer).not.toContainText("CC BY");
  await footer.getByRole("link", { name: "Music credits" }).click();
  await expect(page).toHaveURL(/\/credits$/);
  await expect(page.locator("[data-music-credits]")).toBeVisible();
});

test("each player's credit line links to the full credits, at the song playing", async ({ page }) => {
  await page.goto("/");
  const link = page.locator("[data-hero] [data-credit] [data-credit-all]");
  await expect(link).toHaveAttribute("href", "/credits#california-lullabye");
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

// The site says what the repo's README says (sign-in, privacy, what it connects to, the
// window's close button, the helpers). Closed answers are in the HTML, so read textContent.
test("the FAQ and features say what the README says", async ({ page }) => {
  await page.goto("/");
  const text = (sel: string) => page.locator(sel).evaluate((el) => el.textContent!.replace(/\s+/g, " "));
  const faq = await text("#questions");
  // Signing in.
  expect(faq).toContain("doesn't read your password; the code is open");
  expect(faq).toContain("Use Microsoft Edge, Use Google Chrome, Use Brave or Use Vivaldi");
  expect(faq).toContain("Sign out");
  expect(faq).toContain("ListenBrainz token");
  // 0.3.0: Linux, experimental, its sign-in and its keyring; songs that can't play.
  expect(faq).toContain("experimental Linux build");
  expect(faq).toContain("Linux has no sign-in window");
  expect(faq).toContain("on Linux in your desktop's keyring");
  expect(faq).not.toContain("There's no macOS or Linux version");
  expect(faq).toContain("Look up YouTube directly in Settings > Playback");
  // What it connects to, and what each service gets.
  await expect(page.locator("#questions summary", { hasText: "What does it connect to?" })).toBeVisible();
  expect(faq).toContain("title, artist, album and length");
  expect(faq).toContain("only while lyrics are on screen");
  expect(faq).toContain("first four characters of a hash");
  expect(faq).toContain("through the Discord app on your own PC");
  expect(faq).not.toContain("the services you turn on, nothing else");
  // The helpers.
  expect(faq).toContain("SHA256");
  expect(faq).toContain("at most once a day");
  // 0.1.2: listening signed out, Opus, the local API and Amuse.
  expect(faq).toContain("Without signing in, featherplay searches");
  expect(faq).toContain("Opus");
  expect(faq).toContain("Settings > Streaming quality");
  expect(faq).toContain("Settings > Local API");
  expect(faq).toContain("port 9863");
  // The close button asks, until you tell it to remember.
  expect(await text("#kept")).toContain("remember");
  // What it leaves out: no server of its own (SponsorBlock and LRCLIB aren't "turned on").
  const out = await text("[data-out]");
  expect(out).toContain("no server of its own");
  expect(out).not.toContain("services you turn on");
});

test("the footer's unofficial notice opens the answer to \"Is featherplay official?\"", async ({ page }) => {
  await page.goto("/");
  await page.locator("footer").getByRole("link", { name: "Unofficial notice" }).click();
  await expect(page).toHaveURL(/#official$/);
  await expect(page.locator("#official")).toHaveAttribute("open", "");
  await expect(page.locator("#official")).toBeInViewport();
});

test("the footer's name is set to the page's measure: Tri-vane first, never wider than the text above it", async ({ page }) => {
  for (const width of [390, 1440, 3840]) {
    await page.setViewportSize({ width, height: 900 });
    await page.goto("/");
    const box = await page.evaluate(() => {
      const wrap = document.querySelector(".site-footer .sign-fit")!.getBoundingClientRect();
      const mark = document.querySelector(".site-footer .sign-mark")!.getBoundingClientRect();
      const r = document.createRange();
      r.selectNodeContents(document.querySelector(".site-footer .sign-name")!);
      const name = r.getBoundingClientRect();
      return { left: wrap.left, right: wrap.right, mark: mark.left, nameRight: name.right };
    });
    expect(box.mark).toBeLessThan(box.left + 4);
    expect(box.mark).toBeGreaterThan(box.left - 12);
    expect(box.nameRight).toBeLessThan(box.right + 16);
    expect(box.nameRight).toBeGreaterThan(box.right - 0.06 * (box.right - box.left));
  }
});
