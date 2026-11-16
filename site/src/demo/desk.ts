// The desk's colour (Desk.astro): the playing song's tones for the theme in use, faded in on the
// slot underneath by opacity (900 ms for a new song, the theme's own clock for a theme switch),
// then the slots swap roles. Nothing animates inside the masked layers. Under reduced motion the
// tones change at once.

import { deskTones, type Theme } from "../data/desk";
import type { Player } from "./player";

const SONG_MS = 900;
const THEME_MS = 320;

export function bindDesk(desk: HTMLElement, player: Player): void {
  const slots = [...desk.querySelectorAll<HTMLElement>("[data-slot]")];
  if (slots.length !== 2) return;
  const reduced = matchMedia("(prefers-reduced-motion: reduce)");
  const theme = (): Theme => (document.documentElement.dataset.theme === "light" ? "light" : "dark");
  let front = 0;
  let timer = 0;
  let colour = "";

  const paint = (slot: HTMLElement, hex: string, t: Theme) => {
    const v = deskTones(hex, t);
    // Colours as --qg, --qs, --ql, --qh; the layers' opacities as --q-lit, --q-hi, --q-shade.
    for (const [k, val] of Object.entries(v)) slot.style.setProperty(k.startsWith("q") ? `--${k}` : `--q-${k}`, String(val));
    slot.dataset.tones = t;
    slot.dataset.key = `${t}:${hex}`;
  };
  // A change while a fade runs waits for it to end, then fades on from there: cutting it short
  // repainted the slot that was showing with the new tones at once (a jump in one frame).
  let next: { hex: string; ms: number } | null = null;
  const finish = () => {
    if (!timer) return;
    clearTimeout(timer);
    timer = 0;
    const back = slots[1 - front];
    back.classList.add("front");
    back.classList.remove("fading", "in");
    slots[front].classList.remove("front");
    front = 1 - front;
    // (A frame later: the slot that was showing has gone out of sight by then, with no fade,
    // so the next fade starts from nothing.)
    if (next) {
      const n = next;
      next = null;
      requestAnimationFrame(() => requestAnimationFrame(() => set(n.hex, n.ms)));
    }
  };
  const set = (hex: string, ms: number) => {
    const t = theme();
    const key = `${t}:${hex}`;
    if (timer) {
      // Already fading to these tones: let the fade run; else the next waits for it.
      next = slots[1 - front].dataset.key === key ? null : { hex, ms };
      return;
    }
    const f = slots[front];
    if (f.dataset.key === key) return;
    if (reduced.matches || ms <= 0 || !f.dataset.key) {
      paint(f, hex, t);
      paint(slots[1 - front], hex, t);
      return;
    }
    const back = slots[1 - front];
    paint(back, hex, t);
    back.style.setProperty("--fade", `${ms}ms`);
    back.classList.add("fading");
    // Two frames to paint the new tones while still out of sight, then the fade.
    requestAnimationFrame(() => requestAnimationFrame(() => back.classList.contains("fading") && back.classList.add("in")));
    timer = window.setTimeout(finish, ms + 100);
  };

  player.subscribe((s) => {
    if (!s.song || s.song.colour === colour) return;
    const firstTime = !colour;
    colour = s.song.colour;
    set(colour, firstTime ? 0 : SONG_MS);
  });
  addEventListener("themechange", () => colour && set(colour, THEME_MS));
}
