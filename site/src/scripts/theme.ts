// The page theme: the system's setting, dark when it has none, or the visitor's own choice,
// kept in localStorage["fp-theme"]. The inline script in Base.astro sets `data-theme` before
// the first paint with the same rules; this module keeps it right afterwards.

export type Theme = "dark" | "light";
export type ThemeChoice = Theme | "system";

import { refreshSegmented } from "./segmented";

const KEY = "fp-theme";
const LIGHT = "(prefers-color-scheme: light)";
const GROUND: Record<Theme, string> = { dark: "#0F0F0F", light: "#FFFFFF" };

// The choice this page made, for when storage is blocked (it then lasts for this page only).
let pageChoice: ThemeChoice | null = null;

function storedChoice(): ThemeChoice {
  try {
    const v = localStorage.getItem(KEY);
    return v === "dark" || v === "light" ? v : "system";
  } catch {
    return pageChoice ?? "system";
  }
}

function resolve(choice: ThemeChoice): Theme {
  if (choice !== "system") return choice;
  return matchMedia(LIGHT).matches ? "light" : "dark";
}

export function currentTheme(): Theme {
  return document.documentElement.dataset.theme === "light" ? "light" : "dark";
}

function apply(theme: Theme) {
  const root = document.documentElement;
  const changed = root.dataset.theme !== theme;
  root.dataset.theme = theme;
  document.querySelector('meta[name="theme-color"]')?.setAttribute("content", GROUND[theme]);
  if (changed) window.dispatchEvent(new CustomEvent<Theme>("themechange", { detail: theme }));
}

// The switch. The colours ease live on :root (tokens.css: every theme colour is a registered
// colour that transitions, --t-theme on --ease-theme, and the art cross-fades on --light
// with them), so nothing is snapshotted and the controls keep moving while it happens (the
// picker's pill glides, the toggle's icon turns). Reduced motion: --t-theme is 0, instant.
//
// Each frame of it restyles every element the root's colours reach, so the work is kept to
// what's on screen, and ending it touches one block at a time:
// - The page's blocks out of view sit it out (content-visibility: hidden, at their size, so
//   nothing moves) and take the new theme afterwards, nearest first, one a frame.
// - Every block is .theme-quiet until it has its final colours: its elements' own colour
//   easing (hovers, the glow) waits, so nothing trails the tokens on a clock of its own, and
//   the selection colour steps aside (base.css: Chrome would work one out for every element
//   on every frame).

const BLOCKS = "body > header, main > *, body > footer";
const hushed = new Set<HTMLElement>();
const held = new Set<HTMLElement>();
let switching = 0;

function offscreen(el: Element): boolean {
  // The header's bar is fixed to the window, wherever its place in the page has gone.
  if (el.matches("[data-site-header]")) return false;
  const r = el.getBoundingClientRect();
  return r.bottom < 0 || r.top > innerHeight;
}

/** A block's size, padding and all, to hold its place while it sits out a switch (held with
 *  no padding, so the size is the layout's own to the fraction: a content box worked out by
 *  taking the padding off would miss the layout's rounding of it, and the page under a block
 *  would move by a hair). A transformed block (none is, out of view) falls back to its style. */
function size(block: HTMLElement): string {
  const cs = getComputedStyle(block);
  const px = (...v: string[]) => v.reduce((n, x) => n + parseFloat(x), 0);
  const r = block.getBoundingClientRect();
  const plain = cs.transform === "none";
  const w = (plain ? r.width : px(cs.width)) - px(cs.borderLeftWidth, cs.borderRightWidth);
  const h = (plain ? r.height : px(cs.height)) - px(cs.borderTopWidth, cs.borderBottomWidth);
  return `${w}px ${h}px`;
}

function show(block: HTMLElement) {
  block.style.contentVisibility = "";
  block.style.containIntrinsicSize = "";
  block.style.padding = "";
  held.delete(block);
  // Its segmented controls weren't placed while it was held.
  refreshSegmented(block);
}

// The safety net for a slow device: each frame of the switch restyles the screen, and where
// that takes too long (the first frame over ~50 ms, or two in a row over ~34 ms) the eased
// colours would only stutter, so the switch ends at once (every token at its final value,
// which also completes the art's cross-fades on --light) and the visit's later switches are
// instant from the start. The controls' own motion (the picker's pill) isn't the tokens', so
// it still glides.
const INSTANT = "fp-theme-instant";
const FIRST_FRAME = 50;
const LONG_FRAME = 34;

function instant(): boolean {
  try {
    return sessionStorage.getItem(INSTANT) === "1";
  } catch {
    return false;
  }
}

function finish(running: Animation[]) {
  for (const a of running) a.finish();
}

function watch(running: Animation[], id: number) {
  let last = 0;
  let frames = 0;
  let long = 0;
  const tick = (now: number) => {
    if (id !== switching || running.every((a) => a.playState === "finished")) return;
    if (last) {
      const gap = now - last;
      frames++;
      long = gap > LONG_FRAME ? long + 1 : 0;
      if ((frames === 1 && gap > FIRST_FRAME) || long >= 2) {
        finish(running);
        try {
          sessionStorage.setItem(INSTANT, "1");
        } catch {
          // Storage blocked: the next switch watches again.
        }
        return;
      }
    }
    last = now;
    requestAnimationFrame(tick);
  };
  requestAnimationFrame(tick);
}

function switchTo(theme: Theme) {
  const root = document.documentElement;
  if (root.dataset.theme === theme) return;
  const id = ++switching;
  // All the measuring first, then all the changes (one layout).
  const blocks = [...document.querySelectorAll<HTMLElement>(BLOCKS)];
  const away = blocks.map(offscreen);
  const boxes = blocks.map((b, i) => (away[i] && !held.has(b) ? size(b) : ""));

  blocks.forEach((block, i) => {
    if (boxes[i]) {
      block.style.containIntrinsicSize = boxes[i];
      block.style.padding = "0";
      block.style.contentVisibility = "hidden";
      held.add(block);
    } else if (!away[i] && held.has(block)) show(block);
    block.classList.add("theme-quiet");
    hushed.add(block);
  });
  apply(theme);

  // The tokens' transitions start with this style change.
  getComputedStyle(root).getPropertyValue("--bg");
  const running = root.getAnimations().filter((a) => a instanceof CSSTransition);
  if (running.length) {
    if (instant()) finish(running);
    else watch(running, id);
  }
  const frame = (then: () => void) =>
    requestAnimationFrame(() => {
      if (id === switching) then();
    });
  // A frame after they end (the blocks on screen have taken the final colours, still quiet),
  // those blocks ease again; then the held blocks come
  // back nearest first, each shown in one frame (still quiet) and eased again in the next.
  Promise.allSettled(running.map((a) => a.finished)).then(() =>
    frame(() =>
      frame(() => {
        for (const block of hushed) {
          if (held.has(block)) continue;
          block.classList.remove("theme-quiet");
          hushed.delete(block);
        }
        const centre = innerHeight / 2;
        const distance = (b: Element) => Math.abs(b.getBoundingClientRect().top - centre);
        const queue = [...held].sort((a, b) => distance(a) - distance(b));
        const next = () => {
          const block = queue.shift();
          if (!block) return;
          show(block);
          frame(() => {
            block.classList.remove("theme-quiet");
            hushed.delete(block);
            frame(next);
          });
        };
        frame(next);
      }),
    ),
  );
}

export function setTheme(choice: ThemeChoice): void {
  pageChoice = choice;
  try {
    if (choice === "system") localStorage.removeItem(KEY);
    else localStorage.setItem(KEY, choice);
  } catch {
    // Storage blocked: the choice lasts for this page only.
  }
  switchTo(resolve(choice));
  // System and the theme it resolves to look the same, so a change of choice alone (Dark to
  // System on a dark OS) restyles nothing and sends no themechange: label it here.
  labelToggles();
  syncPickers();
}

/** Ticks the stored choice in any System / Light / Dark picker ([data-theme-choice]). */
function syncPickers() {
  const choice = storedChoice();
  for (const input of document.querySelectorAll<HTMLInputElement>("[data-theme-choice] input")) {
    input.checked = input.value === choice;
  }
  refreshSegmented();
}

// The header's toggle goes round the three: System (follow the OS), Light, Dark, System.
const NEXT: Record<ThemeChoice, ThemeChoice> = { system: "light", light: "dark", dark: "system" };
const NAME: Record<ThemeChoice, string> = { system: "System", light: "Light", dark: "Dark" };

/** Names each toggle for the mode it's in and what a click does next; its icon is the mode (data-choice). */
function labelToggles() {
  const choice = storedChoice();
  const label = `Theme: ${NAME[choice]}. Switch to ${NAME[NEXT[choice]]}`;
  for (const button of document.querySelectorAll<HTMLButtonElement>("[data-theme-toggle]")) {
    button.setAttribute("aria-label", label);
    button.dataset.choice = choice;
    const tip = button.querySelector("[data-tip]");
    if (tip) tip.textContent = label;
  }
}

/** Escape puts the toggle's tooltip away until the pointer or the focus leaves it (WCAG 1.4.13). */
function bindTip(button: HTMLButtonElement) {
  const back = () => delete button.dataset.tipOff;
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && (button.matches(":hover") || button.matches(":focus-visible"))) {
      button.dataset.tipOff = "";
    }
  });
  button.addEventListener("pointerleave", back);
  button.addEventListener("blur", back);
}

let started = false;

/** Follows the system while nothing is stored, and wires every `[data-theme-toggle]` and `[data-theme-choice]`. */
export function initTheme(): void {
  if (started) return;
  started = true;
  apply(resolve(storedChoice()));
  matchMedia(LIGHT).addEventListener("change", () => {
    if (storedChoice() === "system") switchTo(resolve("system"));
  });
  window.addEventListener("themechange", labelToggles);
  window.addEventListener("themechange", syncPickers);
  for (const button of document.querySelectorAll<HTMLButtonElement>("[data-theme-toggle]")) {
    button.hidden = false;
    button.addEventListener("click", () => setTheme(NEXT[storedChoice()]));
    bindTip(button);
  }
  labelToggles();
  // The pickers need script, so they stay hidden without it.
  for (const picker of document.querySelectorAll<HTMLElement>("[data-theme-choice]")) {
    picker.hidden = false;
    picker.addEventListener("change", (e) => {
      const input = e.target as HTMLInputElement;
      if (input.checked) setTheme(input.value as ThemeChoice);
    });
  }
  syncPickers();
}
