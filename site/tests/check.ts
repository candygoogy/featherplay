import { expect, type Page } from "@playwright/test";

// One layout check for every size test (bar.spec, layout.spec): what is wrong with the page as
// it rests at this size. Nothing is finished or nudged first: a glide still in flight is the
// caller's to wait for (settled), so a check can't pass by tidying up what it judges.

/** Widths either side of every rule the bar and the window change at (the bar's container
 *  queries at 420, 560, 800 and 1,180 px, the story's 761), and a coarse step between. */
export const WIDTHS = [320, 360, 390, 419, 421, 480, 559, 561, 640, 760, 761, 799, 801, 900, 1024, 1179, 1181, 1280, 1366, 1440, 1600, 1920, 2560, 3200, 3840];

/** No glide of the bar's parts in flight, and the capsule's ground grown to its size (two
 *  frames for any observer to answer first). */
export const settled = (page: Page) =>
  page.evaluate(
    () =>
      new Promise<boolean>((r) =>
        requestAnimationFrame(() =>
          requestAnimationFrame(() =>
            r(
              [...document.querySelectorAll("[data-site-bar] [data-part]")].every((p) => p.getAnimations().every((a) => a instanceof CSSTransition || a.playState === "finished")) &&
                document.querySelector("[data-site-bar] .cap-bg")!.getAnimations().every((a) => a.playState === "finished"),
            ),
          ),
        ),
      ),
  );

export async function waitSettled(page: Page) {
  await expect.poll(() => settled(page)).toBe(true);
}

/** Everything wrong at this size: the page wider than the screen; the window larger than the
 *  screen, past a side, (on a wide screen at the top) past the top or bottom, or a control of
 *  the app outside it; the bar's parts on top of each other, cut, off the bar or (as the
 *  capsule) outside it. */
export function problems(page: Page, label = ""): Promise<string[]> {
  return page.evaluate((label) => {
    const out: string[] = [];
    const vw = document.documentElement.clientWidth;
    const vh = innerHeight;
    const bar = document.querySelector<HTMLElement>("[data-site-bar]")!;
    const at = `${label || `${innerWidth}x${vh}`} ${bar.dataset.state}`;
    if (document.documentElement.scrollWidth > vw + 1) out.push(`${at}: the page is ${document.documentElement.scrollWidth} px wide`);

    const winEl = document.querySelector<HTMLElement>("[data-window]");
    if (winEl && getComputedStyle(winEl).visibility !== "hidden" && !winEl.hasAttribute("data-minimised")) {
      const win = winEl.getBoundingClientRect();
      if (win.width > vw + 0.5 || win.height > vh + 0.5) out.push(`${at}: the window is larger than the screen`);
      if (win.left < -0.5 || win.right > vw + 0.5) out.push(`${at}: the window runs off a side`);
      if (innerWidth >= 761 && scrollY === 0 && (win.top < -0.5 || win.bottom > vh + 0.5)) out.push(`${at}: the window runs off the top or bottom`);
      // The title bar's and the player bar's controls (the lists inside scroll, clipped).
      for (const c of winEl.querySelectorAll<HTMLElement>(".titlebar button, .bar button")) {
        const r = c.getBoundingClientRect();
        if (!r.width || getComputedStyle(c).visibility === "hidden") continue;
        if (r.right > win.right + 0.5 || r.left < win.left - 0.5 || r.bottom > win.bottom + 0.5) {
          out.push(`${at}: the app's ${c.getAttribute("aria-label") ?? c.textContent!.trim().slice(0, 16)} outside the window`);
          break;
        }
      }
    }

    const parts = [...bar.querySelectorAll<HTMLElement>("[data-part]")]
      .map((el) => ({ el, r: el.getBoundingClientRect(), n: el.getAttribute("aria-label") ?? el.textContent!.trim().slice(0, 12) }))
      .filter((p) => p.r.width > 0 && getComputedStyle(p.el).visibility !== "hidden");
    for (const p of parts) {
      if (p.r.left < -0.5 || p.r.right > vw + 0.5 || p.r.top < -0.5 || p.r.bottom > 72) out.push(`${at}: ${p.n} off the bar`);
      const lbl = p.el.querySelector<HTMLElement>(".lbl");
      if (lbl && lbl.clientWidth > 1 && lbl.scrollWidth > lbl.clientWidth + 0.5) out.push(`${at}: ${p.n} cut`);
    }
    for (let i = 0; i < parts.length; i++)
      for (let j = i + 1; j < parts.length; j++) {
        const a = parts[i].r;
        const b = parts[j].r;
        if (a.left < b.right - 0.5 && b.left < a.right - 0.5 && a.top < b.bottom - 0.5 && b.top < a.bottom - 0.5) out.push(`${at}: ${parts[i].n} on ${parts[j].n}`);
      }
    if (bar.dataset.state === "capsule") {
      const cap = bar.querySelector(".cap-bg")!.getBoundingClientRect();
      for (const p of parts) if (p.r.left < cap.left - 0.5 || p.r.right > cap.right + 0.5) out.push(`${at}: ${p.n} outside the capsule`);
    }
    return out;
  }, label);
}
