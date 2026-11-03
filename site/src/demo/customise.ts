// "Make it yours": the theme radios set the app's own theme (every featherplay on the page,
// never the page itself); the cover radios play that song, and the page's glow follows. Both
// show the player's state, whoever changed it.

import type { Player, PlayerState } from "./player";
import { refreshSegmented } from "../scripts/segmented";

export function bindCustomise(root: HTMLElement, player: Player): void {
  const themes = [...root.querySelectorAll<HTMLInputElement>('input[name="yours-theme"]')];
  const covers = [...root.querySelectorAll<HTMLInputElement>('input[name="yours-song"]')];
  const title = root.querySelector("[data-yours-title]");
  const artist = root.querySelector("[data-yours-artist]");

  for (const t of themes) {
    t.addEventListener("change", () => {
      if (t.checked) player.setTheme(t.value as "page" | "dark" | "light");
    });
  }
  for (const c of covers) {
    c.addEventListener("change", () => {
      if (c.checked && player.state.song?.id !== c.value) player.play(c.value);
      else if (c.checked && !player.state.playing) player.resume();
    });
  }

  let song: string | null = null;
  let theme = "";
  player.subscribe((s: PlayerState) => {
    const t = s.themeSet ? s.theme : "page";
    if (t !== theme) {
      theme = t;
      for (const r of themes) r.checked = r.value === t;
      refreshSegmented(root);
    }
    const id = s.song?.id ?? null;
    if (id !== song) {
      song = id;
      for (const r of covers) r.checked = r.value === id;
      if (title) title.textContent = s.song?.title ?? "";
      if (artist) artist.textContent = s.song?.artist ?? "";
    }
  });
}
