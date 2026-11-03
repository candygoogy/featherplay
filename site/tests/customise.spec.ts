import { expect, test, type Page } from "@playwright/test";
import { songs } from "../src/data/songs";

// "Make it yours": a theme for the app's window (not the page) and a cover that plays and
// colours the page. Review Focus 5: text over the cover glow keeps AA in both themes.

const yours = (page: Page) => page.locator("[data-yours]");
const replica = (page: Page) => yours(page).locator("[data-replica]");

const bg = (page: Page, sel: string) =>
  page.locator(sel).first().evaluate((el) => getComputedStyle(el).backgroundColor);

async function open(page: Page) {
  await page.goto("/");
  await page.locator("#yours").scrollIntoViewIfNeeded();
}

test("picking light or dark restyles the replica only, not the page", async ({ page }) => {
  await open(page);
  const section = page.locator("#yours");
  const pageBg = await bg(page, "body");
  expect(pageBg).toBe("rgb(15, 15, 15)");

  await section.getByRole("radio", { name: "Light", exact: true }).check();
  await expect.poll(() => bg(page, "[data-yours] [data-replica]")).toBe("rgb(255, 255, 255)");
  // Every featherplay on the page is the same app: the hero's restyles too.
  await expect.poll(() => bg(page, "[data-hero] [data-replica]")).toBe("rgb(255, 255, 255)");
  expect(await bg(page, "body")).toBe(pageBg);
  await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
  // The window's own art follows its theme: the light Tri-vane in the title bar (both
  // drawings lie in one box; the other theme's is faded out).
  await expect(replica(page).locator(".titlebar .only-light")).toHaveCSS("opacity", "1");
  await expect(replica(page).locator(".titlebar .only-dark")).toHaveCSS("opacity", "0");

  await section.getByRole("radio", { name: "Dark", exact: true }).check();
  await expect.poll(() => bg(page, "[data-yours] [data-replica]")).toBe("rgb(15, 15, 15)");
  await expect(replica(page).locator(".titlebar .only-dark")).toHaveCSS("opacity", "1");
});

test("until a theme is picked the window follows the page, and can go back to it", async ({ page }) => {
  await open(page);
  const section = page.locator("#yours");
  await expect(section.getByRole("radio", { name: "Match the page" })).toBeChecked();
  await page.locator("[data-theme-toggle]").click();
  await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
  await expect.poll(() => bg(page, "[data-yours] [data-replica]")).toBe("rgb(255, 255, 255)");

  await section.getByRole("radio", { name: "Dark", exact: true }).check();
  await expect.poll(() => bg(page, "[data-yours] [data-replica]")).toBe("rgb(15, 15, 15)");
  await section.getByRole("radio", { name: "Match the page" }).check();
  await expect.poll(() => bg(page, "[data-yours] [data-replica]")).toBe("rgb(255, 255, 255)");
});

test("picking a cover plays that song and the page glow follows", async ({ page }) => {
  await open(page);
  const section = page.locator("#yours");
  const song = songs.find((s) => s.id === "origami")!;
  await section.getByRole("radio", { name: song.title }).check();
  await expect(replica(page)).toHaveAttribute("data-song", song.id);
  await expect(replica(page)).toHaveAttribute("data-playing", "true");
  await expect
    .poll(() => page.evaluate(() => document.documentElement.style.getPropertyValue("--glow")))
    .toBe(song.colour);
  const front = yours(page).locator('[data-glow-layer][data-on="true"]');
  await expect.poll(() => front.evaluate((el) => el.style.getPropertyValue("--c"))).toBe(song.colour);
  // The picker shows what's playing, whoever started it.
  await page.evaluate(() => document.querySelector<HTMLElement>('[data-hero] [data-play="watching-seabirds-soar"]')?.click());
  await expect(section.getByRole("radio", { name: "Watching Seabirds Soar (Harmony)" })).toBeChecked();
});

test("the controls work from the keyboard", async ({ page }) => {
  await open(page);
  const section = page.locator("#yours");
  await section.getByRole("radio", { name: "Match the page" }).focus();
  await page.keyboard.press("ArrowRight"); // radios move and select with the arrows
  await expect(section.getByRole("radio", { name: "Dark", exact: true })).toBeChecked();
  const first = section.getByRole("radio", { name: songs[0].title });
  await first.focus();
  await page.keyboard.press("ArrowRight");
  await expect(replica(page)).toHaveAttribute("data-song", songs[1].id);
});

// The strongest point of the glow, composited by the browser over the page's ground, against
// the colour each piece of text in the section really has.
function parseRgb(s: string): [number, number, number] {
  const n = s.match(/[\d.]+/g)!.map(Number);
  // color(srgb r g b) is 0–1; rgb() is 0–255
  return s.startsWith("color(") ? [n[0] * 255, n[1] * 255, n[2] * 255] : [n[0], n[1], n[2]];
}
function luminance([r, g, b]: [number, number, number]) {
  const f = (v: number) => {
    const c = v / 255;
    return c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4;
  };
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
}
function contrast(a: string, b: string) {
  const [x, y] = [luminance(parseRgb(a)), luminance(parseRgb(b))].sort((p, q) => q - p);
  return (x + 0.05) / (y + 0.05);
}

for (const theme of ["dark", "light"] as const) {
  test(`text over the glow keeps AA contrast for every cover (${theme})`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: theme });
    await open(page);
    const section = page.locator("#yours");
    for (const song of songs) {
      await section.getByRole("radio", { name: song.title }).check();
      const front = yours(page).locator('[data-glow-layer][data-on="true"]');
      await expect.poll(() => front.evaluate((el) => el.style.getPropertyValue("--c"))).toBe(song.colour);
      const pairs = await page.evaluate(() => {
        const field = document.querySelector<HTMLElement>("[data-yours] [data-glow]")!;
        const probe = document.createElement("span");
        probe.style.cssText =
          "position:absolute;width:1px;height:1px;background-color:color-mix(in srgb, var(--c) var(--glow-strength), var(--bg))";
        field.querySelector('[data-glow-layer][data-on="true"]')!.append(probe);
        const ground = getComputedStyle(probe).backgroundColor;
        probe.remove();
        const texts = [...document.querySelectorAll<HTMLElement>("#yours :is(h2, p, legend, label span):not([data-replica] *)")].filter(
          (el) => el.offsetParent !== null && el.textContent!.trim(),
        );
        // Text on a fill of its own (the picked segment) is read against that fill.
        const groundOf = (el: HTMLElement) => {
          for (let n: HTMLElement | null = el; n && n.id !== "yours"; n = n.parentElement) {
            const bg = getComputedStyle(n).backgroundColor;
            if (bg && !/rgba\(0, 0, 0, 0\)|transparent/.test(bg) && !/,\s*0(\.0+)?\)$/.test(bg)) return bg;
          }
          return ground;
        };
        return texts.map((el) => ({ text: el.textContent!.trim().slice(0, 30), color: getComputedStyle(el).color, ground: groundOf(el) }));
      });
      expect(pairs.length).toBeGreaterThan(3);
      for (const p of pairs) {
        expect(contrast(p.color, p.ground), `${song.title}: "${p.text}"`).toBeGreaterThanOrEqual(4.5);
      }
    }
  });
}

test("pressing Lyrics in the window keeps the window's size, and the cover glides from where it was", async ({ page }) => {
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto("/");
  const win = page.locator("[data-yours-window]");
  await win.scrollIntoViewIfNeeded();
  const before = (await win.boundingBox())!;
  const cover = page.locator("#replica-yours .np-cover");
  const start = (await cover.boundingBox())!;
  // Sample the cover's place every frame for the first few frames after the press.
  const xs = await page.evaluate(async () => {
    const pill = document.querySelector<HTMLElement>("#replica-yours .lyrics-pill")!;
    const c = document.querySelector<HTMLElement>("#replica-yours .np-cover")!;
    pill.click();
    const out: number[] = [];
    for (let i = 0; i < 4; i++) {
      await new Promise((r) => requestAnimationFrame(r));
      out.push(c.getBoundingClientRect().x);
    }
    return out;
  });
  // The first frame starts where the cover was (no jump), then it moves.
  expect(Math.abs(xs[0] - start.x)).toBeLessThan(6);
  await page.waitForTimeout(500);
  const after = (await win.boundingBox())!;
  expect(after).toEqual(before);
  const end = (await cover.boundingBox())!;
  expect(end.x - start.x).toBeGreaterThan(40);
  // Nothing inside draws outside the window.
  const outside = await win.evaluate((w) => {
    const b = w.getBoundingClientRect();
    return [...w.querySelectorAll<HTMLElement>(".np-cover, .np-text, .stage, .queue")].filter((e) => {
      const r = e.getBoundingClientRect();
      return r.width > 0 && (r.right > b.right + 1 || r.bottom > b.bottom + 1 || r.left < b.left - 1 || r.top < b.top - 1);
    }).length;
  });
  expect(outside).toBe(0);
});

// The segmented control (Make it yours' theme): one pill
// under the picked choice that glides to the next pick, by click or by the arrows.
/** The lit pill's box in its track, from its clip-path; null until segmented.ts has placed it
 *  (the stylesheet's own clip-path is in %, and a slow machine can be read before that). */
const pill = (page: Page, sel: string) =>
  page.locator(`${sel} [data-seg-lit]`).evaluate((el) => {
    const m = getComputedStyle(el).clipPath.match(/inset\(([\d.]+)px ([\d.]+)px ([\d.]+)px ([\d.]+)px/);
    if (!m) return null;
    const track = el.parentElement!.getBoundingClientRect();
    return { left: Math.round(+m[4]), right: Math.round(track.width - +m[2]) };
  });
/** Picks `value` by clicking its label in the page, and reads the lit pill's left edge on the
 *  next frame: in one round trip, so a slow machine's own delay can't let the glide finish
 *  before the read. */
const pickAndRead = (page: Page, sel: string, value: string) =>
  page.locator(sel).evaluate(async (track, value) => {
    const lit = track.querySelector<HTMLElement>("[data-seg-lit]")!;
    track.querySelector<HTMLInputElement>(`input[value="${value}"]`)!.closest("label")!.click();
    await new Promise((r) => requestAnimationFrame(r));
    const m = getComputedStyle(lit).clipPath.match(/inset\(([\d.]+)px ([\d.]+)px ([\d.]+)px ([\d.]+)px/);
    return m ? Math.round(+m[4]) : NaN;
  }, value);
const box = (page: Page, sel: string, value: string) =>
  page.locator(`${sel} input[value="${value}"]`).evaluate((input) => {
    const l = input.closest("label")!;
    return { left: l.offsetLeft, right: l.offsetLeft + l.offsetWidth };
  });

test("the segmented controls' pill glides to the pick, by click or by the arrows", async ({ page }) => {
  await open(page);
  const sel = "#yours .themes";
  await expect.poll(() => pill(page, sel)).toEqual(await box(page, sel, "page"));
  // The first placement doesn't travel; picks glide from two frames after it (segmented.ts).
  await expect(page.locator(`${sel} [data-seg][data-glide-on]`)).toBeAttached();
  // On its way: not yet there a frame later...
  const early = await pickAndRead(page, sel, "light");
  expect(early).toBeLessThan((await box(page, sel, "light")).left);
  await expect(page.locator(`${sel} input[value="light"]`)).toBeChecked();
  await page.locator(`${sel} input[value="light"]`).focus();
  // ...and on it once it settles.
  await expect.poll(() => pill(page, sel)).toEqual(await box(page, sel, "light"));
  await page.keyboard.press("ArrowLeft");
  await expect(page.locator(`${sel} input[value="dark"]`)).toBeChecked();
  await expect.poll(() => pill(page, sel)).toEqual(await box(page, sel, "dark"));
});

test.describe("reduced motion", () => {
  test.use({ reducedMotion: "reduce" });
  test("the pill moves at once", async ({ page }) => {
    await open(page);
    const sel = "#yours .themes";
    await page.locator(sel).getByRole("radio", { name: "Dark", exact: true }).click();
    expect(await pill(page, sel)).toEqual(await box(page, sel, "dark"));
  });
});
