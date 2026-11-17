// The steps after the first screen (Stage.astro) where the story doesn't run (a phone, a short
// window, reduced motion): a switch that puts the page's one window in each of the three
// shapes, whole, narrow and in the tray, with that step's words under it. The window does the
// moving (src/demo/window.ts: a change of shape morphs as maximise does, the tray is its own
// minimise, both at once with reduced motion); this only says which. The pick follows the
// window too: sent to the tray or brought back some other way (its caption button, the bar's
// tray icon, the desk's shortcut), the switch says where it is. On a desktop (reduced motion,
// a short window) the switch is drawn right under the window it changes, at its right edge,
// where the first screen has the room clear of its notes, instead of a screen further down
// (its words stay under the first screen; its place in the page, for the keys and the ear,
// stays where it is).

import type { Player } from "../demo/player";
import { whenWindow, type Shape } from "../demo/window";
import { refreshSegmented } from "./segmented";
import { STORY } from "./story";

type Pick = Shape | "tray";

export async function startShapes(player: Player): Promise<void> {
  const box = document.querySelector<HTMLElement>("[data-shape-switch]");
  const steps = document.querySelector<HTMLElement>("[data-steps]");
  if (!box || !steps) return;
  const win = await whenWindow;
  const root = document.documentElement;
  const story = matchMedia(STORY);
  const radios = [...box.querySelectorAll<HTMLInputElement>("input[type=radio]")];
  const says = [...steps.querySelectorAll<HTMLElement>("[data-step]")];
  /** The shape the visitor chose (null: the one the page gives the window). */
  let chosen: Shape | null = null;

  const show = (pick: Pick) => {
    for (const r of radios) r.checked = r.value === pick;
    for (const s of says) s.toggleAttribute("data-picked", s.dataset.step === pick);
    refreshSegmented(box);
  };
  /** In the tray (the player's state, kept by the subscription below). */
  let min = false;
  const current = (): Pick => (min ? "tray" : (chosen ?? win.natural));

  const on = () => !story.matches;
  const sync = () => {
    const live = on();
    box.hidden = !live;
    root.toggleAttribute("data-shapes", live);
    // The story lays the window out itself.
    if (!live && chosen) {
      chosen = null;
      win.shape(null);
    }
    if (live) show(current());
  };

  box.addEventListener("change", (e) => {
    const pick = (e.target as HTMLInputElement).value as Pick;
    for (const s of says) s.toggleAttribute("data-picked", s.dataset.step === pick);
    if (pick === "tray") return player.minimise();
    chosen = pick === win.natural ? null : pick;
    // Shaped first, then back from the tray into that shape.
    win.shape(chosen);
    player.restore();
  });
  // Sent away or brought back by anything else: the switch says so.
  player.subscribe((s) => {
    if (s.minimised === min) return;
    min = s.minimised;
    if (on()) show(current());
  });
  // The page's own shape can change with its size (a phone turned): the pick follows it.
  win.onLayout(() => {
    if (on() && !chosen) show(current());
  });
  story.addEventListener("change", sync);
  sync();

  // ---- Under the window, on a desktop ----
  const first = document.querySelector<HTMLElement>("[data-first]");
  const slot = document.querySelector<HTMLElement>("[data-win-slot]");
  const block = box;
  const deskMq = matchMedia("(min-width: 761px)");
  const GAP = 14;
  const lift = () => {
    if (!first || !slot) return;
    steps.removeAttribute("data-lifted");
    block.style.removeProperty("left");
    block.style.removeProperty("top");
    // Only beside the words (the wide composition): stacked, the window is right above the steps.
    if (!on() || !deskMq.matches || first.dataset.laid !== "wide") return;
    const s = slot.getBoundingClientRect();
    steps.toggleAttribute("data-lifted", true);
    const w = block.offsetWidth;
    const h = block.offsetHeight;
    const st = steps.getBoundingClientRect();
    const f = first.getBoundingClientRect();
    // At the window's right edge (its notes lean on its left, towards the words), below any
    // note that would be in the way.
    const left = s.right - w;
    let top = s.bottom + GAP;
    for (const n of first.querySelectorAll<HTMLElement>(":scope > .hand")) {
      if (getComputedStyle(n).opacity === "0" || !n.getClientRects().length) continue;
      const r = n.getBoundingClientRect();
      if (r.right > left && r.left < left + w && r.bottom > top && r.top < top + h) top = r.bottom + GAP;
    }
    if (top + h > f.bottom - GAP) {
      steps.removeAttribute("data-lifted");
      return;
    }
    block.style.left = `${Math.round(left - st.left)}px`;
    block.style.top = `${Math.round(top - st.top)}px`;
  };
  let liftFrame = 0;
  const relift = () => {
    cancelAnimationFrame(liftFrame);
    liftFrame = requestAnimationFrame(lift);
  };
  win.onLayout(relift);
  story.addEventListener("change", relift);
  addEventListener("resize", relift);
  document.fonts?.ready.then(relift);
  relift();
}
