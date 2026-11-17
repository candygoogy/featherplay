// The steps after the first screen (Stage.astro) where the story doesn't run (a phone, a short
// window, reduced motion): a switch that puts the page's one window in each of the three
// shapes, whole, narrow and in the tray, with that step's words under it. The window does the
// moving (src/demo/window.ts: a change of shape morphs as maximise does, the tray is its own
// minimise, both at once with reduced motion); this only says which. The pick follows the
// window too: sent to the tray or brought back some other way (its caption button, the bar's
// tray icon, the desk's shortcut), the switch says where it is.

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
}
